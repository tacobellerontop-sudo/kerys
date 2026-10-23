//! kerys configuration: the TOML schema, defaults, validation, and the
//! one-time move of `[bindings]` out of an old `rustwm.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::action::{Action, DEFAULT_MODE};
use crate::keys::Chord;

/// The commented config written on first run. Its `[bindings]` and `[modes]`
/// tables are the built-in key map.
pub const DEFAULT_CONFIG: &str = include_str!("default.toml");

/// rustWM's pipe, where window-manager commands go by default.
pub const DEFAULT_PIPE: &str = r"\\.\pipe\rustwm";

/// A binding value that removes a default binding.
const UNBIND: &str = "none";

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub bindings: BTreeMap<String, String>,
    /// Extra binding modes: mode name -> chord -> command.
    pub modes: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// Named pipe that window-manager commands are sent to.
    pub pipe: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            pipe: DEFAULT_PIPE.into(),
        }
    }
}

/// One mode's key map.
pub type Bindings = Vec<(Chord, Action)>;

/// A validated config, ready to hand to the keyboard hook.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub pipe: String,
    /// Bindings of the default mode.
    pub bindings: Bindings,
    /// Bindings of every other mode, by mode name.
    pub modes: Vec<(String, Bindings)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// TOML syntax or type error; the message includes line and column.
    #[error("{0}")]
    Parse(String),
    #[error("invalid config:\n  {}", .0.join("\n  "))]
    Invalid(Vec<String>),
}

fn default_config() -> Config {
    toml::from_str(DEFAULT_CONFIG).expect("default config parses")
}

/// Parses one table of bindings over `into`, reporting problems under
/// `section`.
fn parse_bindings(
    section: &str,
    table: &BTreeMap<String, String>,
    into: &mut BTreeMap<Chord, Action>,
    errors: &mut Vec<String>,
) {
    for (key, command) in table {
        let chord = match key.parse::<Chord>() {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("{section}.\"{key}\": {e}"));
                continue;
            }
        };
        if command.trim() == UNBIND {
            into.remove(&chord);
            continue;
        }
        match command.parse::<Action>() {
            Ok(a) => {
                into.insert(chord, a);
            }
            Err(e) => errors.push(format!("{section}.\"{key}\": {e}")),
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// `%USERPROFILE%\.config\kerys\kerys.toml` (or `$HOME/...` elsewhere).
pub fn default_path() -> PathBuf {
    home().join(".config").join("kerys").join("kerys.toml")
}

/// Where rustWM kept its bindings before they moved to kerys.
pub fn rustwm_config_path() -> PathBuf {
    home().join(".config").join("rustwm").join("rustwm.toml")
}

/// What [`ensure_config`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    /// The config already existed.
    Existing,
    /// Wrote [`DEFAULT_CONFIG`].
    Default,
    /// Wrote the bindings found in this rustWM config.
    Migrated(PathBuf),
}

/// Creates the config at `path` if nothing is there yet: from the
/// `[bindings]` and `[modes]` of the rustWM config at `rustwm` when it has
/// any, otherwise from [`DEFAULT_CONFIG`].
pub fn ensure_config(path: &Path, rustwm: &Path) -> std::io::Result<Created> {
    if path.exists() {
        return Ok(Created::Existing);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let migrated = std::fs::read_to_string(rustwm)
        .ok()
        .and_then(|text| migrate(&text, rustwm));
    match migrated {
        Some(text) => {
            std::fs::write(path, text)?;
            Ok(Created::Migrated(rustwm.to_path_buf()))
        }
        None => {
            std::fs::write(path, DEFAULT_CONFIG)?;
            Ok(Created::Default)
        }
    }
}

/// Builds a kerys config from the `[bindings]` and `[modes]` of an old
/// rustWM config, keeping their order. `None` if it has neither, or if the
/// result would not be a valid kerys config.
pub fn migrate(rustwm_toml: &str, from: &Path) -> Option<String> {
    let mut old: toml::Table = toml::from_str(rustwm_toml).ok()?;
    let mut moved = toml::Table::new();
    for key in ["bindings", "modes"] {
        if let Some(v) = old.remove(key) {
            moved.insert(key.into(), v);
        }
    }
    if moved.is_empty() {
        return None;
    }
    let body = toml::to_string(&moved).ok()?;
    parse(&body).ok()?;
    Some(format!(
        "# kerys configuration\n\
         #\n\
         # Key bindings moved from rustWM to kerys. These were copied from\n\
         # {}\n\
         # They are merged over the built-in defaults; set a chord to \"none\"\n\
         # to unbind it. See the kerys README for everything you can put here,\n\
         # or delete this file to get the commented default.\n\n\
         [general]\n\
         pipe = '{DEFAULT_PIPE}'\n\n\
         {body}",
        from.display()
    ))
}

/// Reads, parses and validates the config at `path`.
pub fn load(path: &Path) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse(&text)
}

