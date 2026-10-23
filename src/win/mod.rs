//! The Windows side of kerys: the keyboard hook, the worker that carries out
//! actions, the message loop that keeps them running, and the settings window.

pub mod daemon;
pub mod gui;
pub mod hook;
pub mod worker;
