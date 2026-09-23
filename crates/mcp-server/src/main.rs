//! `wz-mcp`: a restricted MCP server exposing user-approved
//! Workbench Zero workspace capabilities to external agents over stdio
//! (spec §55-57).
//!
//! Security model:
//!   - The server is read-only: it lists and reads, it never writes.
//!   - Nothing is exposed by default. Every tool must be explicitly allowed
//!     via `--allow tool1,tool2` or the config file
//!     `~/.config/workbench-zero/mcp-server.json` (`{"allow": [...]}`).
//!   - The server is a separate process with no connection to the running
//!     app: it opens the workspace directory directly, so it cannot touch
//!     app state, secrets, or terminals.
//!
//! JSON-RPC 2.0 over stdio, MCP protocol version 2024-11-05.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";

const TOOL_MEMO_LIST: &str = "memo.list";
const TOOL_MEMO_READ: &str = "memo.read";
const TOOL_MEMO_SEARCH: &str = "memo.search";
const TOOL_TASKS_TODAY: &str = "tasks.today";
const TOOL_TASKS_LIST: &str = "tasks.list";
const TOOL_WORKSPACE_FILES: &str = "workspace.files";

struct Server {
    workspace: PathBuf,
    allow: Vec<String>,
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": TOOL_MEMO_LIST,
            "description": "List memo files in the workspace Memo folder. Returns titles and URIs.",
            "inputSchema": { "type": "object", "properties": { "limit": { "type": "number", "description": "Maximum memos to return (default 50)" } } }
        }),
        json!({
            "name": TOOL_MEMO_READ,
            "description": "Read one memo by its file name.",
            "inputSchema": { "type": "object", "properties": { "name": { "type": "string", "description": "Memo file name from memo.list" } }, "required": ["name"] }
        }),
        json!({
            "name": TOOL_MEMO_SEARCH,
            "description": "Full-text search over memo file names and contents (simple substring match).",
            "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] }
        }),
        json!({
            "name": TOOL_TASKS_TODAY,
            "description": "List tasks that are due today or overdue and not completed.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": TOOL_TASKS_LIST,
            "description": "List all tasks with status, due date, priority and project.",
            "inputSchema": { "type": "object", "properties": { "status": { "type": "string", "enum": ["todo", "doing", "done"] } } }
        }),
        json!({
            "name": TOOL_WORKSPACE_FILES,
            "description": "List files in the workspace (relative paths), optionally under a subfolder.",
            "inputSchema": { "type": "object", "properties": { "subdir": { "type": "string", "description": "Relative subfolder (default: workspace root)" }, "limit": { "type": "number" } } }
        }),
    ]
}

impl Server {
    fn memo_dir(&self) -> PathBuf {
        self.workspace.join("Memos")
    }

    fn task_file(&self) -> PathBuf {
        self.workspace.join("Tasks").join("tasks.json")
    }

