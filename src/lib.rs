//! kerys: a global hotkey daemon for Windows. It turns key chords into
//! rustWM commands (sent over rustWM's named pipe), binding-mode switches,
//! or programs to run.
//!
//! Everything but the keyboard hook builds and tests on any OS.

pub mod action;
pub mod cmdline;
pub mod config;
pub mod keys;
#[cfg(windows)]
pub mod watch;
#[cfg(windows)]
pub mod win;
