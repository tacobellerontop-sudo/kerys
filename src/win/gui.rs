//! The settings window (`kerys gui`): view and edit the key map, then save it
//! to kerys.toml, which the running daemon reloads on its own.
//!
//! It is a separate, short-lived process. Closing the window ends it, so it
//! costs nothing while it is not open; the daemon never draws anything.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, CreateFontIndirectW, DeleteObject, GetSysColorBrush, HFONT, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
    LIST_VIEW_ITEM_STATE_FLAGS, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVIS_FOCUSED,
    LVIS_SELECTED, LVITEMW, LVM_DELETEALLITEMS, LVM_ENSUREVISIBLE, LVM_GETNEXTITEM,
    LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVN_ITEMCHANGED, LVNI_SELECTED, LVS_EX_DOUBLEBUFFER,
    LVS_EX_FULLROWSELECT, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL, NMHDR, NMLISTVIEW,
    WC_LISTVIEWW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
    SystemParametersInfoForDpi,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CBN_SELCHANGE,
    CBS_AUTOHSCROLL, CBS_DROPDOWN, CBS_DROPDOWNLIST, CallNextHookEx, CreateWindowExW,
    DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL, ES_READONLY, GetClientRect,
    GetMessageW, GetWindowTextLengthW, GetWindowTextW, HC_ACTION, HHOOK, HMENU, IDCANCEL, IDYES,
    IsDialogMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MB_ICONERROR, MB_ICONWARNING, MB_OK,
    MB_YESNOCANCEL, MSG, MessageBoxW, MoveWindow, NONCLIENTMETRICSW, PostMessageW, PostQuitMessage,
    RegisterClassW, SPI_GETNONCLIENTMETRICS, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW,
    SetWindowPos, SetWindowTextW, SetWindowsHookExW, ShowWindow, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE,
    WM_COMMAND, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_KEYUP, WM_NOTIFY, WM_SETFONT, WM_SIZE,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSW, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_EX_CONTROLPARENT, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use super::daemon;
use super::hook::{is_modifier, pressed, suppress_start_menu};
use crate::action::DEFAULT_MODE;
use crate::config;
use crate::editor::Model;
use crate::keys::Chord;

// Control ids.
const ID_MODE: u16 = 100;
const ID_MODE_NAME: u16 = 101;
const ID_ADD_MODE: u16 = 102;
const ID_DEL_MODE: u16 = 103;
const ID_LIST: u16 = 104;
const ID_KEYS: u16 = 105;
const ID_RECORD: u16 = 106;
const ID_CMD: u16 = 107;
const ID_SET: u16 = 108;
const ID_DELETE: u16 = 109;
const ID_SAVE: u16 = 110;

/// The recording hook caught a chord; `wparam` is [`pack`]ed.
const WM_RECORDED: u32 = WM_APP + 1;
/// Recording is over (Win released, or Escape pressed).
const WM_RECORD_DONE: u32 = WM_APP + 2;

/// Commands offered in the Command box. Any other text works too.
const SUGGESTIONS: &[&str] = &[
    "focus left",
    "focus down",
    "focus up",
    "focus right",
    "focus next",
    "focus prev",
    "move left",
    "move down",
    "move up",
    "move right",
    "workspace 1",
    "move-to-workspace 1",
    "focus-monitor left",
    "focus-monitor right",
    "move-to-monitor left",
    "move-to-monitor right",
    "terminal",
    "close",
    "promote",
    "cycle-layout",
    "layout dwindle",
    "layout master",
    "layout monocle",
    "toggle-float",
    "toggle-fullscreen",
    "resize grow",
    "resize shrink",
    "toggle-special scratch",
    "workspace-mode toggle",
    "retile",
    "reload",
    "quit",
    "exec notepad.exe",
    "run notepad.exe",
    "mode default",
];

/// Window handles of the controls.
struct Controls {
    mode_label: HWND,
    mode: HWND,
    mode_name: HWND,
    add_mode: HWND,
    del_mode: HWND,
    list: HWND,
    keys_label: HWND,
    keys: HWND,
    record: HWND,
    cmd_label: HWND,
    cmd: HWND,
    set: HWND,
    delete: HWND,
    status: HWND,
    save: HWND,
}

