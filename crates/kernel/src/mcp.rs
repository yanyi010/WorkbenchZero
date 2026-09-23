//! MCP (Model Context Protocol) client support over stdio transport
//! (spec §55-57). The kernel spawns external MCP servers declared in the
//! user's config, performs the JSON-RPC 2.0 handshake, lists tools and
//! forwards tool calls. Plugin callers need the `mcp:connect` permission.
//! Nothing is exposed to external clients without explicit per-server
//! enablement (capability selection).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use wz_common::{atomic_write_str, load_json, JsonLoad, MutexRecover};

use crate::{CallerCtx, KResult, Kernel, KernelError};

const PROTOCOL_VERSION: &str = "2024-11-05";
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

pub struct McpConnection {
    child: Child,
    stdin: std::process::ChildStdin,
    pending: Arc<Mutex<HashMap<u64, std::sync::mpsc::Sender<Value>>>>,
    next_id: Arc<AtomicU64>,
    pub server_info: Value,
    pub tools: Vec<Value>,
}

pub struct McpManager {
    config_path: Mutex<Option<PathBuf>>,
    connections: Mutex<HashMap<String, Arc<Mutex<McpConnection>>>>,
    /// Serializes config read-modify-write so concurrent mutations cannot
    /// lose each other.
    config_ops: Mutex<()>,
    /// Servers currently mid-handshake (connect TOCTOU guard).
    connecting: Mutex<std::collections::HashSet<String>>,
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            config_path: Mutex::new(None),
            connections: Mutex::new(HashMap::new()),
            config_ops: Mutex::new(()),
            connecting: Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub fn set_config_path(&self, path: PathBuf) {
        *self.config_path.lock_or_recover() = Some(path);
    }

    fn read_config(&self) -> Vec<ServerConfig> {
        let path = self.config_path.lock_or_recover().clone();
        let Some(path) = path else { return vec![] };
        match load_json(&path) {
            Ok(JsonLoad::Loaded(v)) | Ok(JsonLoad::RecoveredFromTmp(v)) => v,
            Ok(JsonLoad::Missing) => vec![],
            Ok(JsonLoad::Corrupt { quarantined_to }) => {
                tracing::error!(quarantined = %quarantined_to.display(), "mcp config corrupt; servers list reset, corrupt copy preserved");
                vec![]
            }
            Err(e) => {
                tracing::warn!(error = %e, "mcp config unreadable");
                vec![]
            }
        }
    }