    fn call(&self, tool: &str, args: &Value) -> Result<Value, String> {
        if !self.allow.iter().any(|a| a == tool || a == "*") {
            return Err(format!(
                "tool `{tool}` is not on this server's allow-list. Allowed: {}",
                self.allow.join(", ")
            ));
        }
        match tool {
            TOOL_MEMO_LIST => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
                let mut memos = vec![];
                for entry in walkdir::WalkDir::new(self.memo_dir())
                    .max_depth(3)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_type().is_file()
                        && entry.path().extension().map(|e| e == "md").unwrap_or(false)
                    {
                        let rel = entry
                            .path()
                            .strip_prefix(&self.workspace)
                            .unwrap_or(entry.path())
                            .display()
                            .to_string();
                        memos.push(json!({
                            "name": entry.file_name().to_string_lossy(),
                            "path": rel,
                            "title": frontmatter_title(&std::fs::read_to_string(entry.path()).unwrap_or_default())
                                .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string()),
                        }));
                        if memos.len() >= limit {
                            break;
                        }
                    }
                }
                Ok(json!({ "memos": memos }))
            }
            TOOL_MEMO_READ => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or("`name` is required")?;
                // Contain path traversal: only a bare file name is accepted.
                if name.contains('/') || name.contains('\\') || name.contains("..") {
                    return Err("`name` must be a bare file name".into());
                }
                let path = self.memo_dir().join(name);
                if !path.starts_with(self.memo_dir()) || !path.is_file() {
                    return Err(format!("memo `{name}` not found"));
                }
                let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                Ok(json!({ "name": name, "content": content }))
            }
            TOOL_MEMO_SEARCH => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or("`query` is required")?
                    .to_lowercase();
                let mut hits = vec![];
                for entry in walkdir::WalkDir::new(self.memo_dir())
                    .max_depth(3)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
                    if content.to_lowercase().contains(&query)
                        || entry
                            .file_name()
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&query)
                    {
                        hits.push(json!({
                            "name": entry.file_name().to_string_lossy(),
                            "path": entry.path().strip_prefix(&self.workspace).unwrap_or(entry.path()).display().to_string(),
                        }));
                        if hits.len() >= 50 {
                            break;
                        }
                    }
                }
                Ok(json!({ "hits": hits }))
            }
            TOOL_TASKS_TODAY => {
                let tasks = self.load_tasks()?;
                let today = chrono::Local::now().date_naive();
                let due: Vec<&Value> = tasks
                    .iter()
                    .filter(|t| {
                        let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("todo");
                        if status == "done" {
                            return false;
                        }
                        t.get("due")
                            .and_then(|v| v.as_str())
                            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                            .map(|d| d <= today)
                            .unwrap_or(false)
                    })
                    .collect();
                Ok(json!({ "tasks": due }))
            }
            TOOL_TASKS_LIST => {
                let mut tasks = self.load_tasks()?;
                if let Some(status) = args.get("status").and_then(|v| v.as_str()) {
                    tasks.retain(|t| {
                        t.get("status").and_then(|v| v.as_str()).unwrap_or("todo") == status
                    });
                }
                Ok(json!({ "tasks": tasks }))
            }
            TOOL_WORKSPACE_FILES => {
                let subdir = args.get("subdir").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
                let base = if subdir.is_empty() {
                    self.workspace.clone()
                } else {
                    // Contain traversal *before* any filesystem access:
                    // absolute subdirs and `..` segments must not escape.
                    if PathBuf::from(subdir).is_absolute() {
                        return Err("subdir must be relative".into());
                    }
                    let resolved = wz_permissions::resolve(&self.workspace.join(subdir))
                        .map_err(|e| format!("invalid subdir: {e}"))?;
                    if !resolved.starts_with(&self.workspace) {
                        return Err("subdir escapes the workspace".into());
                    }
                    resolved
                };
                let mut files = vec![];
                for entry in walkdir::WalkDir::new(&base)
                    .max_depth(4)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.depth() == 0 {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    files.push(json!({
                        "path": entry.path().strip_prefix(&self.workspace).unwrap_or(entry.path()).display().to_string(),
                        "isDir": entry.file_type().is_dir(),
                    }));
                    if files.len() >= limit {
                        break;
                    }
                }
                Ok(json!({ "files": files }))
            }
            other => Err(format!("unknown tool `{other}`")),
        }
    }

    fn load_tasks(&self) -> Result<Vec<Value>, String> {
        let path = self.task_file();
        if !path.exists() {
            return Ok(vec![]);
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        Ok(parsed
            .get("tasks")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default())
    }
}