struct Gui {
    hwnd: HWND,
    path: PathBuf,
    model: Model,
    /// Index into `model.modes` shown in the list.
    mode: usize,
    /// The chord in the Keys box.
    chord: Option<Chord>,
    dirty: bool,
    c: Controls,
    font: HFONT,
    record_hook: Option<HHOOK>,
}

thread_local! {
    static GUI: RefCell<Option<Gui>> = const { RefCell::new(None) };
}

/// Runs `f` on the window state. Returns `None` while the state is already
/// borrowed: controls notify us synchronously about our own changes to them,
/// and those notifications can be ignored.
fn with_gui<R>(f: impl FnOnce(&mut Gui) -> R) -> Option<R> {
    GUI.with(|g| g.try_borrow_mut().ok()?.as_mut().map(f))
}

fn text(hwnd: HWND) -> String {
    // SAFETY: plain window-text calls with a buffer of the reported size.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

fn set_text(hwnd: HWND, s: &str) {
    // SAFETY: a valid window and string.
    let _ = unsafe { SetWindowTextW(hwnd, &HSTRING::from(s)) };
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: messages sent to our own controls with valid arguments.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

fn message_box(
    hwnd: HWND,
    body: &str,
    style: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE,
) -> i32 {
    // SAFETY: valid owner and strings.
    unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(body), w!("kerys"), style).0 }
}

fn child(
    parent: HWND,
    class: PCWSTR,
    title: &str,
    style: u32,
    ex_style: WINDOW_EX_STYLE,
    id: u16,
) -> HWND {
    // SAFETY: creating a child control of our window; the id is passed as
    // the menu handle, as Win32 expects for child windows.
    unsafe {
        CreateWindowExW(
            ex_style,
            class,
            &HSTRING::from(title),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            GetModuleHandleW(None).ok().map(Into::into),
            None,
        )
        .unwrap_or_default()
    }
}

fn button(parent: HWND, title: &str, id: u16) -> HWND {
    child(
        parent,
        w!("BUTTON"),
        title,
        WS_TABSTOP.0 | BS_PUSHBUTTON as u32,
        WINDOW_EX_STYLE::default(),
        id,
    )
}

fn label(parent: HWND, title: &str) -> HWND {
    child(
        parent,
        w!("STATIC"),
        title,
        0,
        WINDOW_EX_STYLE::default(),
        0,
    )
}

fn create_controls(hwnd: HWND) -> Controls {
    let edit_style = WS_TABSTOP.0 | ES_AUTOHSCROLL as u32;
    let c = Controls {
        mode_label: label(hwnd, "Mode:"),
        mode: child(
            hwnd,
            w!("COMBOBOX"),
            "",
            WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST as u32,
            WINDOW_EX_STYLE::default(),
            ID_MODE,
        ),
        mode_name: child(
            hwnd,
            w!("EDIT"),
            "",
            edit_style,
            WS_EX_CLIENTEDGE,
            ID_MODE_NAME,
        ),
        add_mode: button(hwnd, "Add mode", ID_ADD_MODE),
        del_mode: button(hwnd, "Remove mode", ID_DEL_MODE),
        list: child(
            hwnd,
            WC_LISTVIEWW,
            "",
            WS_TABSTOP.0 | WS_BORDER.0 | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
            WINDOW_EX_STYLE::default(),
            ID_LIST,
        ),
        keys_label: label(hwnd, "Keys:"),
        keys: child(
            hwnd,
            w!("EDIT"),
            "",
            WS_TABSTOP.0 | (ES_AUTOHSCROLL | ES_READONLY) as u32,
            WS_EX_CLIENTEDGE,
            ID_KEYS,
        ),
        record: button(hwnd, "Record keys", ID_RECORD),
        cmd_label: label(hwnd, "Command:"),
        cmd: child(
            hwnd,
            w!("COMBOBOX"),
            "",
            WS_TABSTOP.0 | WS_VSCROLL.0 | (CBS_DROPDOWN | CBS_AUTOHSCROLL) as u32,
            WINDOW_EX_STYLE::default(),
            ID_CMD,
        ),
        set: button(hwnd, "Add / update", ID_SET),
        delete: button(hwnd, "Delete", ID_DELETE),
        status: label(hwnd, ""),
        save: button(hwnd, "Save", ID_SAVE),
    };
    send(
        c.list,
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        0,
        (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
    );
    for (i, title) in ["Keys", "Command"].iter().enumerate() {
        let title = HSTRING::from(*title);
        let col = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            cx: 100,
            pszText: PWSTR(title.as_ptr() as *mut _),
            ..Default::default()
        };
        send(c.list, LVM_INSERTCOLUMNW, i, &raw const col as isize);
    }
    /// `EM_SETCUEBANNER`: grey hint text shown while an edit box is empty.
    const EM_SETCUEBANNER: u32 = 0x1501;
    let hint = HSTRING::from("new mode name");
    send(c.mode_name, EM_SETCUEBANNER, 0, hint.as_ptr() as isize);
    for s in SUGGESTIONS {
        let s = HSTRING::from(*s);
        send(c.cmd, CB_ADDSTRING, 0, s.as_ptr() as isize);
    }
    c
}

