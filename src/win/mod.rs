//! The Windows side of kerys: the keyboard hook, the worker that carries out
//! actions, and the message loop that keeps them running.

pub mod daemon;
pub mod hook;
pub mod worker;
