use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{path::BaseDirectory, AppHandle, Manager};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

/// Asks the OS for a free loopback port. There is a tiny race between dropping
/// this listener and the sidecar binding it; `wait_until_ready` plus the
/// sidecar's own output makes a collision visible instead of silent.
pub fn pick_free_port() -> tauri::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn app_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    app.path().resolve("app", BaseDirectory::Resource)
}

pub fn spawn(app: &AppHandle, port: u16) -> tauri::Result<CommandChild> {
    let dir = app_dir(app)?;
    let entry = dir.join("bin").join("pi-web.js");

    let command = app
        .shell()
        .sidecar("node")?
        .current_dir(&dir)
        // proxy.ts disables the login gate when PI_WEB_PASSWORD is falsy, and
        // an empty string is falsy; never inherit one from the environment.
        .env("PI_WEB_PASSWORD", "")
        .env("PI_WEB_HOSTNAME", "127.0.0.1")
        .env("PI_WEB_DESKTOP", "1")
        .args([
            entry.to_string_lossy().as_ref(),
            "start",
            "-H",
            "127.0.0.1",
            "-p",
            &port.to_string(),
        ]);

    let (mut rx, child) = command.spawn()?;

    // Mirror the server log so `tauri dev` shows what the sidecar is doing.
    // Swap this for tauri-plugin-log once a log target is configured.
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) | CommandEvent::Stderr(line) => {
                    eprintln!("pi-web: {}", String::from_utf8_lossy(&line).trim_end());
                }
                CommandEvent::Terminated(payload) => {
                    eprintln!("pi-web exited: {:?}", payload);
                }
                _ => {}
            }
        }
    });

    Ok(child)
}

pub fn wait_until_ready(url: &str, timeout: Duration) -> tauri::Result<()> {
    let authority = url.strip_prefix("http://").unwrap_or(url);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(authority).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err(tauri::Error::Anyhow(anyhow::anyhow!(
        "pi-web sidecar did not start within {timeout:?}"
    )))
}