impl Controls {
    fn all(&self) -> [HWND; 15] {
        [
            self.mode_label,
            self.mode,
            self.mode_name,
            self.add_mode,
            self.del_mode,
            self.list,
            self.keys_label,
            self.keys,
            self.record,
            self.cmd_label,
            self.cmd,
            self.set,
            self.delete,
            self.status,
            self.save,
        ]
    }
}

/// The system's dialog font at `dpi`.
fn create_font(dpi: u32) -> HFONT {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: `metrics` is the struct this action fills, with cbSize set.
    unsafe {
        let _ = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some((&raw mut metrics).cast()),
            0,
            dpi,
        );
        CreateFontIndirectW(&metrics.lfMessageFont)
    }
}

impl Gui {
    fn dpi(&self) -> u32 {
        // SAFETY: our own window.
        unsafe { GetDpiForWindow(self.hwnd) }.max(96)
    }

    fn set_font(&mut self) {
        let font = create_font(self.dpi());
        for hwnd in self.c.all() {
            send(hwnd, WM_SETFONT, font.0 as usize, 1);
        }
        // SAFETY: the old font is no longer selected into any control.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.font.0)) };
        self.font = font;
    }

    fn layout(&self) {
        let s = |v: i32| v * self.dpi() as i32 / 96;
        let mut rc = RECT::default();
        // SAFETY: our own window.
        let _ = unsafe { GetClientRect(self.hwnd, &mut rc) };
        let (w, h) = (rc.right, rc.bottom);
        let m = s(12);
        let row = s(26);
        let place = |hwnd: HWND, x: i32, y: i32, cw: i32, ch: i32| {
            // SAFETY: moving our own child controls.
            let _ = unsafe { MoveWindow(hwnd, x, y, cw.max(0), ch.max(0), true) };
        };
        // A label sits a little lower than the control beside it.
        let lbl = s(4);

        let c = &self.c;
        place(c.mode_label, m, m + lbl, s(48), row);
        place(c.mode, m + s(52), m, s(150), s(300));
        place(c.mode_name, m + s(212), m, s(120), row);
        place(c.add_mode, m + s(340), m, s(96), row);
        place(c.del_mode, m + s(444), m, s(110), row);

        let bottom = h - m - row;
        let set_row = bottom - s(14) - row;
        let cmd_row = set_row - s(8) - row;
        let keys_row = cmd_row - s(8) - row;
        let list_top = m + row + s(10);
        place(c.list, m, list_top, w - 2 * m, keys_row - s(12) - list_top);
        let list_w = w - 2 * m - s(24);
        send(c.list, LVM_SETCOLUMNWIDTH, 0, (list_w * 2 / 5) as isize);
        send(c.list, LVM_SETCOLUMNWIDTH, 1, (list_w * 3 / 5) as isize);

        let field_x = m + s(80);
        place(c.keys_label, m, keys_row + lbl, s(76), row);
        place(c.keys, field_x, keys_row, s(220), row);
        place(c.record, field_x + s(228), keys_row, s(110), row);
        place(c.cmd_label, m, cmd_row + lbl, s(76), row);
        place(c.cmd, field_x, cmd_row, w - field_x - m, s(300));
        place(c.set, field_x, set_row, s(110), row);
        place(c.delete, field_x + s(118), set_row, s(90), row);
        place(c.status, m, bottom + lbl, w - 3 * m - s(96), row);
        place(c.save, w - m - s(96), bottom, s(96), row);
    }

