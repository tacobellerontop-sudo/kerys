//! The data behind the settings window: an editable copy of the key map, and
//! turning it back into a config file. Kept free of Win32 so it is tested on
//! any OS.

use crate::action::{Action, DEFAULT_MODE};
use crate::config::{self, ConfigError, Loaded};
use crate::keys::Chord;

/// One mode's bindings, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    pub name: String,
    pub bindings: Vec<(Chord, String)>,
}

/// Everything the settings window edits. `modes[0]` is the default mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub pipe: String,
    pub modes: Vec<Mode>,
}

fn strings(bindings: &[(Chord, Action)]) -> Vec<(Chord, String)> {
    bindings.iter().map(|(c, a)| (*c, a.to_string())).collect()
}

impl Model {
    pub fn from_loaded(loaded: &Loaded) -> Self {
        let mut modes = vec![Mode {
            name: DEFAULT_MODE.into(),
            bindings: strings(&loaded.bindings),
        }];
        modes.extend(loaded.modes.iter().map(|(name, b)| Mode {
            name: name.clone(),
            bindings: strings(b),
        }));
        Self {
            pipe: loaded.pipe.clone(),
            modes,
        }
    }

    /// Binds `chord` to `command` in `mode`, replacing what it did before.
    /// Returns the binding's index.
    pub fn set(&mut self, mode: usize, chord: Chord, command: &str) -> Result<usize, String> {
        command.parse::<Action>().map_err(|e| e.to_string())?;
        let bindings = &mut self.modes[mode].bindings;
        let command = command.trim().to_string();
        match bindings.iter().position(|(c, _)| *c == chord) {
            Some(i) => {
                bindings[i].1 = command;
                Ok(i)
            }
            None => {
                bindings.push((chord, command));
                Ok(bindings.len() - 1)
            }
        }
    }

    pub fn remove(&mut self, mode: usize, chord: Chord) {
        self.modes[mode].bindings.retain(|(c, _)| *c != chord);
    }

