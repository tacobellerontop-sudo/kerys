//! The `kerys` command: run the daemon, or start, stop and check it.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use kerys::config;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "kerys",
    version,
    about = "Global hotkeys for Windows that drive rustWM",
    after_help = "With no command, kerys runs in this console until Ctrl+C or `kerys stop`. \
                  Double-clicking kerys.exe opens the settings window instead."
)]
struct Cli {
    /// Config file to use instead of %USERPROFILE%\.config\kerys\kerys.toml.
    #[arg(long, short, global = true)]
    config: Option<PathBuf>,
    /// Write logs to this file. Without a console (as when rustWM starts
    /// kerys) logs go to %LOCALAPPDATA%\kerys\kerys.log.
    #[arg(long)]
    log: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Start kerys in the background, logging to %LOCALAPPDATA%\kerys\kerys.log.
    Start,
    /// Stop the running kerys.
    Stop,
    /// Check the config for errors.
    Check,
    /// Print every binding, by mode.
    Bindings,
    /// Open the settings window to view and edit bindings.
    Gui,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = cli.config.clone().unwrap_or_else(config::default_path);
    let result = match cli.command {
        Some(Cmd::Gui) => gui(&config),
        None if cli.log.is_none() && double_clicked() => gui(&config),
        None => {
            let log = cli.log.or_else(|| {
                let console = std::io::stderr().is_terminal();
                (!console).then(default_log_path)
            });
            init_logging(log.as_deref()).and_then(|()| run(&config))
        }
        Some(Cmd::Start) => start(&config),
        Some(Cmd::Stop) => stop(),
        Some(Cmd::Check) => config::load(&config)
            .map(|_| println!("{}: OK", config.display()))
            .map_err(Into::into),
        Some(Cmd::Bindings) => {
            print_bindings(&config);
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kerys: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn default_log_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("kerys")
        .join("kerys.log")
}

fn init_logging(log: Option<&Path>) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match log {
        Some(path) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let file = std::fs::File::create(path)?;
            builder
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .init();
        }
        None => builder.init(),
    }
    Ok(())
}

/// Loads the config, falling back to the built-in defaults if it is broken so
/// the keys always work.
fn load_or_default(path: &Path) -> (config::Loaded, Option<config::ConfigError>) {
    match config::load(path) {
        Ok(loaded) => (loaded, None),
        Err(e) => (config::parse("").expect("empty config is valid"), Some(e)),
    }
}

fn print_bindings(path: &Path) {
    let (loaded, error) = load_or_default(path);
    if let Some(e) = error {
        println!("{e}; showing the built-in defaults");
        println!();
    }
    let width = loaded
        .bindings
        .iter()
        .chain(loaded.modes.iter().flat_map(|(_, b)| b))
        .map(|(c, _)| c.to_string().len())
        .max()
        .unwrap_or(0);
    for (chord, action) in &loaded.bindings {
        println!("  {:width$}  {action}", chord.to_string());
    }
    for (mode, bindings) in &loaded.modes {
        println!();
        println!("  In {mode} mode:");
        for (chord, action) in bindings {
            println!("  {:width$}  {action}", chord.to_string());
        }
    }
}

#[cfg(windows)]
fn run(path: &Path) -> anyhow::Result<()> {
    use kerys::{watch, win};
    use tracing::{error, info, warn};

    let Some(_instance) = win::daemon::Instance::claim()? else {
        info!("kerys is already running");
        return Ok(());
    };
    info!(version = env!("CARGO_PKG_VERSION"), "kerys starting");

    match config::ensure_config(path, &config::rustwm_config_path()) {
        Ok(config::Created::Existing) => {}
        Ok(config::Created::Default) => info!(path = %path.display(), "wrote default config"),
        Ok(config::Created::Migrated(from)) => info!(
            path = %path.display(),
            from = %from.display(),
            "copied key bindings from the rustWM config"
        ),
        Err(e) => warn!(path = %path.display(), error = %e, "could not write default config"),
    }
    let (loaded, config_error) = load_or_default(path);
    match config_error {
        None => info!(path = %path.display(), "config loaded"),
        Some(e) => error!(path = %path.display(), "{e}; using built-in defaults"),
    }
    let apply = |loaded: &config::Loaded| {
        win::worker::set_pipe(&loaded.pipe);
        win::hook::set_bindings(loaded);
    };
    apply(&loaded);
    win::worker::spawn()?;

    let reload_path = path.to_path_buf();
    let _watcher = watch::watch_config(path, move || match config::load(&reload_path) {
        Ok(loaded) => {
            apply(&loaded);
            info!("config reloaded");
        }
        Err(e) => error!("{e}; keeping the previous config"),
    })
    .inspect_err(|e| warn!(error = %e, "config hot reload disabled"))
    .ok();

    info!("kerys running; `kerys stop` or Ctrl+C to quit");
    win::daemon::run()?;
    info!("kerys stopped");
    Ok(())
}

/// Started from Explorer rather than a terminal: Windows gave us a console
/// of our own, which no other process shares.
#[cfg(windows)]
fn double_clicked() -> bool {
    use windows::Win32::System::Console::GetConsoleProcessList;
    let mut ids = [0u32; 2];
    // SAFETY: a valid buffer; the call fills at most its length.
    unsafe { GetConsoleProcessList(&mut ids) == 1 }
}

#[cfg(windows)]
fn gui(path: &Path) -> anyhow::Result<()> {
    use windows::Win32::System::Console::FreeConsole;
    // Drop the console Explorer opened for us; only the window remains.
    // SAFETY: no preconditions.
    let _ = unsafe { FreeConsole() };
    kerys::win::gui::run(path)
}

#[cfg(windows)]
fn start(path: &Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};

    use anyhow::Context;

    /// No console window for the background daemon.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let log = default_log_path();
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("--config")
        .arg(path)
        .arg("--log")
        .arg(&log)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("failed to launch kerys")?;
    // Give it a moment: an instant exit means another kerys is running or
    // the hook failed.
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "kerys exited ({status}); is it already running? See {}",
                log.display()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("kerys started (log: {})", log.display());
    Ok(())
}

#[cfg(windows)]
fn stop() -> anyhow::Result<()> {
    if !kerys::win::daemon::stop() {
        anyhow::bail!("kerys is not running");
    }
    println!("kerys stopped");
    Ok(())
}

#[cfg(not(windows))]
fn run(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("kerys only runs on Windows")
}

#[cfg(not(windows))]
fn double_clicked() -> bool {
    false
}

#[cfg(not(windows))]
fn gui(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("the settings window only works on Windows")
}

#[cfg(not(windows))]
fn start(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("kerys only runs on Windows")
}

#[cfg(not(windows))]
fn stop() -> anyhow::Result<()> {
    anyhow::bail!("kerys only runs on Windows")
}
