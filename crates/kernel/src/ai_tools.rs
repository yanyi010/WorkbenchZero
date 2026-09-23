//! AI tool registry and plugin log buffers.
//!
//! Plugins contribute AI-callable tools (spec §53). Tool *execution* is
//! routed by the shell to the owning plugin's logic iframe; the kernel only
//! maintains the registry (metadata + ownership) so the AI plugin can list
//! tools. LLM tool calling never bypasses user permissions: high-risk tools
//! require explicit confirmation in the chat UI.

use std::collections::HashMap;
use std::sync::RwLock;

use serde_json::Value;

use crate::KResult;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTool {
    pub name: String,
    pub plugin_id: String,
    pub description: String,
    pub parameters: Value,
    pub high_risk: bool,
}

pub struct AiToolRegistry {
    tools: RwLock<HashMap<String, AiTool>>,
}

impl Default for AiToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AiToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, plugin_id: &str, value: Value) -> KResult<()> {
        let name = value
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::KernelError::Message("ai tool requires a name".into()))?
            .to_string();
        let tool = AiTool {
            description: value
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            parameters: value.get("parameters").cloned().unwrap_or(Value::Null),
            high_risk: value
                .get("highRisk")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            plugin_id: plugin_id.to_string(),
            name,
        };
        self.tools.write().unwrap().insert(tool.name.clone(), tool);
        Ok(())
    }

    /// Register a manifest-declared tool for a plugin (idempotent refresh).
    pub fn register_manifest_tools(
        &self,
        plugin_id: &str,
        manifest_tools: &[eigendesk_plugin_runtime::AiToolContribution],
    ) {
        self.unregister_by_plugin(plugin_id);
        for t in manifest_tools {
            let _ = self.register(
                plugin_id,
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                    "highRisk": t.high_risk,
                }),
            );
        }
    }

    pub fn unregister_by_plugin(&self, plugin_id: &str) {
        self.tools
            .write()
            .unwrap()
            .retain(|_, t| t.plugin_id != plugin_id);
    }

    pub fn list(&self) -> Vec<AiTool> {
        let mut tools: Vec<AiTool> = self.tools.read().unwrap().values().cloned().collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    pub fn tool(&self, name: &str) -> Option<AiTool> {
        self.tools.read().unwrap().get(name).cloned()
    }
}

/// Pending AI tool invocations. The kernel routes a call to the owning
/// plugin's logic iframe via a `plugin-push` and parks the RPC thread until
/// the plugin answers with `ai.toolResult` (or the timeout fires).
pub struct PendingToolCalls {
    next_id: std::sync::atomic::AtomicU64,
    pending: RwLock<HashMap<u64, (String, std::sync::mpsc::Sender<Value>)>>,
}

impl Default for PendingToolCalls {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingToolCalls {
    pub fn new() -> Self {
        Self {
            next_id: std::sync::atomic::AtomicU64::new(1),
            pending: RwLock::new(HashMap::new()),
        }
    }

    /// Park a tool call; returns the request id and the receiving end the
    /// RPC thread will block on.
    pub fn begin(&self, owner: &str) -> (u64, std::sync::mpsc::Receiver<Value>) {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending
            .write()
            .unwrap()
            .insert(id, (owner.to_string(), tx));
        (id, rx)
    }

    /// Resolve a pending call; only the recorded owner may answer.
    pub fn complete(&self, request_id: u64, owner: &str, result: Value) -> bool {
        let entry = self.pending.write().unwrap().remove(&request_id);
        match entry {
            Some((expected_owner, tx)) if expected_owner == owner => tx.send(result).is_ok(),
            _ => false,
        }
    }
}

/// Ring buffer of plugin log lines (cap 500 per plugin).
pub struct PluginLogs {
    logs: RwLock<HashMap<String, Vec<Value>>>,
}

impl Default for PluginLogs {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginLogs {
    pub fn new() -> Self {
        Self {
            logs: RwLock::new(HashMap::new()),
        }
    }

    pub fn push(&self, plugin_id: &str, level: &str, message: &str) {
        let mut logs = self.logs.write().unwrap();
        let entry = logs.entry(plugin_id.to_string()).or_default();
        entry.push(serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "level": level,
            "message": message,
        }));
        let len = entry.len();
        if len > 500 {
            entry.drain(..len - 500);
        }
    }

    pub fn get(&self, plugin_id: &str) -> Vec<Value> {
        self.logs
            .read()
            .unwrap()
            .get(plugin_id)
            .cloned()
            .unwrap_or_default()
    }
}
