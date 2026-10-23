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
            _ => Ok(Action::Wm(s.into())),
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Mode(m) => write!(f, "mode {m}"),
            Action::Run(c) => write!(f, "run {c}"),
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
    }

    #[test]
    fn round_trips_through_display() {
        for s in ["mode resize", "run notepad.exe", "move-to-workspace 3"] {
            assert_eq!(action(s).to_string(), s);
        }
    }
}
