use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
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

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's
/// `civil_from_days`; UTC only, which is fine for correlating logs - and it
/// keeps the binary free of a date/time dependency.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let time = secs % 86_400;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
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
/// no capability: the frontend never triggers it. A directory opens in
/// Explorer only - Notepad would just complain about it.
pub fn reveal(app: &AppHandle, path: &Path) {
    if path.is_dir() {
        let _ = StdCommand::new("explorer.exe").arg(path).spawn();
        return;
    }
    let _ = StdCommand::new("notepad.exe").arg(path).spawn();
    if let Some(dir) = log_dir(app) {
        let _ = StdCommand::new("explorer.exe").arg(dir).spawn();
    }
}
