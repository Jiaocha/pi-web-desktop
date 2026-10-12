//! Portable ("green") mode.
//!
//! When the build ships a `portable.txt` next to the executable, everything the
//! app writes stays inside the extracted folder:
//!
//!   <root>/data/logs     desktop.log, startup-failure.txt
//!   <root>/data/webview  WebView2 user data (cookies, localStorage)
//!   <root>/data/pi       pi-web's agent dir (sessions, settings, run records)
//!
//! Without the marker the app behaves exactly as the installed build does and
//! uses the per-user app data directory, so one binary serves both layouts.

use std::path::PathBuf;

/// Marker file the portable build drops next to the exe.
pub const MARKER: &str = "portable.txt";

/// Directory the executable lives in: the root of the extracted zip.
pub fn root_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
}

pub fn is_portable() -> bool {
    root_dir()
        .map(|root| root.join(MARKER).is_file())
        .unwrap_or(false)
}

/// `<root>/data` in portable mode, `None` otherwise so callers can fall back
/// to the per-user app data directory.
pub fn data_dir() -> Option<PathBuf> {
    let root = root_dir()?;
    if root.join(MARKER).is_file() {
        Some(root.join("data"))
    } else {
        None
    }
}
