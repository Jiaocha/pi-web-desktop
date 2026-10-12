use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

/// Ring buffer of the last sidecar lines, kept in memory for the failure
/// report and mirrored into the log file.
pub struct ServerLog {
    lines: Mutex<Vec<String>>,
}

impl ServerLog {
    pub fn new() -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, line: &str) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(line.to_string());
            if lines.len() > 500 {
                let overflow = lines.len() - 500;
                lines.drain(..overflow);
            }
        }
    }

    pub fn tail(&self, count: usize) -> String {
        let lines = self.lines.lock().ok();
        let Some(lines) = lines else {
            return String::new();
        };
        lines
            .iter()
            .rev()
            .take(count)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Log directory: `<root>/data/logs` in portable mode, otherwise
/// `<app data>/logs`. Always written, even in release builds, because a
/// window-less failure is otherwise invisible.
pub fn log_dir(app: &AppHandle) -> Option<PathBuf> {
    if let Some(dir) = crate::portable::data_dir() {
        return Some(dir.join("logs"));
    }
    app.path().app_data_dir().ok().map(|dir| dir.join("logs"))
}

fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    // Whole-second UTC is enough to correlate with the sidecar's own output.
    format!("{secs}")
}

/// Appends one line to `<log dir>/desktop.log`, creating the directory on
/// first use. Silently ignores I/O errors: logging must never break startup.
pub fn write(app: &AppHandle, line: &str) {
    let Some(dir) = log_dir(app) else { return };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("desktop.log"))
    else {
        return;
    };
    let _ = writeln!(file, "[{}] {}", timestamp(), line);
}

/// Opens a file with the OS default handler (detached). Rust-side, so it needs
/// no capability: the frontend never triggers it.
pub fn reveal(app: &AppHandle, file: &PathBuf) {
    let _ = StdCommand::new("notepad.exe").arg(file).spawn();
    if let Some(dir) = log_dir(app) {
        let _ = StdCommand::new("explorer.exe").arg(dir).spawn();
    }
}
