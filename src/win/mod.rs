//! The Windows side of kerys: the keyboard hook, the worker that carries out
//! actions, the message loop that keeps them running, the settings window, and
//! the line to Slowshell, the desktop shell.

pub mod daemon;
pub mod gui;
pub mod hook;
pub mod shell;
pub mod worker;
