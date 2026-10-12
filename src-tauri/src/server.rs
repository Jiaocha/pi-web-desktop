use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{path::BaseDirectory, AppHandle, Manager};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::logging::{self, ServerLog};
use crate::ServerHandle;

/// Asks the OS for a free loopback port. There is a tiny race between dropping
/// this listener and the sidecar binding it; a collision shows up as a failed
/// health check instead of a silent hang.
pub fn pick_free_port() -> tauri::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Tauri's `path().resolve()` returns verbatim paths on Windows
/// (`\\?\D:\...`). CreateProcessW rejects those as the program name with
/// "The system cannot find the file specified" (os error 2) - which is exactly
/// how the portable build failed - so strip the prefix before spawning.
fn simplify(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

/// The bundled Node runtime keeps Tauri's externalBin name, so both the NSIS
/// install and the portable zip end up with the same file next to the exe.
fn node_runtime_name() -> &'static str {
    if cfg!(all(windows, target_arch = "x86_64")) {
        "node-x86_64-pc-windows-msvc.exe"
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        "node-aarch64-pc-windows-msvc.exe"
    } else if cfg!(windows) {
        "node.exe"
    } else {
        "node"
    }
}

fn node_runtime(app: &AppHandle) -> tauri::Result<PathBuf> {
    let name = node_runtime_name();
    let root = crate::portable::root_dir().ok_or_else(|| {
        tauri::Error::Anyhow(anyhow::anyhow!("cannot determine the exe directory"))
    })?;
    let direct = root.join(name);
    if direct.exists() {
        return Ok(simplify(direct));
    }
    if let Ok(resolved) = app.path().resolve(name, BaseDirectory::Resource) {
        if resolved.exists() {
            return Ok(simplify(resolved));
        }
    }
    Err(tauri::Error::Anyhow(anyhow::anyhow!(
        "node runtime {name} not found in {}",
        root.display()
    )))
}

/// Where the Pi Web payload lives: the resource dir for the installed layout,
/// `<exe dir>/app` for the portable zip.
fn app_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    if let Ok(dir) = app.path().resolve("app", BaseDirectory::Resource) {
        let dir = simplify(dir);
        if dir.join("bin").join("pi-web.js").exists() {
            return Ok(dir);
        }
    }
    if let Some(dir) = crate::portable::root_dir().map(|root| root.join("app")) {
        if dir.join("bin").join("pi-web.js").exists() {
            return Ok(dir);
        }
    }
    Err(tauri::Error::Anyhow(anyhow::anyhow!(
        "pi-web payload not found next to the executable or in the resource dir"
    )))
}

/// Copies the child's output into the log ring and the log file. Without this
/// the pipes fill up and the server blocks on write.
fn pump(reader: impl std::io::Read + Send + 'static, log: Arc<ServerLog>, app: AppHandle) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = line.trim_end().to_string();
                    if !text.is_empty() {
                        log.push(&text);
                        logging::write(&app, &format!("pi-web: {text}"));
                        eprintln!("pi-web: {text}");
                    }
                }
            }
        }
    });
}

pub fn spawn(app: &AppHandle, port: u16, log: &Arc<ServerLog>) -> tauri::Result<Child> {
    let dir = app_dir(app)?;
    let entry = dir.join("bin").join("pi-web.js");
    let node = node_runtime(app)?;

    logging::write(app, &format!("node: {}", node.display()));
    logging::write(app, &format!("app dir: {}", dir.display()));
    logging::write(app, &format!("entry: {}", entry.display()));
    logging::write(app, &format!("port: {port}"));

    let mut command = StdCommand::new(&node);
    command
        .current_dir(&dir)
        // proxy.ts disables the login gate when PI_WEB_PASSWORD is falsy, and
        // an empty string is falsy; never inherit one from the environment.
        .env("PI_WEB_PASSWORD", "")
        .env("PI_WEB_HOSTNAME", "127.0.0.1")
        .env("PI_WEB_DESKTOP", "1")
        .env("PI_WEB_NO_OPEN", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // `pi-web [options]` starts the server; there is no `start` subcommand.
        // --no-open keeps the OS browser from opening on top of the app window.
        .args([
            entry.to_string_lossy().as_ref(),
            "-H",
            "127.0.0.1",
            "-p",
            &port.to_string(),
            "--no-open",
        ]);

    // Portable mode: pi-web keeps every byte of state (sessions, settings, the
    // run records the CLI writes) under its agent dir, so pointing that into
    // data/ is what makes the folder self-contained.
    if let Some(data) = crate::portable::data_dir() {
        let agent = data.join("pi").join("agent");
        if let Err(error) = std::fs::create_dir_all(&agent) {
            logging::write(app, &format!("agent dir {}: {error}", agent.display()));
        }
        logging::write(app, &format!("agent dir: {}", agent.display()));
        command.env("PI_CODING_AGENT_DIR", agent.to_string_lossy().as_ref());
    }

    // node.exe is a console subsystem binary; without this flag spawning it
    // from a GUI app pops up a console window.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = command.spawn().map_err(|error| {
        tauri::Error::Anyhow(anyhow::anyhow!("spawn {} failed: {error}", node.display()))
    })?;

    if let Some(stdout) = child.stdout.take() {
        pump(stdout, log.clone(), app.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, log.clone(), app.clone());
    }

    Ok(child)
}

/// Kills the sidecar and everything it spawned. `Child::kill()` only
/// terminates the direct child, but pi-web forks Next.js worker processes:
/// those survive, keep the port bound and hold the agent dir open, so the next
/// launch fails to start. On Windows `taskkill /T` walks the whole tree.
pub fn kill_tree(app: &AppHandle, child: &mut Child) {
    #[cfg(windows)]
    {
        let pid = child.id();
        logging::write(app, &format!("stopping sidecar tree (pid {pid})"));
        let mut command = StdCommand::new("taskkill");
        command
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let _ = command.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Waits for the sidecar to accept connections. Fails fast when the child
/// exits, reporting its exit code and output instead of burning the timeout.
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
        // A MutexGuard held across the poll would deadlock the pump threads'
        // logging on the same state, so drop it immediately.
        let status = {
            let state = app.state::<ServerHandle>();
            state.0.lock().ok().and_then(|mut guard| {
                guard
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten())
            })
        };
        if let Some(status) = status {
            return Err(tauri::Error::Anyhow(anyhow::anyhow!(
                "pi-web sidecar exited before it was ready (code {:?}).\n{}",
                status.code(),
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
