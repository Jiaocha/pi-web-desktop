use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;

/// Checks for an update and offers to install it. Silent on failure: a broken
/// update server must never block the app from starting.
pub fn check(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let updater = match handle.updater() {
            Ok(updater) => updater,
            Err(error) => {
                eprintln!("updater unavailable: {error}");
                return;
            }
        };

        match updater.check().await {
            Ok(Some(update)) => {
                let install = handle
                    .dialog()
                    .message(format!("发现新版本 {}，是否立即更新？", update.version))
                    .title("Pi Web 更新")
                    .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancel)
                    .blocking_show();

                if !install {
                    return;
                }

                match update.download_and_install(|_, _| {}, || {}).await {
                    // Windows installers quit the app on install; relaunch is
                    // handled by NSIS, restart() only covers the other targets.
                    Ok(()) => handle.restart(),
                    Err(error) => eprintln!("update failed: {error}"),
                }
            }
            Ok(None) => {}
            Err(error) => eprintln!("update check failed: {error}"),
        }
    });
}