    /// Caller must hold `config_ops`.
    fn write_config(&self, servers: &[ServerConfig]) -> KResult<()> {
        let path = self.config_path.lock_or_recover().clone();
        let Some(path) = path else {
            return Err(KernelError::Message("mcp config path not set".into()));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write_str(&path, &serde_json::to_string_pretty(servers)?)
            .map_err(|e| KernelError::Message(format!("cannot write mcp config: {e}")))?;
        Ok(())
    }

    pub fn servers(&self) -> Vec<ServerConfig> {
        self.read_config()
    }

    pub fn add_server(&self, name: &str, command: &str, args: &[String]) -> KResult<()> {
        let _op = self.config_ops.lock_or_recover();
        let mut servers = self.read_config();
        if servers.iter().any(|s| s.name == name) {
            return Err(KernelError::Message(format!(
                "mcp server `{name}` already exists"
            )));
        }
        servers.push(ServerConfig {
            name: name.to_string(),
            command: command.to_string(),
            args: args.to_vec(),
            enabled: true,
        });
        self.write_config(&servers)
    }

    pub fn remove_server(&self, name: &str) -> KResult<()> {
        let _op = self.config_ops.lock_or_recover();
        let mut servers = self.read_config();
        servers.retain(|s| s.name != name);
        self.write_config(&servers)?;
        self.disconnect(name)
    }

    pub fn set_server_enabled(&self, name: &str, enabled: bool) -> KResult<()> {
        let _op = self.config_ops.lock_or_recover();
        let mut servers = self.read_config();
        for s in &mut servers {
            if s.name == name {
                s.enabled = enabled;
            }
        }
        self.write_config(&servers)?;
        if !enabled {
            self.disconnect(name)?;
        }
        Ok(())
    }

    pub fn shutdown(&self) -> Result<(), KernelError> {
        let conns: Vec<_> = self.connections.lock_or_recover().drain().collect();
        for (_, conn) in conns {
            let mut guard = conn.lock_or_recover();
            let _ = guard.child.kill();
            let _ = guard.child.wait();
        }
        Ok(())
    }

    pub fn disconnect(&self, name: &str) -> KResult<()> {
        if let Some(conn) = self.connections.lock_or_recover().remove(name) {
            let mut guard = conn.lock_or_recover();
            let _ = guard.child.kill();
            let _ = guard.child.wait();
        }
        Ok(())
    }

    pub fn status(&self) -> Vec<Value> {
        let servers = self.read_config();
        let connections = self.connections.lock_or_recover();
        servers
            .into_iter()
            .map(|s| {
                let connected = connections.contains_key(&s.name);
                json!({
                    "name": s.name,
                    "command": s.command,
                    "args": s.args,
                    "enabled": s.enabled,
                    "connected": connected,
                })
            })
            .collect()
    }

    /// Connect (spawn + initialize + tools/list) one configured server.
    ///
    /// The `connecting` set makes the check-then-spawn atomic: two parallel
    /// connects can never spawn duplicate children or leak a reader thread.
    pub fn connect(&self, name: &str) -> KResult<Value> {
        if self.connections.lock_or_recover().contains_key(name) {
            return Ok(json!({ "name": name, "alreadyConnected": true }));
        }
        {
            let mut connecting = self.connecting.lock_or_recover();
            if !connecting.insert(name.to_string()) {
                return Err(KernelError::Message(format!(
                    "mcp server `{name}` is already connecting"
                )));
            }
        }
        let result = self.connect_inner(name);
        self.connecting.lock_or_recover().remove(name);
        result
    }

    fn connect_inner(&self, name: &str) -> KResult<Value> {
        let config = self
            .read_config()
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| KernelError::Message(format!("unknown mcp server `{name}`")))?;
        if !config.enabled {
            return Err(KernelError::Message(format!(
                "mcp server `{name}` is disabled"
            )));
        }

        let mut child = Command::new(&config.command)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                KernelError::Message(format!("failed to start `{}`: {e}", config.command))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| KernelError::Message("mcp server stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| KernelError::Message("mcp server stdout unavailable".into()))?;

        let pending: Arc<Mutex<HashMap<u64, std::sync::mpsc::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicU64::new(1));
        let conn = Arc::new(Mutex::new(McpConnection {
            child,
            stdin,
            pending: pending.clone(),
            next_id: next_id.clone(),
            server_info: Value::Null,
            tools: vec![],
        }));

        // Reader thread: dispatch responses to waiters by id.
        let reader_conn = conn.clone();
        std::thread::Builder::new()
            .name(format!("ed-mcp-{name}"))
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                        let tx = reader_conn
                            .lock_or_recover()
                            .pending
                            .lock_or_recover()
                            .remove(&id);
                        if let Some(tx) = tx {
                            let _ = tx.send(msg);
                        }
                    }
                }
                // The server closed stdout: fail all pending waiters so no
                // caller hangs until the 120s timeout.
                let leftover: Vec<_> = reader_conn
                    .lock_or_recover()
                    .pending
                    .lock_or_recover()
                    .drain()
                    .collect();
                for (_, tx) in leftover {
                    let _ = tx.send(json!({"error": {"message": "mcp server closed the connection"}}));
                }
            })
            .map_err(|e| KernelError::Message(format!("cannot spawn reader: {e}")))?;

        // Handshake: initialize.
        let init_result = call(
            &conn,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "workbench-zero", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        {
            let mut guard = conn.lock_or_recover();
            guard.server_info = init_result
                .get("serverInfo")
                .cloned()
                .unwrap_or(Value::Null);
        }
        let _ = notify(&conn, "notifications/initialized", json!({}));
        let tools_result = call(&conn, "tools/list", json!({}))?;
        let tools = tools_result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        {
            let mut guard = conn.lock_or_recover();
            guard.tools = tools.clone();
        }
        self.connections
            .lock_or_recover()
            .insert(name.to_string(), conn.clone());
        Ok(json!({
            "name": name,
            "serverInfo": {
                "name": tools_result.pointer("/serverInfo/name").cloned().unwrap_or(Value::Null),
            },
            "toolCount": tools.len(),
        }))
    }

    pub fn list_tools(&self) -> Vec<Value> {
        let connections = self.connections.lock().unwrap();
        let mut out = vec![];
        for (server, conn) in connections.iter() {
            let guard = conn.lock_or_recover();
            for tool in &guard.tools {
                out.push(json!({
                    "server": server,
                    "name": tool.get("name").cloned().unwrap_or(Value::Null),
                    "description": tool.get("description").cloned().unwrap_or(Value::Null),
                    "inputSchema": tool.get("inputSchema").cloned().unwrap_or(Value::Null),
                }));
            }
        }
        out
    }

    pub fn call_tool(&self, server: &str, tool: &str, arguments: &Value) -> KResult<Value> {
        let conn = self
            .connections
            .lock()
            .unwrap()
            .get(server)
            .cloned()
            .ok_or_else(|| {
                KernelError::Message(format!("mcp server `{server}` is not connected"))
            })?;
        let result = call(
            &conn,
            "tools/call",
            json!({
                "name": tool,
                "arguments": arguments,
            }),
        )?;
        Ok(result)
    }
}

