#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod server;
pub mod tray;
pub mod updater;

use std::sync::Mutex;

use tauri::{Manager, RunEvent};
use tauri_plugin_shell::process::CommandChild;

/// Holds the sidecar handle so it can be killed before the process exits.
/// Without this, closing the window leaves `node.exe` running in the background.
pub struct ServerHandle(pub Mutex<Option<CommandChild>>);

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(ServerHandle(Mutex::new(None)))
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

            // 1. Pick a free loopback port; the port cannot be hardcoded
            //    because a Pi Web dev server may already own 30141.
            let port = server::pick_free_port()?;
            let url = format!("http://127.0.0.1:{port}");

            // 2. Start the Next.js server through the official launcher.
            let child = server::spawn(&handle, port)?;
            handle
                .state::<ServerHandle>()
                .0
                .lock()
                .expect("server handle mutex")
                .replace(child);

            // 3. Wait until it accepts connections before showing a window.
            server::wait_until_ready(&url, std::time::Duration::from_secs(30))?;

            // 4. Create the real window pointed at the sidecar.
            tauri::WebviewWindowBuilder::new(
                &handle,
                "main",
                tauri::WebviewUrl::External(url.parse()?),
            )
            .title("Pi Web")
            .inner_size(1280.0, 850.0)
            .min_inner_size(900.0, 620.0)
            // Phase 1 (M1): keep the system titlebar. The frameless variant
            // needs a frontend titlebar with data-tauri-drag-region, which is
            // tracked separately (PLAN.md T5).
            .decorations(true)
            .visible(true)
            .build()?;

            tray::build(&handle)?;

            // 5. Update check; never blocks startup.
            updater::check(&handle);

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
