//! Native PTY service for the Terminal plugin (spec §62-63). Sessions are
//! session-scoped: they die with the workspace/app. Output is streamed to
//! the owning plugin's iframe via base64-framed pushes.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::fs::b64_encode;
use crate::{CallerCtx, Kernel, KernelError};

pub struct PtySession {
    pub id: String,
    pub shell: String,
    pub cwd: String,
    pub created_at: String,
    /// Owning plugin (None = the application shell itself).
    pub owner: Option<String>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn std::io::Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pub alive: AtomicBool,
}

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<String, Arc<PtySession>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The plugin that created a session (None for app-created sessions).
    pub fn owner_of(&self, session_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap()
            .get(session_id)
            .and_then(|s| s.owner.clone())
    }

    pub fn list(&self) -> Vec<serde_json::Value> {
        let sessions = self.sessions.lock().unwrap();
        let mut out: Vec<serde_json::Value> = sessions
            .values()
            .map(|s| {
                serde_json::json!({
                    "sessionId": s.id,
                    "shell": s.shell,
                    "cwd": s.cwd,
                    "createdAt": s.created_at,
                    "alive": s.alive.load(Ordering::SeqCst),
                })
            })
            .collect();
        out.sort_by(|a, b| a["sessionId"].as_str().unwrap_or("").cmp(b["sessionId"].as_str().unwrap_or("")));
        out
    }

    pub fn get(&self, id: &str) -> Option<Arc<PtySession>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    fn remove(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }

    pub fn kill_all(&self) {
        let sessions: Vec<Arc<PtySession>> = self.sessions.lock().unwrap().values().cloned().collect();
        for session in sessions {
            session.kill();
        }
    }
}

impl PtySession {
    pub fn write(&self, data: &[u8]) -> Result<(), KernelError> {
        let mut writer = self.writer.lock().unwrap();
        writer
            .write_all(data)
            .and_then(|_| writer.flush())
            .map_err(|e| KernelError::Message(format!("pty write failed: {e}")))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), KernelError> {
        self.master
            .lock()
            .unwrap()
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| KernelError::Message(format!("pty resize failed: {e}")))
    }

    pub fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
        self.alive.store(false, Ordering::SeqCst);
    }
}

/// Create a PTY session. Requires the `process:spawn` permission for plugin
/// callers (spec §63).
pub fn create(
    kernel: &Arc<Kernel>,
    caller: &CallerCtx,
    shell: Option<&str>,
    cwd: Option<&str>,
    cols: u16,
    rows: u16,
    env: Option<&serde_json::Value>,
) -> Result<serde_json::Value, KernelError> {
    if let Some(plugin_id) = &caller.plugin_id {
        kernel
            .permissions
            .check_flag(plugin_id, "process:spawn")
            .map_err(|e| KernelError::Permission(e.to_string()))?;
    }

    let shell = shell
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            kernel
                .settings
                .get("core.terminal.shell")
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/bash".to_string());

    let cwd_path = match cwd {
        Some(c) if !c.trim().is_empty() => kernel.resolve_path(c)?,
        _ => kernel
            .current_workspace()
            .map(|ws| ws.workspace.root().to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir()),
    };

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| KernelError::Message(format!("openpty failed: {e}")))?;

    let mut cmd = CommandBuilder::new(&shell);
    if cwd_path.is_dir() {
        cmd.cwd(&cwd_path);
    }
    cmd.env("TERM", "xterm-256color");
    if let Some(env_map) = env.and_then(|e| e.as_object()) {
        for (k, v) in env_map {
            if let Some(value) = v.as_str() {
                if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.is_empty() {
                    cmd.env(k, value);
                }
            }
        }
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| KernelError::Message(format!("failed to spawn `{shell}`: {e}")))?;
    let killer = child.clone_killer();
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| KernelError::Message(format!("pty reader failed: {e}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| KernelError::Message(format!("pty writer failed: {e}")))?;

    let session_id = format!("pty-{}", uuid::Uuid::new_v4().simple());
    let session = Arc::new(PtySession {
        id: session_id.clone(),
        shell: shell.clone(),
        cwd: cwd_path.display().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        owner: caller.plugin_id.clone(),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        killer: Mutex::new(killer),
        alive: AtomicBool::new(true),
    });
    kernel.pty.sessions.lock().unwrap().insert(session_id.clone(), session.clone());

    let target = caller.plugin_id.clone();
    let target_read = target.clone();
    let kernel_for_read = kernel.clone();
    let sid = session_id.clone();
    std::thread::Builder::new()
        .name(format!("ed-{sid}-read"))
        .spawn(move || {
            let mut buf = [0u8; 16384];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = b64_encode(&buf[..n]);
                        match &target_read {
                            Some(plugin) => {
                                kernel_for_read
                                    .push
                                    .push_to_plugin(plugin, "pty", serde_json::json!({ "sessionId": sid, "data": data }));
                            }
                            None => {
                                kernel_for_read
                                    .push
                                    .push("pty", None, serde_json::json!({ "sessionId": sid, "data": data }));
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(session = %sid, error = %e, "pty read error");
                        break;
                    }
                }
            }
        })
        .expect("spawn pty reader thread");

    let kernel_for_wait = kernel.clone();
    let sid = session_id.clone();
    let session_for_wait = session.clone();
    let target_wait = target.clone();
    std::thread::Builder::new()
        .name(format!("ed-{sid}-wait"))
        .spawn(move || {
            let status = child.wait();
            session_for_wait.alive.store(false, Ordering::SeqCst);
            let success = status.map(|s| s.success()).unwrap_or(false);
            match &target_wait {
                Some(plugin) => {
                    kernel_for_wait
                        .push
                        .push_to_plugin(plugin, "pty", serde_json::json!({ "sessionId": sid, "kind": "exit", "success": success }));
                }
                None => {
                    kernel_for_wait
                        .push
                        .push("pty", None, serde_json::json!({ "sessionId": sid, "kind": "exit", "success": success }));
                }
            }
            kernel_for_wait.events.emit(
                "terminal.exited",
                target_wait.clone(),
                serde_json::json!({ "sessionId": sid, "success": success }),
            );
            kernel_for_wait.pty.remove(&sid);
        })
        .expect("spawn pty wait thread");

    kernel.events.emit(
        "terminal.created",
        target.clone(),
        serde_json::json!({ "sessionId": session_id, "shell": shell }),
    );

    Ok(serde_json::json!({
        "sessionId": session_id,
        "shell": shell,
        "cwd": cwd_path.display().to_string(),
    }))
}
