//! Talking to Slowshell, the desktop shell, over its control pipe.
//!
//! Slowshell's pipe name carries its process id, so it writes down where it
//! listens in `%LOCALAPPDATA%\Slowshell\control.json`, `{"pipe": ..., "pid": ...}`.
//! `shellctl` finds it the same way. Each request is one line of JSON on a fresh
//! connection and gets one line back: the shell serves one request per
//! connection, so there is nothing worth keeping open.

use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// `ERROR_PIPE_BUSY`: every server instance is serving another client.
const ERROR_PIPE_BUSY: i32 = 231;

/// Why a request did not reach the shell.
#[derive(Debug)]
pub enum Error {
    /// No Slowshell is running (or it never published where it listens).
    NotRunning,
    /// The shell answered, and said no.
    Refused(String),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => f.write_str("Slowshell is not running"),
            Error::Refused(why) => write!(f, "Slowshell refused it: {why}"),
            Error::Io(e) => write!(f, "cannot reach Slowshell: {e}"),
        }
    }
}

fn endpoint_file() -> Option<PathBuf> {
    // Mirrors `slowshell_core::paths::state_dir`.
    if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(local).join("Slowshell").join("control.json"));
    }
    let roaming = std::env::var_os("APPDATA").filter(|v| !v.is_empty())?;
    Some(
        PathBuf::from(roaming)
            .join("Slowshell")
            .join("state")
            .join("control.json"),
    )
}

/// The pipe a running Slowshell listens on.
fn pipe() -> Result<String, Error> {
    let text = std::fs::read_to_string(endpoint_file().ok_or(Error::NotRunning)?)
        .map_err(|_| Error::NotRunning)?;
    let endpoint: serde_json::Value = serde_json::from_str(&text).map_err(|_| Error::NotRunning)?;
    endpoint["pipe"]
        .as_str()
        .map(str::to_owned)
        .ok_or(Error::NotRunning)
}

/// Sends one request and waits for the shell's answer.
///
/// A file left behind by a shell that crashed names a pipe nobody serves, which
/// opening reports as not found, so that also reads as `NotRunning`.
pub fn request(request: &serde_json::Value) -> Result<(), Error> {
    let pipe = pipe()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let file = loop {
        match OpenOptions::new().read(true).write(true).open(&pipe) {
            Ok(f) => break f,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::NotRunning),
            Err(e) => return Err(Error::Io(e)),
        }
    };
    let mut writer = file.try_clone().map_err(Error::Io)?;
    let mut line = serde_json::to_vec(request).map_err(|e| Error::Io(e.into()))?;
    line.push(b'\n');
    writer.write_all(&line).map_err(Error::Io)?;
    writer.flush().map_err(Error::Io)?;

    let mut reply = String::new();
    BufReader::new(file)
        .read_line(&mut reply)
        .map_err(Error::Io)?;
    let reply: serde_json::Value = serde_json::from_str(&reply)
        .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
    if reply["status"] == "ok" {
        Ok(())
    } else {
        let why = reply["message"].as_str().unwrap_or("request failed");
        Err(Error::Refused(why.to_owned()))
    }
}