fn frontmatter_title(content: &str) -> Option<String> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    for line in rest[..end].lines() {
        if let Some(title) = line.strip_prefix("title:") {
            return Some(title.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn main() {
    let mut workspace: Option<PathBuf> = None;
    let mut allow: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => {
                if let Some(ws) = args.next() {
                    workspace = Some(PathBuf::from(ws));
                }
            }
            "--allow" => {
                if let Some(list) = args.next() {
                    allow = list
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--help" => {
                eprintln!("wz-mcp — restricted MCP server for Workbench Zero workspaces");
                eprintln!();
                eprintln!("Usage: wz-mcp --workspace <dir> [--allow tool1,tool2,...]");
                eprintln!();
                eprintln!(
                    "Tools: {}",
                    tool_definitions()
                        .iter()
                        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                eprintln!();
                eprintln!(
                    "Alternatively configure allows in ~/.config/workbench-zero/mcp-server.json:"
                );
                eprintln!("  {{\"allow\": [\"memo.list\", \"tasks.today\"]}}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument `{other}`; see --help");
                std::process::exit(2);
            }
        }
    }

    let workspace = match workspace {
        Some(ws) if ws.is_dir() => ws,
        _ => {
            eprintln!("error: --workspace <dir> is required and must exist");
            std::process::exit(2);
        }
    };

    // Config file supplements CLI flags (union).
    let config_path = dirs_config_path();
    if let Ok(raw) = std::fs::read_to_string(&config_path) {
        if let Ok(cfg) = serde_json::from_str::<Value>(&raw) {
            if let Some(list) = cfg.get("allow").and_then(|a| a.as_array()) {
                for item in list {
                    if let Some(s) = item.as_str() {
                        if !allow.iter().any(|a| a == s) {
                            allow.push(s.to_string());
                        }
                    }
                }
            }
        }
    }

    let server = Server {
        workspace: workspace.canonicalize().unwrap_or(workspace),
        allow,
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = msg
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default();
        let id = msg.get("id").cloned();
        let is_request = id.is_some();
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "wz-mcp", "version": env!("CARGO_PKG_VERSION") }
            })),
            "notifications/initialized" => {
                // Notification: no response.
                continue;
            }
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => {
                let name = msg
                    .pointer("/params/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let arguments = msg
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match server.call(name, &arguments) {
                    Ok(content) => Ok(json!({
                        "content": [ { "type": "text", "text": serde_json::to_string_pretty(&content).unwrap_or_default() } ],
                        "isError": false,
                    })),
                    Err(e) => Ok(json!({
                        "content": [ { "type": "text", "text": e } ],
                        "isError": true,
                    })),
                }
            }
            "ping" => Ok(json!({})),
            other => Err(format!("unknown method `{other}`")),
        };
        if !is_request {
            continue;
        }
        let response = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id.unwrap(), "result": result }),
            Err(e) => {
                json!({ "jsonrpc": "2.0", "id": id.unwrap(), "error": { "code": -32601, "message": e } })
            }
        };
        let _ = serde_json::to_writer(&mut out, &response);
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(sub: &str) -> Server {
        let base = std::env::temp_dir().join(format!(
            "wz-mcp-test-{}-{}",
            std::process::id(),
            sub
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::write(base.join("docs").join("a.md"), "hello").unwrap();
        Server {
            workspace: base.canonicalize().unwrap(),
            allow: vec!["*".into()],
        }
    }

    #[test]
    fn workspace_files_containment() {
        let s = server("containment");
        // Normal case works.
        let ok = s
            .call(TOOL_WORKSPACE_FILES, &json!({ "subdir": "docs" }))
            .unwrap();
        assert!(ok["files"].as_array().unwrap().iter().any(|f| f["path"] == "docs/a.md"));
        // `..` and absolute paths must be refused.
        for subdir in ["..", "../..", "./..", "/etc", "docs/../../.."] {
            let r = s.call(TOOL_WORKSPACE_FILES, &json!({ "subdir": subdir }));
            assert!(r.is_err(), "subdir `{subdir}` must be rejected: {r:?}");
        }
    }

    #[test]
    fn memo_read_rejects_traversal() {
        let s = server("memo");
        for name in ["../secret.txt", "..", "a/b", "x\\y"] {
            assert!(s.call(TOOL_MEMO_READ, &json!({ "name": name })).is_err());
        }
    }
}

fn dirs_config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg)
                .join("workbench-zero")
                .join("mcp-server.json");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("workbench-zero")
            .join("mcp-server.json");
    }
    PathBuf::from("wz-mcp.json")
}
