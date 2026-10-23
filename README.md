# kerys

Global hotkeys for Windows, made for the [rustWM](https://github.com/tacobellerontop-sudo/rustWM)
tiling window manager. kerys watches the keyboard, and when you press a bound chord it
sends a command to rustWM, switches binding mode, or starts a program. It plays the
same role for rustWM that whkd plays for komorebi.

rustWM itself has no hotkeys: `rustwm.exe` starts `kerys.exe` (found next to it or on
`PATH`) and stops it again when it quits. You can also run kerys on its own, for
example to drive another window manager with a compatible pipe, or just to launch
programs.

## How it fits together

```text
  keyboard ──► kerys.exe ──{"cmd":"focus left"}──► \\.\pipe\rustwm ──► rustwm.exe
   (WH_KEYBOARD_LL hook,                          (the same pipe and commands
    modes, run <program>)                          `rwm` uses)
```

- **kerys** owns everything about keys: the low-level keyboard hook, chord parsing,
  binding modes (like Hyprland submaps), and Start-menu suppression after a Win chord.
- **rustWM** owns windows. It only sees command strings, the same ones `rwm <command>`
  sends, so anything kerys can do you can also script with `rwm`.

## Using it

The rustWM release zip already contains `kerys.exe`; keep it next to `rustwm.exe` and
`rwm start` brings both up. To run kerys by itself:

```powershell
kerys                 # run in this console (Ctrl+C to quit)
kerys start           # run in the background, logging to %LOCALAPPDATA%\kerys\kerys.log
kerys stop
kerys check           # check the config for errors
kerys bindings        # list every binding, by mode
```

Only one kerys runs at a time; starting a second one does nothing.

## Config

On first run kerys writes `%USERPROFILE%\.config\kerys\kerys.toml`. If you had
`[bindings]` or `[modes]` in an older `rustwm.toml`, they are copied over instead, so
your keys keep working. Saved changes apply right away; a broken file is logged and the
previous bindings stay active.

```toml
[general]
pipe = '\\.\pipe\rustwm'          # where window-manager commands go

[bindings]                        # merged over the defaults; "none" unbinds
"win+h" = "focus left"            # sent to rustWM
"win+t" = "run wezterm-gui.exe"   # kerys starts the program itself
"win+alt+r" = "mode resize"       # switch binding mode
"win+l" = "none"

[modes.resize]                    # only these keys work while the mode is on
"h" = "resize shrink"
"l" = "resize grow"
"escape" = "mode default"         # every mode needs a way back
```

A binding's command is one of:

| Command | What happens |
|---|---|
| `mode <name>` | Switch to `[modes.<name>]`; `mode default` goes back to `[bindings]`. |
| `run <command line>` | kerys starts the program. Works without rustWM. |
| anything else | Sent to rustWM, e.g. `focus left`, `workspace 3`, `exec notepad.exe`. See `rwm --help`. |

Keys: letters, digits, `f1`–`f24`, `enter`, `space`, `tab`, `escape`, `backspace`,
`delete`, `insert`, `home`, `end`, `pageup`, `pagedown`, arrows (`left` …),
`grave`, `minus`, `equal`, `comma`, `period`, `slash`, `semicolon`, `quote`,
`lbracket`, `rbracket`, `backslash`. Modifiers: `win`, `ctrl`, `alt`, `shift`.

**Win + L** always locks the PC unless `DisableLockWorkstation` is set
(`HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\System`).

## Building

```sh
cargo build --release
cargo test
```

The hook only runs on Windows; config parsing and its tests work on any OS. Logging is
controlled with `RUST_LOG`.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
