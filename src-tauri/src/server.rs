use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{path::BaseDirectory, AppHandle, Manager};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

use crate::logging::{self, ServerLog};

/// Asks the OS for a free loopback port. There is a tiny race between dropping
/// this listener and the sidecar binding it; a collision shows up as a failed
/// health check instead of a silent hang.
pub fn pick_free_port() -> tauri::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn app_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    app.path().resolve("app", BaseDirectory::Resource)
}

/// The shell plugin's error type has no `From` impl into `tauri::Error`.
fn shell_error<E: std::fmt::Display>(error: E) -> tauri::Error {
    tauri::Error::Anyhow(anyhow::anyhow!("{error}"))
}

pub fn spawn(app: &AppHandle, port: u16, log: &Arc<ServerLog>) -> tauri::Result<CommandChild> {
    let dir = app_dir(app)?;
    let entry = dir.join("bin").join("pi-web.js");
    logging::write(app, &format!("app dir: {}", dir.display()));
    logging::write(app, &format!("entry: {}", entry.display()));
    logging::write(app, &format!("port: {port}"));

    if !entry.exists() {
        return Err(tauri::Error::Anyhow(anyhow::anyhow!(
            "pi-web entry not found: {}",
            entry.display()
        )));
    }

    // `pi-web [options]` starts the server; there is no `start` subcommand.
    // --no-open keeps the OS browser from opening on top of the app window.
    let command = app
        .shell()
        .sidecar("node")
        .map_err(shell_error)?
        .current_dir(&dir)
        // proxy.ts disables the login gate when PI_WEB_PASSWORD is falsy, and
        // an empty string is falsy; never inherit one from the environment.
        .env("PI_WEB_PASSWORD", "")
        .env("PI_WEB_HOSTNAME", "127.0.0.1")
        .env("PI_WEB_DESKTOP", "1")
        .env("PI_WEB_NO_OPEN", "1")
        .args([
            entry.to_string_lossy().as_ref(),
            "-H",
            "127.0.0.1",
            "-p",
            &port.to_string(),
            "--no-open",
        ]);

    let (mut rx, child) = command.spawn().map_err(shell_error)?;

    let sink = log.clone();
    let sink_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) | CommandEvent::Stderr(line) => {
                    let text = String::from_utf8_lossy(&line).trim_end().to_string();
                    if !text.is_empty() {
                        sink.push(&text);
                        logging::write(&sink_app, &format!("pi-web: {text}"));
                        eprintln!("pi-web: {text}");
                    }
                }
                CommandEvent::Terminated(payload) => {
                    let message = format!("pi-web exited: {:?}", payload);
                    sink.push(&message);
                    logging::write(&sink_app, &message);
                }
                _ => {}
            }
        }
    });

    Ok(child)
}

/// Waits for the sidecar to accept connections. Fails fast when the child
/// exits with its output attached, instead of burning the full timeout.
pub fn wait_until_ready(
    app: &AppHandle,
    url: &str,
    timeout: Duration,
    log: &Arc<ServerLog>,
) -> tauri::Result<()> {
    let authority = url.strip_prefix("http://").unwrap_or(url);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(authority).is_ok() {
            logging::write(app, "sidecar ready");
            return Ok(());
        }
        if let Some(code) = exit_code(log) {
            return Err(tauri::Error::Anyhow(anyhow::anyhow!(
                "pi-web sidecar exited before it was ready (code {code}).\n{}",
                log.tail(40)
            )));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err(tauri::Error::Anyhow(anyhow::anyhow!(
        "pi-web sidecar did not start within {timeout:?}.\n{}",
        log.tail(40)
    )))
}

/// Exit code observed in the log ring once the child terminates.
fn exit_code(log: &Arc<ServerLog>) -> Option<i32> {
    let text = log.tail(5);
    text.lines().rev().find_map(|line| {
        let rest = line.split("code: Some(").nth(1)?;
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    })
}
