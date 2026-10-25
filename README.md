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
kerys gui             # settings window (also: double-click kerys.exe)
kerys                 # run in this console (Ctrl+C to quit)
kerys start           # run in the background, logging to %LOCALAPPDATA%\kerys\kerys.log
kerys stop
kerys check           # check the config for errors
kerys bindings        # list every binding, by mode
```

Only one kerys runs at a time; starting a second one does nothing.

### Settings window

`kerys gui`, double-clicking `kerys.exe`, or "Edit keybindings" in the `rwm` menu opens
a small window that lists your bindings by mode. In it you can:

- press **Record keys** and then the chord you want, which is caught before anything else
  sees it,
- pick or type a command, then press **Add / update**; **Delete** removes a binding,
- add or remove modes,
- press **Save**: the file is checked first, the previous one is kept as
  `kerys.toml.bak`, and the running daemon picks up the change right away.

The window is a separate process that ends completely when you close it. Nothing stays
behind: no tray icon, no hidden window. The daemon itself never draws anything. It waits
in the keyboard hook using no CPU while idle and a few MB of memory. Saving from the
window rewrites `kerys.toml` without your comments, so edit the file by hand if you
want to keep them.

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
"win+shift+t" = "run wezterm-gui.exe"  # kerys starts the program itself
"win+alt+r" = "mode resize"       # switch binding mode
"win+l" = "none"

[modes.resize]                    # only these keys work while the mode is on
"h" = "resize left"               # move the edge to the neighbour on the left
"l" = "resize right"
"escape" = "mode default"         # every mode needs a way back
```

A binding's command is one of:

| Command | What happens |
|---|---|
| `mode <name>` | Switch to `[modes.<name>]`; `mode default` goes back to `[bindings]`. |
| `run <command line>` | kerys starts the program. Works without rustWM. |
| anything else | Sent to rustWM, e.g. `focus left`, `workspace 3`, `exec notepad.exe`. See `rwm --help`. |

The defaults (see `src/default.toml`) cover everything rustWM does. Besides focus,
move, workspaces and monitors, they include:

| Keys | Command |
|---|---|
| Win + Alt + H/J/K/L or arrows | `stack <dir>`: put the window in its neighbour's tile |
| Win + ] / Win + [ | `cycle-stack next` / `prev`: show the next window of the stack |
| Win + Alt + U | `unstack` |
| Win + M | `toggle-maximize` (fill the screen, keep the taskbar) |
| Win + Shift + P | `toggle-pin` (float on every workspace) |
| Win + C | `center` a floating window |
| Win + T | `toggle-split` (turn a dwindle split) |
| Win + Alt + F | `flip-layout horizontal` |
| Win + O | `focus last` |
| Win + Backspace | `workspace last` |
| Win + PageUp / PageDown | `workspace prev` / `next` |
| Win + Ctrl + Alt + Left/Right | `swap-workspaces` with the monitor that way |
| Win + Shift + Space | `cycle-layout prev` |
| Win + Alt + P | `toggle-pause` |
| Win + Alt + = / Win + Alt + - | `master-count +1` / `-1` (windows in the master area) |
| Win + Alt + O | `master-side next` (move the master area round the screen) |
| Win + Alt + M | `minimize` |

In resize mode (Win + Alt + R), H/J/K/L move the focused window's edge that way.

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