    fn title(&self) {
        let star = if self.dirty { "*" } else { "" };
        set_text(self.hwnd, &format!("{star}kerys keybindings"));
    }

    fn status(&self, s: &str) {
        set_text(self.c.status, s);
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.title();
        self.status("Not saved yet.");
    }

    fn fill_modes(&self) {
        send(self.c.mode, CB_RESETCONTENT, 0, 0);
        for m in &self.model.modes {
            let name = if m.name == DEFAULT_MODE {
                HSTRING::from("default (always on)")
            } else {
                HSTRING::from(m.name.as_str())
            };
            send(self.c.mode, CB_ADDSTRING, 0, name.as_ptr() as isize);
        }
        send(self.c.mode, CB_SETCURSEL, self.mode, 0);
        // SAFETY: our own control.
        let _ = unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                self.c.del_mode,
                self.mode > 0,
            )
        };
    }

    /// Refills the list for the current mode and selects row `select`.
    fn fill_list(&self, select: Option<usize>) {
        send(self.c.list, LVM_DELETEALLITEMS, 0, 0);
        for (i, (chord, command)) in self.model.modes[self.mode].bindings.iter().enumerate() {
            let keys = HSTRING::from(chord.to_string());
            let item = LVITEMW {
                mask: LVIF_TEXT,
                iItem: i as i32,
                pszText: PWSTR(keys.as_ptr() as *mut _),
                ..Default::default()
            };
            send(self.c.list, LVM_INSERTITEMW, 0, &raw const item as isize);
            let command = HSTRING::from(command.as_str());
            let sub = LVITEMW {
                iSubItem: 1,
                pszText: PWSTR(command.as_ptr() as *mut _),
                ..Default::default()
            };
            send(self.c.list, LVM_SETITEMTEXTW, i, &raw const sub as isize);
        }
        if let Some(i) = select {
            let state = LVITEMW {
                stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
                state: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
                ..Default::default()
            };
            send(self.c.list, LVM_SETITEMSTATE, i, &raw const state as isize);
            send(self.c.list, LVM_ENSUREVISIBLE, i, 0);
        }
    }

    fn selected(&self) -> Option<usize> {
        let i = send(
            self.c.list,
            LVM_GETNEXTITEM,
            usize::MAX,
            LVNI_SELECTED as isize,
        );
        (i >= 0).then_some(i as usize)
    }

    fn show_binding(&mut self, i: usize) {
        if let Some((chord, command)) = self.model.modes[self.mode].bindings.get(i).cloned() {
            self.chord = Some(chord);
            set_text(self.c.keys, &chord.to_string());
            set_text(self.c.cmd, &command);
        }
    }

    fn select_mode(&mut self, mode: usize) {
        self.mode = mode.min(self.model.modes.len() - 1);
        self.fill_modes();
        self.fill_list(None);
        self.chord = None;
        set_text(self.c.keys, "");
        set_text(self.c.cmd, "");
    }

    fn set_binding(&mut self) {
        let Some(chord) = self.chord else {
            message_box(
                self.hwnd,
                "Pick a binding in the list, or press Record keys first.",
                MB_OK | MB_ICONWARNING,
            );
            return;
        };
        match self.model.set(self.mode, chord, &text(self.c.cmd)) {
            Ok(i) => {
                self.fill_list(Some(i));
                self.changed();
            }
            Err(e) => {
                message_box(
                    self.hwnd,
                    &format!("That command doesn't work: {e}."),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }

    fn delete_binding(&mut self) {
        let Some(i) = self.selected() else { return };
        let (chord, _) = self.model.modes[self.mode].bindings[i];
        self.model.remove(self.mode, chord);
        let len = self.model.modes[self.mode].bindings.len();
        let next = (len > 0).then(|| i.min(len - 1));
        self.fill_list(next);
        match next {
            Some(n) => self.show_binding(n),
            None => {
                self.chord = None;
                set_text(self.c.keys, "");
                set_text(self.c.cmd, "");
            }
        }
        self.changed();
    }

    fn add_mode(&mut self) {
        match self.model.add_mode(&text(self.c.mode_name)) {
            Ok(i) => {
                set_text(self.c.mode_name, "");
                self.select_mode(i);
                self.changed();
                self.status(
                    "Mode added. Bind a key to \"mode <name>\" in the default mode to enter it.",
                );
            }
            Err(e) => {
                message_box(
                    self.hwnd,
                    &format!("Can't add that mode: {e}."),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }

    fn remove_mode(&mut self) {
        if self.mode == 0 {
            return;
        }
        self.model.remove_mode(self.mode);
        self.select_mode(0);
        self.changed();
    }

    /// Validates and writes the config. Returns false if nothing was saved.
    fn save(&mut self) -> bool {
        let text = match self.model.validate() {
            Ok(t) => t,
            Err(e) => {
                message_box(
                    self.hwnd,
                    &format!("Not saved, the bindings have a problem:\n\n{e}"),
                    MB_OK | MB_ICONERROR,
                );
                return false;
            }
        };
        // Keep the previous file once, in case the user had comments in it.
        if self.path.exists() {
            let _ = std::fs::copy(&self.path, self.path.with_extension("toml.bak"));
        }
        if let Err(e) = std::fs::write(&self.path, text) {
            message_box(
                self.hwnd,
                &format!("Could not write {}: {e}", self.path.display()),
                MB_OK | MB_ICONERROR,
            );
            return false;
        }
        self.dirty = false;
        self.title();
        let applied = if daemon::is_running() {
            "kerys is using it now."
        } else {
            "kerys will use it when it starts."
        };
        self.status(&format!("Saved. {applied}"));
        true
    }

    fn start_recording(&mut self) {
        if self.record_hook.is_some() {
            return;
        }
        RECORD_TARGET.store(self.hwnd.0 as isize, Ordering::Relaxed);
        RECORD_WAIT_WIN.store(false, Ordering::Relaxed);
        // SAFETY: a low-level hook on this thread, which pumps messages.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(record_proc),
                GetModuleHandleW(None).ok().map(Into::into),
                0,
            )
        };
        match hook {
            Ok(h) => {
                self.record_hook = Some(h);
                set_text(self.c.keys, "Press the keys... (Esc cancels)");
                set_text(self.c.record, "Recording...");
            }
            Err(e) => {
                message_box(
                    self.hwnd,
                    &format!("Could not watch the keyboard: {e}"),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
    }

    fn stop_recording(&mut self) {
        if let Some(h) = self.record_hook.take() {
            // SAFETY: the hook we installed on this thread.
            let _ = unsafe { UnhookWindowsHookEx(h) };
        }
        RECORD_TARGET.store(0, Ordering::Relaxed);
        set_text(self.c.record, "Record keys");
        set_text(
            self.c.keys,
            &self.chord.map(|c| c.to_string()).unwrap_or_default(),
        );
    }

    fn recorded(&mut self, chord: Chord) {
        self.chord = Some(chord);
        set_text(self.c.keys, &chord.to_string());
        // Show what the chord already does in this mode, if anything.
        let bindings = &self.model.modes[self.mode].bindings;
        match bindings.iter().position(|(c, _)| *c == chord) {
            Some(i) => {
                self.fill_list(Some(i));
                set_text(self.c.cmd, &bindings[i].1);
                self.status("Already bound: change the command and press Add / update.");
            }
            None => self.status("New binding: pick a command and press Add / update."),
        }
        // SAFETY: our own control.
        let _ = unsafe { SetFocus(Some(self.c.cmd)) };
    }

    fn command(&mut self, id: u16, code: u16) {
        match (id, code) {
            (ID_MODE, c) if c == CBN_SELCHANGE as u16 => {
                let i = send(self.c.mode, CB_GETCURSEL, 0, 0);
                if i >= 0 {
                    self.select_mode(i as usize);
                }
            }
            (ID_ADD_MODE, _) => self.add_mode(),
            (ID_DEL_MODE, _) => self.remove_mode(),
            (ID_RECORD, _) => self.start_recording(),
            (ID_SET, _) => self.set_binding(),
            (ID_DELETE, _) => self.delete_binding(),
            (ID_SAVE, _) => {
                self.save();
            }
            _ => {}
        }
    }

    /// Asks about unsaved changes. Returns true if the window may close.
    fn may_close(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        let answer = message_box(
            self.hwnd,
            "Save your changes before closing?",
            MB_YESNOCANCEL | MB_ICONWARNING,
        );
        if answer == IDYES.0 {
            self.save()
        } else {
            answer != IDCANCEL.0
        }
    }
}

/// The window the recording hook reports to; 0 when not recording.
static RECORD_TARGET: AtomicIsize = AtomicIsize::new(0);
/// A Win chord was recorded and Win is still down.
static RECORD_WAIT_WIN: AtomicBool = AtomicBool::new(false);

fn pack(chord: Chord) -> usize {
    chord.vk as usize
        | (chord.win as usize) << 16
        | (chord.ctrl as usize) << 17
        | (chord.alt as usize) << 18
        | (chord.shift as usize) << 19
}

fn unpack(w: usize) -> Chord {
    Chord {
        vk: w as u16,
        win: w & 1 << 16 != 0,
        ctrl: w & 1 << 17 != 0,
        alt: w & 1 << 18 != 0,
        shift: w & 1 << 19 != 0,
    }
}

fn post(msg: u32, wparam: usize) {
    let hwnd = HWND(RECORD_TARGET.load(Ordering::Relaxed) as *mut _);
    // SAFETY: posting to our own window.
    let _ = unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(0)) };
}

/// Catches the next chord before anything else sees it, the kerys daemon
/// included: hooks installed later run first.
fn record_key(msg: u32, vk: u32) -> bool {
    if RECORD_TARGET.load(Ordering::Relaxed) == 0 {
        return false;
    }
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    let win = vk == VK_LWIN.0 as u32 || vk == VK_RWIN.0 as u32;
    if up && win && RECORD_WAIT_WIN.swap(false, Ordering::Relaxed) {
        // Win+key was swallowed, so Windows would open Start on release.
        suppress_start_menu();
        post(WM_RECORD_DONE, 0);
        return false;
    }
    if RECORD_WAIT_WIN.load(Ordering::Relaxed) {
        // Already recorded; swallow further keys until Win comes up.
        return down && !is_modifier(vk);
    }
    if !down || is_modifier(vk) {
        return false;
    }
    let chord = Chord {
        win: pressed(VK_LWIN) || pressed(VK_RWIN),
        ctrl: pressed(VK_CONTROL),
        alt: pressed(VK_MENU),
        shift: pressed(VK_SHIFT),
        vk: vk as u16,
    };
    let bare_escape =
        vk == VK_ESCAPE.0 as u32 && !(chord.win || chord.ctrl || chord.alt || chord.shift);
    if bare_escape {
        post(WM_RECORD_DONE, 0);
    } else {
        post(WM_RECORDED, pack(chord));
        if chord.win {
            RECORD_WAIT_WIN.store(true, Ordering::Relaxed);
        } else {
            post(WM_RECORD_DONE, 0);
        }
    }
    true
}

unsafe extern "system" fn record_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if !kb.flags.contains(LLKHF_INJECTED) && record_key(wparam.0 as u32, kb.vkCode) {
            return LRESULT(1);
        }
    }
    // SAFETY: passing the hook arguments on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SIZE => {
            with_gui(|g| g.layout());
        }
        WM_DPICHANGED => {
            // SAFETY: for WM_DPICHANGED, lparam points to the suggested RECT.
            let rc = unsafe { &*(lparam.0 as *const RECT) };
            // SAFETY: resizing our own window as Windows suggests.
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            with_gui(|g| {
                g.set_font();
                g.layout();
            });
            return LRESULT(0);
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            let code = (wparam.0 >> 16) as u16;
            with_gui(|g| g.command(id, code));
        }
        WM_NOTIFY => {
            // SAFETY: for WM_NOTIFY, lparam points to an NMHDR.
            let hdr = unsafe { &*(lparam.0 as *const NMHDR) };
            if hdr.idFrom == ID_LIST as usize && hdr.code == LVN_ITEMCHANGED {
                // SAFETY: LVN_ITEMCHANGED carries an NMLISTVIEW.
                let nm = unsafe { &*(lparam.0 as *const NMLISTVIEW) };
                let selected = |state: u32| state & LVIS_SELECTED.0 != 0;
                if selected(nm.uNewState) && !selected(nm.uOldState) {
                    with_gui(|g| g.show_binding(nm.iItem as usize));
                }
            }
        }
        WM_RECORDED => {
            with_gui(|g| g.recorded(unpack(wparam.0)));
            return LRESULT(0);
        }
        WM_RECORD_DONE => {
            with_gui(|g| g.stop_recording());
            return LRESULT(0);
        }
        WM_CLOSE => {
            if with_gui(|g| g.may_close()).unwrap_or(true) {
                // SAFETY: destroying our own window.
                let _ = unsafe { DestroyWindow(hwnd) };
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            with_gui(|g| g.stop_recording());
            // SAFETY: no preconditions.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: forwarding the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Loads the config for editing, falling back to the defaults if it is
/// broken. The second value says what went wrong, for the user.
fn load(path: &Path) -> (Model, Option<String>) {
    let _ = config::ensure_config(path, &config::rustwm_config_path());
    match config::load(path) {
        Ok(loaded) => (Model::from_loaded(&loaded), None),
        Err(e) => (
            Model::from_loaded(&config::parse("").expect("empty config is valid")),
            Some(format!(
                "{} has a problem, so the window shows the built-in bindings. \
                 Saving replaces the file (the old one is kept as kerys.toml.bak).\n\n{e}",
                path.display()
            )),
        ),
    }
}

/// Shows the settings window and returns when it is closed.
pub fn run(path: &Path) -> anyhow::Result<()> {
    // SAFETY: plain process and common-controls setup calls with valid
    // arguments, then window creation on this thread.
    let hwnd = unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: w!("kerys-gui"),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                None,
                windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
            )?,
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            w!("kerys-gui"),
            w!("kerys keybindings"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
            windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };

    let (model, problem) = load(path);
    let gui = Gui {
        hwnd,
        path: path.to_path_buf(),
        model,
        mode: 0,
        chord: None,
        dirty: false,
        c: create_controls(hwnd),
        font: HFONT::default(),
        record_hook: None,
    };
    GUI.with(|g| *g.borrow_mut() = Some(gui));
    with_gui(|g| {
        let s = |v: i32| v * g.dpi() as i32 / 96;
        // SAFETY: sizing and showing our own window.
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                s(640),
                s(560),
                SWP_NOZORDER | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE,
            );
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        g.set_font();
        g.layout();
        g.select_mode(0);
        g.title();
        g.status(if daemon::is_running() {
            "kerys is running: saved changes apply right away."
        } else {
            "kerys is not running: saved changes apply when it starts."
        });
    });
    if let Some(p) = problem {
        message_box(hwnd, &p, MB_OK | MB_ICONWARNING);
    }

    let mut msg = MSG::default();
    // SAFETY: standard message pump; IsDialogMessageW gives Tab navigation.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    GUI.with(|g| {
        if let Some(g) = g.borrow_mut().take() {
            // SAFETY: the window and its controls are gone.
            let _ = unsafe { DeleteObject(HGDIOBJ(g.font.0)) };
        }
    });
    Ok(())
}