    /// Adds an empty mode with an Escape binding back to the default mode.
    /// Returns its index.
    pub fn add_mode(&mut self, name: &str) -> Result<usize, String> {
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) || name.contains(['"', '.']) {
            return Err("a mode name is one word, like `resize`".into());
        }
        if self.modes.iter().any(|m| m.name == name) {
            return Err(format!("there already is a `{name}` mode"));
        }
        let escape: Chord = "escape".parse().expect("valid chord");
        self.modes.push(Mode {
            name: name.into(),
            bindings: vec![(escape, format!("mode {DEFAULT_MODE}"))],
        });
        Ok(self.modes.len() - 1)
    }

    /// Removes a mode other than the default one.
    pub fn remove_mode(&mut self, mode: usize) {
        if mode > 0 && mode < self.modes.len() {
            self.modes.remove(mode);
        }
    }

    /// The config file for this model. Bindings are merged over the built-in
    /// defaults on load, so defaults that were removed are written as
    /// `"none"` and built-in modes that were removed as empty tables.
    pub fn to_toml(&self) -> String {
        let defaults = Model::from_loaded(&config::parse("").expect("empty config is valid"));
        let quote = |s: &str| toml::Value::String(s.into()).to_string();
        let mut out = String::from(
            "# kerys configuration, saved by the kerys settings window (`kerys gui`).\n\
             #\n\
             # A binding's command is \"mode <name>\", \"run <command line>\", or a\n\
             # rustWM command such as \"focus left\". \"none\" removes a built-in\n\
             # binding; an empty [modes.<name>] removes a built-in mode. The kerys\n\
             # README has the details.\n\n",
        );
        out += &format!("[general]\npipe = {}\n", quote(&self.pipe));

        let table = |out: &mut String, header: &str, mode: &Mode, builtin: Option<&Mode>| {
            *out += &format!("\n[{header}]\n");
            for (chord, command) in &mode.bindings {
                *out += &format!("{} = {}\n", quote(&chord.to_string()), quote(command));
            }
            for (chord, _) in builtin.map_or(&[][..], |m| &m.bindings) {
                if !mode.bindings.iter().any(|(c, _)| c == chord) {
                    *out += &format!("{} = \"none\"\n", quote(&chord.to_string()));
                }
            }
        };
        let builtin = |name: &str| defaults.modes.iter().find(|m| m.name == name);
        table(&mut out, "bindings", &self.modes[0], builtin(DEFAULT_MODE));
        for mode in &self.modes[1..] {
            // A user mode replaces a built-in one wholesale, so no "none"s.
            table(&mut out, &format!("modes.{}", mode.name), mode, None);
        }
        for gone in defaults.modes[1..]
            .iter()
            .filter(|d| !self.modes.iter().any(|m| m.name == d.name))
        {
            out += &format!("\n[modes.{}]\n", gone.name);
        }
        out
    }

    /// Checks the model the way kerys will when it loads the saved file.
    pub fn validate(&self) -> Result<String, ConfigError> {
        let text = self.to_toml();
        config::parse(&text).map(|_| text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    /// Loading sorts bindings by chord and modes by name.
    fn sorted(mut m: Model) -> Model {
        m.modes[1..].sort_by(|a, b| a.name.cmp(&b.name));
        for mode in &mut m.modes {
            mode.bindings.sort();
        }
        m
    }

    fn defaults() -> Model {
        Model::from_loaded(&config::parse("").unwrap())
    }

    #[test]
    fn unchanged_defaults_round_trip() {
        let model = defaults();
        let text = model.validate().unwrap();
        assert!(!text.contains("= \"none\""));
        assert_eq!(Model::from_loaded(&config::parse(&text).unwrap()), model);
    }

    #[test]
    fn edits_survive_save_and_load() {
        let mut model = defaults();
        model.set(0, chord("win+t"), "run wezterm-gui.exe").unwrap();
        model.set(0, chord("win+h"), "focus prev").unwrap();
        model.remove(0, chord("win+l"));
        let m = model.add_mode("launch").unwrap();
        model.set(m, chord("f"), "run firefox.exe").unwrap();
        model.set(0, chord("win+o"), "mode launch").unwrap();

        let text = model.validate().unwrap();
        assert!(text.contains("\"win+l\" = \"none\""), "{text}");
        let loaded = Model::from_loaded(&config::parse(&text).unwrap());
        assert_eq!(sorted(loaded), sorted(model));
    }

    #[test]
    fn removed_builtin_mode_stays_removed() {
        let mut model = defaults();
        let resize = model.modes.iter().position(|m| m.name == "resize").unwrap();
        model.remove_mode(resize);
        // Win+Alt+R still points at it: saving must fail, not trap the keys.
        assert!(model.validate().is_err());
        model.remove(0, chord("win+alt+r"));
        let text = model.validate().unwrap();
        assert!(text.contains("[modes.resize]\n"));
        let loaded = config::parse(&text).unwrap();
        assert!(loaded.modes.is_empty());
    }

    #[test]
    fn rejects_bad_input() {
        let mut model = defaults();
        assert!(model.set(0, chord("win+x"), "run").is_err());
        assert!(model.set(0, chord("win+x"), " ").is_err());
        assert!(model.add_mode("two words").is_err());
        assert!(model.add_mode("resize").is_err());
        model.remove_mode(0);
        assert_eq!(model.modes[0].name, DEFAULT_MODE);
    }

    #[test]
    fn mode_without_way_back_fails_validation() {
        let mut model = defaults();
        let m = model.add_mode("trap").unwrap();
        model.remove(m, chord("escape"));
        model.set(m, chord("h"), "focus left").unwrap();
        let err = model.validate().unwrap_err().to_string();
        assert!(err.contains("modes.trap: needs a binding"), "{err}");
    }
}
