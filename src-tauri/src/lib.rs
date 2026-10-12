#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod logging;
pub mod portable;
pub mod server;
pub mod tray;
pub mod updater;

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Manager, RunEvent};
use tauri_plugin_shell::process::CommandChild;

use logging::ServerLog;

/// Holds the sidecar handle and its log ring so both can be killed on exit
/// and reported on failure.
pub struct ServerHandle(pub Mutex<Option<CommandChild>>, pub Arc<ServerLog>);

/// PI_WEB_DESKTOP_DEBUG=1 opens devtools on startup; the log file is always
/// written, so diagnostics work in release builds too.
fn debug_enabled() -> bool {
    matches!(
        std::env::var("PI_WEB_DESKTOP_DEBUG").ok().as_deref(),
        Some(value) if !value.is_empty() && value != "0"
    )
}

/// PI_WEB_DESKTOP_DEV_URL points the shell at an already-running server
/// (e.g. http://127.0.0.1:30141). The sidecar is skipped entirely, so
/// frontend iteration needs no payload rebuild and no reinstall.
fn dev_url() -> Option<String> {
    std::env::var("PI_WEB_DESKTOP_DEV_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn boot(app: &tauri::AppHandle) -> tauri::Result<String> {
    if let Some(url) = dev_url() {
        logging::write(app, &format!("dev url override: {url} (sidecar skipped)"));
        // Still manage an empty handle: RunEvent::Exit and the tray read it.
        app.manage(ServerHandle(Mutex::new(None), Arc::new(ServerLog::new())));
        return Ok(url);
    }

    let port = server::pick_free_port()?;
    let url = format!("http://127.0.0.1:{port}");
    let log = Arc::new(ServerLog::new());

    let child = server::spawn(app, port, &log)?;
    app.manage(ServerHandle(Mutex::new(Some(child)), log.clone()));

    server::wait_until_ready(app, &url, Duration::from_secs(30), &log)?;
    Ok(url)
}

fn report_failure(app: &tauri::AppHandle, error: &str) {
    logging::write(app, &format!("startup failed: {error}"));
    eprintln!("startup failed: {error}");

    let Some(dir) = logging::log_dir(app) else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join("startup-failure.txt");
    if fs::write(&file, error).is_ok() {
        // Notepad + Explorer: a modal dialog inside setup would block before
        // the event loop exists, and a silent exit is what cost us a release.
        logging::reveal(app, &file);
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Closing the window hides it to the tray: exiting here would abort
        // running agent sessions. The tray's "退出" is the real exit path.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            logging::write(&handle, "starting Pi Web Desktop");
            let portable = portable::is_portable();
            logging::write(&handle, &format!("portable mode: {portable}"));
            if portable {
                logging::write(
                    &handle,
                    &format!(
                        "portable root: {}",
                        portable::root_dir().map_or_else(
                            || "<unknown>".to_string(),
                            |root| root.display().to_string()
                        )
                    ),
                );
            }

            match boot(&handle) {
                Ok(url) => {
                    logging::write(&handle, &format!("window url: {url}"));
                    // Phase 1 (M1): keep the system titlebar. The frameless
                    // variant needs a frontend titlebar with
                    // data-tauri-drag-region (PLAN.md T5).
                    let mut builder = tauri::WebviewWindowBuilder::new(
                        &handle,
                        "main",
                        tauri::WebviewUrl::External(url.parse()?),
                    )
                    .title(if dev_url().is_some() {
                        "Pi Web (dev)"
                    } else {
                        "Pi Web"
                    })
                    .inner_size(1280.0, 850.0)
                    .min_inner_size(900.0, 620.0)
                    .decorations(true)
                    .visible(true);

                    // Portable: keep the WebView2 profile (cookies,
                    // localStorage) inside the folder next to the exe instead
                    // of %APPDATA%.
                    if let Some(data) = portable::data_dir() {
                        builder = builder.data_directory(data.join("webview"));
                    }
                    builder.build()?;

                    // devtools feature is enabled, so this works in release too.
                    if debug_enabled() {
                        if let Some(window) = handle.get_webview_window("main") {
                            window.open_devtools();
                        }
                    }

                    tray::build(&handle)?;
                    // The updater installs an NSIS package, which a portable
                    // copy cannot accept: it would write to Program Files and
                    // clobber a folder the user never installed. Portable users
                    // re-download the zip.
                    if portable {
                        logging::write(&handle, "updater skipped: portable build");
                    } else {
                        updater::check(&handle);
                    }
                }
                Err(error) => {
                    report_failure(&handle, &format!("{error}"));
                    handle.exit(1);
                }
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Tauri application");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            // Bind the child first: a MutexGuard temporary in an `if let`
            // scrutinee would outlive `state` (E0597).
            let child = {
                let state = app_handle.state::<ServerHandle>();
                state.0.lock().ok().and_then(|mut guard| guard.take())
            };
            if let Some(child) = child {
                let _ = child.kill();
            }
        }
    });
}