/// Parses and validates config text.
pub fn parse(text: &str) -> Result<Loaded, ConfigError> {
    let config: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
    config.validate()
}

impl Config {
    /// Checks everything serde cannot and builds the key maps. All problems
    /// are reported at once.
    pub fn validate(self) -> Result<Loaded, ConfigError> {
        let mut errors = Vec::new();
        if !self.general.pipe.starts_with(r"\\.\pipe\") {
            errors.push(format!(
                r"general.pipe: `{}` is not a pipe name like \\.\pipe\rustwm",
                self.general.pipe
            ));
        }

        let defaults = default_config();
        let mut bindings = BTreeMap::new();
        parse_bindings("bindings", &defaults.bindings, &mut bindings, &mut errors);
        parse_bindings("bindings", &self.bindings, &mut bindings, &mut errors);

        // User modes replace built-in modes of the same name.
        let mut mode_tables = defaults.modes;
        mode_tables.extend(self.modes);
        let mut modes = Vec::new();
        for (name, table) in &mode_tables {
            let section = format!("modes.{name}");
            if name == DEFAULT_MODE {
                errors.push(format!("{section}: the default mode is [bindings]"));
                continue;
            }
            let mut map = BTreeMap::new();
            parse_bindings(&section, table, &mut map, &mut errors);
            let back = Action::Mode(DEFAULT_MODE.into());
            if !map.is_empty() && !map.values().any(|a| *a == back) {
                errors.push(format!(
                    "{section}: needs a binding to \"mode {DEFAULT_MODE}\""
                ));
            }
            modes.push((name.clone(), map.into_iter().collect::<Bindings>()));
        }
        // Every `mode <name>` must name a mode that exists.
        let all = bindings
            .values()
            .chain(modes.iter().flat_map(|(_, b)| b.iter().map(|(_, a)| a)));
        for action in all {
            if let Action::Mode(m) = action {
                if m != DEFAULT_MODE && !modes.iter().any(|(n, _)| n == m) {
                    errors.push(format!("`{action}`: there is no [modes.{m}]"));
                }
            }
        }

        if !errors.is_empty() {
            return Err(ConfigError::Invalid(errors));
        }
        Ok(Loaded {
            pipe: self.general.pipe,
            bindings: bindings.into_iter().collect(),
            modes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(text: &str) -> Vec<String> {
        match parse(text) {
            Err(ConfigError::Invalid(e)) => e,
            other => panic!("expected validation errors, got {other:?}"),
        }
    }

    fn find<'a>(bindings: &'a Bindings, key: &str) -> Option<&'a Action> {
        let chord: Chord = key.parse().unwrap();
        bindings.iter().find(|(c, _)| *c == chord).map(|(_, a)| a)
    }

    #[test]
    fn default_config_is_valid() {
        let loaded = parse(DEFAULT_CONFIG).unwrap();
        assert_eq!(loaded.pipe, DEFAULT_PIPE);
        assert_eq!(loaded.bindings.len(), default_config().bindings.len());
        assert_eq!(
            find(&loaded.bindings, "win+h"),
            Some(&Action::Wm("focus left".into()))
        );
        assert_eq!(
            find(&loaded.bindings, "win+alt+r"),
            Some(&Action::Mode("resize".into()))
        );
    }

    #[test]
    fn empty_config_uses_defaults() {
        let loaded = parse("").unwrap();
        assert_eq!(loaded.pipe, DEFAULT_PIPE);
        assert_eq!(loaded.bindings.len(), default_config().bindings.len());
        let (name, binds) = &loaded.modes[0];
        assert_eq!(name, "resize");
        assert_eq!(find(binds, "escape"), Some(&Action::Mode("default".into())));
    }

    #[test]
    fn user_bindings_merge_over_defaults() {
        let loaded = parse(
            r#"
            [general]
            pipe = '\\.\pipe\otherwm'

            [bindings]
            "win+t" = "run wezterm-gui.exe start --cwd ~"
            "win+h" = "none"
            "win+enter" = "exec alacritty.exe"
            "#,
        )
        .unwrap();
        assert_eq!(loaded.pipe, r"\\.\pipe\otherwm");
        let b = &loaded.bindings;
        assert_eq!(
            find(b, "win+t"),
            Some(&Action::Run("wezterm-gui.exe start --cwd ~".into()))
        );
        assert_eq!(find(b, "win+h"), None);
        assert_eq!(
            find(b, "win+enter"),
            Some(&Action::Wm("exec alacritty.exe".into()))
        );
        assert_eq!(find(b, "win+j"), Some(&Action::Wm("focus down".into())));
    }

    #[test]
    fn syntax_errors_report_line_and_column() {
        let Err(ConfigError::Parse(msg)) = parse("[bindings]\n\"win+h\" = 3\n") else {
            panic!("expected a parse error");
        };
        assert!(msg.contains("line 2"), "{msg}");

        let Err(ConfigError::Parse(msg)) = parse("[general]\nbogus = 1\n") else {
            panic!("unknown keys are errors");
        };
        assert!(msg.contains("bogus"), "{msg}");
    }

    #[test]
    fn semantic_errors_are_all_reported() {
        let errors = invalid(
            r#"
            [general]
            pipe = "rustwm"

            [bindings]
            "win+banana" = "focus left"
            "win+x" = "run"
            "win+m" = "mode nowhere"

            [modes.trap]
            "h" = "focus left"

            [modes.default]
            "x" = "close"
            "#,
        )
        .join("\n");
        for needle in [
            "general.pipe",
            "banana",
            "`run` needs",
            "no [modes.nowhere]",
            "modes.trap: needs a binding",
            "default mode is [bindings]",
        ] {
            assert!(errors.contains(needle), "missing `{needle}` in:\n{errors}");
        }
    }

    #[test]
    fn custom_modes() {
        let loaded = parse(
            r#"
            [bindings]
            "win+m" = "mode move"
            [modes.move]
            "h" = "move left"
            "escape" = "mode default"
            "#,
        )
        .unwrap();
        assert_eq!(loaded.modes.len(), 2);
    }

    #[test]
    fn migrates_bindings_from_rustwm_toml() {
        let rustwm = r#"
            [general]
            default_layout = "master"

            [bindings]
            "win+t" = "exec wezterm-gui.exe"
            "win+h" = "none"

            [modes.move]
            "h" = "move left"
            "escape" = "mode default"

            [[rules]]
            match = { exe = "Spotify.exe" }
            actions = ["float"]
            "#;
        let text = migrate(rustwm, Path::new("rustwm.toml")).unwrap();
        assert!(!text.contains("default_layout") && !text.contains("Spotify"));
        let loaded = parse(&text).unwrap();
        assert_eq!(
            find(&loaded.bindings, "win+t"),
            Some(&Action::Wm("exec wezterm-gui.exe".into()))
        );
        assert_eq!(find(&loaded.bindings, "win+h"), None);
        assert!(loaded.modes.iter().any(|(n, _)| n == "move"));

        assert_eq!(
            migrate("[general]\nworkspaces = [\"a\"]", Path::new("x")),
            None
        );
        assert_eq!(migrate("not toml [", Path::new("x")), None);
    }

    #[test]
    fn ensure_config_migrates_once() {
        let dir = std::env::temp_dir().join(format!("kerys-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rustwm = dir.join("rustwm.toml");
        let path = dir.join("kerys").join("kerys.toml");

        assert_eq!(ensure_config(&path, &rustwm).unwrap(), Created::Default);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
        assert_eq!(ensure_config(&path, &rustwm).unwrap(), Created::Existing);

        std::fs::remove_file(&path).unwrap();
        std::fs::write(&rustwm, "[bindings]\n\"win+t\" = \"terminal\"\n").unwrap();
        assert_eq!(
            ensure_config(&path, &rustwm).unwrap(),
            Created::Migrated(rustwm.clone())
        );
        assert!(load(&path).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
