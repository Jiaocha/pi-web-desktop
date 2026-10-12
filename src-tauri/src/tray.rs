use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use crate::{logging, ServerHandle};

const SHOW: &str = "show";
const DEVTOOLS: &str = "devtools";
const LOGS: &str = "logs";
const QUIT: &str = "quit";

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id(SHOW, "显示窗口").build(app)?;
    let devtools = MenuItemBuilder::with_id(DEVTOOLS, "开发者工具").build(app)?;
    let logs = MenuItemBuilder::with_id(LOGS, "打开日志目录").build(app)?;
    let quit = MenuItemBuilder::with_id(QUIT, "退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&show, &devtools, &logs, &quit])
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("missing default window icon")))?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Pi Web")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW => focus_main(app),
            DEVTOOLS => open_devtools(app),
            LOGS => open_logs(app),
            QUIT => shutdown(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                focus_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn open_devtools(app: &AppHandle) {
    focus_main(app);
    if let Some(window) = app.get_webview_window("main") {
        window.open_devtools();
    }
}

fn open_logs(app: &AppHandle) {
    let Some(dir) = logging::log_dir(app) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    logging::reveal(app, &dir.join("desktop.log"));
}

fn shutdown(app: &AppHandle) {
    let state = app.state::<ServerHandle>();
    if let Some(child) = state.0.lock().ok().and_then(|mut guard| guard.take()) {
        let _ = child.kill();
    }
    app.exit(0);
}
