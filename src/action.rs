//! What a binding does when its chord is pressed.

use std::fmt;
use std::str::FromStr;

/// The mode kerys starts in; its bindings are the `[bindings]` table.
pub const DEFAULT_MODE: &str = "default";

/// A binding's action, parsed from its value in the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `mode <name>`: switch binding mode. Handled by kerys itself.
    Mode(String),
    /// `run <command line>`: kerys starts the program itself, so it works
    /// without a window manager.
    Run(String),
    /// `shell <command>`: sent to a running Slowshell desktop shell, e.g.
    /// `shell toggle launcher`. Checked when the config loads.
    Shell(ShellCommand),
    /// Anything else is a window-manager command, sent as is to rustWM over
    /// its pipe (`focus left`, `workspace 3`, ...).
    Wm(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error("empty command")]
    Empty,
    #[error("`mode` needs one mode name, like `mode resize`")]
    ModeName,
    #[error("`run` needs a command line, like `run notepad.exe`")]
    RunCommand,
    #[error(
        "`shell` needs a Slowshell command: `toggle <panel>`, `open <panel>`, \
         `close <panel>`, `reload`, `overlay` or `stop`"
    )]
    ShellCommand,
}

/// A request to Slowshell, the desktop shell, as written after `shell`.
///
/// These are the `shellctl` commands that make sense on a key. They are parsed
/// when the config loads, so a typo is reported by `kerys check` rather than
/// when the key is pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellCommand {
    /// Open a hidden panel if it is closed, close it if it is open.
    Toggle(String),
    Open(String),
    Close(String),
    Reload,
    /// The developer error overlay.
    Overlay,
    Stop,
}

impl ShellCommand {
    /// The one line of JSON Slowshell's control pipe expects, without the
    /// newline. Same protocol as `shellctl`.
    pub fn request(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            ShellCommand::Toggle(name) => json!({ "cmd": "toggle", "name": name }),
            ShellCommand::Open(name) => json!({ "cmd": "open", "name": name }),
            ShellCommand::Close(name) => json!({ "cmd": "close", "name": name }),
            ShellCommand::Reload => json!({ "cmd": "reload" }),
            ShellCommand::Overlay => json!({ "cmd": "toggleOverlay" }),
            ShellCommand::Stop => json!({ "cmd": "stop" }),
        }
    }
}

impl FromStr for ShellCommand {
    type Err = ActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let words: Vec<&str> = s.split_whitespace().collect();
        Ok(match words.as_slice() {
            ["toggle", name] => ShellCommand::Toggle((*name).into()),
            ["open", name] => ShellCommand::Open((*name).into()),
            ["close", name] => ShellCommand::Close((*name).into()),
            ["reload"] => ShellCommand::Reload,
            ["overlay"] => ShellCommand::Overlay,
            ["stop"] => ShellCommand::Stop,
            _ => return Err(ActionError::ShellCommand),
        })
    }
}

impl fmt::Display for ShellCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellCommand::Toggle(n) => write!(f, "toggle {n}"),
            ShellCommand::Open(n) => write!(f, "open {n}"),
            ShellCommand::Close(n) => write!(f, "close {n}"),
            ShellCommand::Reload => f.write_str("reload"),
            ShellCommand::Overlay => f.write_str("overlay"),
            ShellCommand::Stop => f.write_str("stop"),
        }
    }
}

/// The request that tells Slowshell which binding mode kerys is in, so a bar
/// can show it (`kerys.mode` in a Slowshell config).
pub fn mode_request(mode: &str) -> serde_json::Value {
    serde_json::json!({ "cmd": "set", "key": "kerys.mode", "value": mode })
}

impl FromStr for Action {
    type Err = ActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (verb, rest) = s
            .split_once(char::is_whitespace)
            .map_or((s, ""), |(v, r)| (v, r.trim()));
        match verb {
            "" => Err(ActionError::Empty),
            "mode" if rest.is_empty() || rest.contains(char::is_whitespace) => {
                Err(ActionError::ModeName)
            }
            "mode" => Ok(Action::Mode(rest.into())),
            "run" if crate::cmdline::split_program(rest).is_none() => Err(ActionError::RunCommand),
            "run" => Ok(Action::Run(rest.into())),
            "shell" => rest.parse().map(Action::Shell),
            _ => Ok(Action::Wm(s.into())),
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Mode(m) => write!(f, "mode {m}"),
            Action::Run(c) => write!(f, "run {c}"),
            Action::Shell(c) => write!(f, "shell {c}"),
            Action::Wm(c) => f.write_str(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(s: &str) -> Action {
        s.parse().unwrap()
    }

    #[test]
    fn parses_each_kind() {
        assert_eq!(action("mode resize"), Action::Mode("resize".into()));
        assert_eq!(
            action("  run wezterm-gui.exe start --cwd ~ "),
            Action::Run("wezterm-gui.exe start --cwd ~".into())
        );
        assert_eq!(action("focus left"), Action::Wm("focus left".into()));
        assert_eq!(action("terminal"), Action::Wm("terminal".into()));
        // `exec` belongs to rustWM, which launches the program itself.
        assert_eq!(
            action("exec notepad.exe"),
            Action::Wm("exec notepad.exe".into())
        );
    }

    #[test]
    fn rejects_bad_actions() {
        assert_eq!("  ".parse::<Action>(), Err(ActionError::Empty));
        assert_eq!("mode".parse::<Action>(), Err(ActionError::ModeName));
        assert_eq!("mode a b".parse::<Action>(), Err(ActionError::ModeName));
        assert_eq!("run".parse::<Action>(), Err(ActionError::RunCommand));
        assert_eq!(
            r#"run "broken"#.parse::<Action>(),
            Err(ActionError::RunCommand)
        );
        for bad in [
            "shell",
            "shell toggle",
            "shell toggle a b",
            "shell launch x",
        ] {
            assert_eq!(
                bad.parse::<Action>(),
                Err(ActionError::ShellCommand),
                "{bad}"
            );
        }
    }

    #[test]
    fn shell_commands_become_slowshell_requests() {
        // The exact JSON is Slowshell's `shellctl` protocol, so it is pinned.
        let request = |s: &str| match action(s) {
            Action::Shell(c) => c.request().to_string(),
            other => panic!("{s} parsed as {other:?}"),
        };
        assert_eq!(
            request("shell toggle launcher"),
            r#"{"cmd":"toggle","name":"launcher"}"#
        );
        assert_eq!(
            request("shell  open   menu "),
            r#"{"cmd":"open","name":"menu"}"#
        );
        assert_eq!(
            request("shell close menu"),
            r#"{"cmd":"close","name":"menu"}"#
        );
        assert_eq!(request("shell reload"), r#"{"cmd":"reload"}"#);
        assert_eq!(request("shell overlay"), r#"{"cmd":"toggleOverlay"}"#);
        assert_eq!(request("shell stop"), r#"{"cmd":"stop"}"#);
        assert_eq!(
            mode_request("resize").to_string(),
            r#"{"cmd":"set","key":"kerys.mode","value":"resize"}"#
        );
    }

    #[test]
    fn round_trips_through_display() {
        for s in [
            "mode resize",
            "run notepad.exe",
            "move-to-workspace 3",
            "shell toggle launcher",
            "shell reload",
        ] {
            assert_eq!(action(s).to_string(), s);
        }
    }
}
