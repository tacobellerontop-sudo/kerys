//! The worker thread: runs programs and sends window-manager commands, so the
//! keyboard hook never blocks on a pipe or a process launch.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::sync::mpsc::{self, Sender};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use crate::action::{Action, ShellCommand, mode_request};
use crate::cmdline::split_program;

/// `ERROR_PIPE_BUSY`: every server instance is serving another client.
const ERROR_PIPE_BUSY: i32 = 231;

/// Programs started by `run` get their own console, if they need one.
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

static SENDER: OnceLock<Sender<Action>> = OnceLock::new();

/// The window manager's pipe; changes when the config is reloaded.
static PIPE: RwLock<String> = RwLock::new(String::new());

/// Queues an action for the worker. Returns at once.
pub fn send(action: Action) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(action);
    }
}

pub fn set_pipe(pipe: &str) {
    if let Ok(mut p) = PIPE.write() {
        pipe.clone_into(&mut p);
    }
}

/// Starts the worker thread. Call once.
pub fn spawn() -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    if SENDER.set(tx).is_err() {
        return Err(io::Error::other("worker already started"));
    }
    std::thread::Builder::new()
        .name("worker".into())
        .spawn(move || {
            let mut wm = None;
            for action in rx {
                match action {
                    Action::Run(command_line) => run(&command_line),
                    Action::Wm(cmd) => command(&mut wm, &cmd),
                    Action::Shell(cmd) => shell(&cmd),
                    // The hook has already switched mode; this only tells the
                    // desktop shell, so its bar can show it.
                    Action::Mode(mode) => announce_mode(&mode),
                }
            }
        })?;
    Ok(())
}

fn run(command_line: &str) {
    let Some((program, args)) = split_program(command_line) else {
        return;
    };
    let mut cmd = std::process::Command::new(program);
    if !args.is_empty() {
        cmd.raw_arg(args);
    }
    match cmd.creation_flags(CREATE_NEW_CONSOLE).spawn() {
        Ok(_) => debug!(%command_line, "started"),
        Err(e) => warn!(%command_line, error = %e, "could not start program"),
    }
}

/// Sends a `shell ...` binding's command to Slowshell.
fn shell(cmd: &ShellCommand) {
    match super::shell::request(&cmd.request()) {
        Ok(()) => debug!(%cmd, "sent to Slowshell"),
        Err(e) => warn!(%cmd, error = %e, "shell command failed"),
    }
}

/// Tells Slowshell the binding mode changed. Quiet when no shell is running:
/// plenty of people run kerys without one.
fn announce_mode(mode: &str) {
    match super::shell::request(&mode_request(mode)) {
        Ok(()) | Err(super::shell::Error::NotRunning) => {}
        Err(e) => debug!(%mode, error = %e, "could not tell Slowshell the mode"),
    }
}

/// A connection to the window manager's pipe.
struct Connection {
    reader: BufReader<File>,
    writer: File,
}

impl Connection {
    fn open(pipe: &str) -> io::Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match OpenOptions::new().read(true).write(true).open(pipe) {
                Ok(file) => {
                    return Ok(Self {
                        writer: file.try_clone()?,
                        reader: BufReader::new(file),
                    });
                }
                Err(e)
                    if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Sends `{"cmd": ...}`. Fails if the window manager went away since the
    /// connection was opened.
    fn send(&mut self, cmd: &str) -> io::Result<()> {
        let mut line = serde_json::to_vec(&serde_json::json!({ "cmd": cmd }))?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        self.writer.flush()
    }

    /// Waits for the one-line reply to the last command.
    fn reply(&mut self) -> io::Result<serde_json::Value> {
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        serde_json::from_str(&reply).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Sends `cmd` over the kept-open connection `wm`, reconnecting once if the
/// window manager restarted since the last command. A command that was sent
/// is never sent again, even if its reply is lost.
fn command(wm: &mut Option<Connection>, cmd: &str) {
    let pipe = PIPE.read().map(|p| p.clone()).unwrap_or_default();
    for attempt in 0..2 {
        if wm.is_none() {
            match Connection::open(&pipe) {
                Ok(c) => *wm = Some(c),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    warn!(%cmd, %pipe, "the window manager is not running");
                    return;
                }
                Err(e) => {
                    warn!(%cmd, %pipe, error = %e, "cannot reach the window manager");
                    return;
                }
            }
        }
        let Some(conn) = wm.as_mut() else { return };
        if let Err(e) = conn.send(cmd) {
            *wm = None;
            if attempt == 1 {
                warn!(%cmd, error = %e, "lost the connection to the window manager");
            }
            continue;
        }
        match conn.reply() {
            Ok(reply) if reply["ok"] != true => {
                let error = reply["error"].as_str().unwrap_or("request failed");
                warn!(%cmd, %error, "the window manager refused the command");
            }
            Ok(_) => {}
            // e.g. `quit`: the window manager exits before it answers.
            Err(_) => *wm = None,
        }
        return;
    }
}
