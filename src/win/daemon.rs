//! The daemon's message loop, single-instance guard and `kerys stop`.

use std::io;

use tracing::{debug, warn};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW, GetMessageW,
    HWND_MESSAGE, MSG, PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WNDCLASSW,
};
use windows::core::w;

use super::hook;

/// Holds the named mutex that marks kerys as running, for the life of the
/// process.
pub struct Instance(HANDLE);

impl Instance {
    /// Claims the instance. `Ok(None)` if another kerys is already running.
    pub fn claim() -> io::Result<Option<Self>> {
        // SAFETY: valid static name; no security attributes.
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\kerys")) }?;
        // SAFETY: no preconditions; reads this thread's last error.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: closing the handle we were just given.
            let _ = unsafe { CloseHandle(handle) };
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: closing the handle we own.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CLOSE {
        // SAFETY: no preconditions.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: forwarding the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A message-only window whose class name lets `kerys stop` find us.
fn create_window() -> Option<HWND> {
    // SAFETY: standard class registration and window creation with static
    // wide strings.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: w!("kerys"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("kerys"),
            w!("kerys"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

/// Installs the keyboard hook and pumps messages on this thread until
/// `kerys stop` (or anything else) closes our window.
pub fn run() -> anyhow::Result<()> {
    let Some(hook) = hook::install() else {
        anyhow::bail!("could not install the keyboard hook");
    };
    let window = create_window();
    if window.is_none() {
        warn!("could not create the control window; `kerys stop` will not find this kerys");
    }
    debug!("message loop running");
    let mut msg = MSG::default();
    // SAFETY: standard message pump; GetMessageW returns 0 on WM_QUIT, -1 on error.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(w) = window {
            let _ = DestroyWindow(w);
        }
    }
    hook::uninstall(hook);
    Ok(())
}

/// Asks a running kerys to quit. Returns false if none is running.
pub fn stop() -> bool {
    // SAFETY: finding a message-only window by its static class name, then
    // posting it a message.
    unsafe {
        match FindWindowExW(Some(HWND_MESSAGE), None, w!("kerys"), None) {
            Ok(hwnd) if !hwnd.is_invalid() => {
                PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
            }
            _ => false,
        }
    }
}