fn notify(conn: &Arc<Mutex<McpConnection>>, method: &str, params: Value) -> KResult<()> {
    send_raw(
        conn,
        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }),
    )
}

fn send_raw(conn: &Arc<Mutex<McpConnection>>, msg: Value) -> KResult<()> {
    let mut guard = conn.lock_or_recover();
    serde_json::to_writer(&mut guard.stdin, &msg)
        .map_err(|e| KernelError::Message(format!("mcp write failed: {e}")))?;
    guard
        .stdin
        .write_all(b"\n")
        .map_err(|e| KernelError::Message(format!("mcp write failed: {e}")))?;
    guard
        .stdin
        .flush()
        .map_err(|e| KernelError::Message(format!("mcp flush failed: {e}")))
}

/// JSON-RPC request with response correlation through the reader thread.
fn call(conn: &Arc<Mutex<McpConnection>>, method: &str, params: Value) -> KResult<Value> {
    let (id, rx) = {
        let mut guard = conn.lock_or_recover();
        let id = guard.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel();
        guard.pending.lock_or_recover().insert(id, tx);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        serde_json::to_writer(&mut guard.stdin, &request)
            .map_err(|e| KernelError::Message(format!("mcp write failed: {e}")))?;
        guard
            .stdin
            .write_all(b"\n")
            .map_err(|e| KernelError::Message(format!("mcp write failed: {e}")))?;
        guard
            .stdin
            .flush()
            .map_err(|e| KernelError::Message(format!("mcp flush failed: {e}")))?;
        (id, rx)
    };
    match rx.recv_timeout(CALL_TIMEOUT) {
        Ok(response) => {
            if let Some(err) = response.get("error") {
                return Err(KernelError::Message(format!(
                    "mcp call `{method}` failed: {err}"
                )));
            }
            Ok(response.get("result").cloned().unwrap_or(Value::Null))
        }
        Err(_) => {
            conn.lock_or_recover().pending.lock_or_recover().remove(&id);
            Err(KernelError::Message(format!(
                "mcp call `{method}` timed out or connection closed (limit {}s)",
                CALL_TIMEOUT.as_secs()
            )))
        }
    }
}

/// Kernel-level check for plugin callers of mcp methods.
pub fn check_permission(kernel: &Kernel, caller: &CallerCtx) -> KResult<()> {
    let Some(plugin_id) = &caller.plugin_id else {
        return Ok(());
    };
    kernel
        .permissions
        .check_flag(plugin_id, "mcp:connect")
        .map_err(|e| KernelError::Permission(e.to_string()))
}
