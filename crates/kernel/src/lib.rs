//! Workbench Zero kernel: the Rust core that coordinates capabilities (spec §4-5).
//!
//! The kernel is UI-framework agnostic. The Tauri shell feeds it JSON-RPC
//! requests and installs a push sink that receives kernel→UI messages.
//! Capability breadth grows through plugins, never through core growth.

pub mod ai_tools;
pub mod core_commands;
pub mod db;
pub mod diagnostics;
pub mod fs;
pub mod mcp;
pub mod net;
pub mod notify;
pub mod pty;
pub mod push;
pub mod rpc;
pub mod session;
pub mod watcher;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use once_cell::sync::Lazy;
use wz_commands::CommandRegistry;
use wz_events::EventBus;
use wz_permissions::Evaluator;
use wz_plugin_runtime::PluginManager;
use wz_settings::{Scope, SettingDescriptor, SettingType, SettingsService};
use wz_storage::{Dirs, PluginStateStore};
use wz_workspace::{Workspace, WorkspaceManager};

/// Global AI tool registry (contributed by plugins, manifest + dynamic).
pub static AI_TOOLS: Lazy<ai_tools::AiToolRegistry> = Lazy::new(ai_tools::AiToolRegistry::new);

/// Per-plugin structured log ring buffers (surfaced in the plugin store UI).
pub static PLUGIN_LOGS: Lazy<ai_tools::PluginLogs> = Lazy::new(ai_tools::PluginLogs::new);
/// Pending AI tool invocations routed to owning plugins.
pub static PENDING_TOOL_CALLS: Lazy<ai_tools::PendingToolCalls> =
    Lazy::new(ai_tools::PendingToolCalls::new);

/// Push messages flow kernel -> shell main frame in batches.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PushMessage {
    /// Topic: `event`, `plugin-state`, `notification`, `plugin-push`,
    /// `shortcut`, `mcp-status`.
    pub topic: String,
    /// Target plugin id for `plugin-push` messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub payload: serde_json::Value,
}

pub type PushSink = Arc<dyn Fn(&[PushMessage]) + Send + Sync + 'static>;

pub struct KernelConfig {
    /// App version reported to diagnostics.
    pub app_version: String,
    /// Bundled first-party plugin packages (Tauri resource dir).
    pub bundled_dir: Option<PathBuf>,
    /// Override the XDG data dir (tests / MCP server).
    pub data_override: Option<PathBuf>,
    /// `--safe-mode`: disable third-party plugins for this session (spec §86).
    pub safe_mode: bool,
}

/// Who is calling. `plugin_id: None` means the trusted application shell
/// (the only holder of the RPC token).
#[derive(Debug, Clone)]
pub struct CallerCtx {
    pub plugin_id: Option<String>,
}

pub struct WorkspaceState {
    pub workspace: Workspace,
    pub conn: Mutex<rusqlite::Connection>,
    pub plugin_stores: Mutex<std::collections::HashMap<String, Arc<PluginStateStore>>>,
}

pub struct Kernel {
    pub dirs: Dirs,
    pub settings: SettingsService,
    pub workspaces: Mutex<WorkspaceManager>,
    pub plugins: PluginManager,
    pub permissions: Evaluator,
    pub commands: CommandRegistry,
    pub events: EventBus,
    pub secrets: wz_secrets::SecretsService,
    pub diagnostics: diagnostics::Diagnostics,
    pub pty: pty::PtyManager,
    pub net: net::NetService,
    pub mcp: mcp::McpManager,
    pub notifications: Mutex<Vec<notify::NotificationRecord>>,
    pub session: session::SessionKv,
    pub push: push::PushHub,
    pub workspace_state: RwLock<Option<Arc<WorkspaceState>>>,
    pub runtime: tokio::runtime::Runtime,
    pub safe_mode: bool,
    pub app_version: String,
    token: RwLock<Option<String>>,
    shutting_down: AtomicBool,
}

const RPC_MAX_PARAMS_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RpcRequest {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RpcResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, code: &str, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(RpcError {
                code: code.to_string(),
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Errors carried through RPC. Messages follow the error philosophy in
/// spec §83: what failed, which plugin, what context, how to recover.
#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("{0}")]
    Message(String),
    #[error("permission denied: {0}")]
    Permission(String),
    #[error("no workspace is open")]
    NoWorkspace,
    #[error("unknown method `{0}`")]
    UnknownMethod(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("message too large (limit {0} bytes)")]
    TooLarge(usize),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl KernelError {
    pub fn code(&self) -> &'static str {
        match self {
            KernelError::Message(_) => "kernel/error",
            KernelError::Permission(_) => "kernel/permission-denied",
            KernelError::NoWorkspace => "kernel/no-workspace",
            KernelError::UnknownMethod(_) => "kernel/unknown-method",
            KernelError::Unauthorized(_) => "kernel/unauthorized",
            KernelError::TooLarge(_) => "kernel/too-large",
            KernelError::Io(_) | KernelError::Serde(_) => "kernel/io",
        }
    }
}

pub type KResult<T> = Result<T, KernelError>;

impl Kernel {
    /// Bootstrap the kernel (spec §80 startup discipline: shell -> workspace
    /// metadata -> plugin manifests -> ready; nothing heavy is activated).
    pub fn bootstrap(config: KernelConfig, sink: PushSink) -> anyhow::Result<Arc<Kernel>> {
        let mut startup = diagnostics::StartupTracker::new();
        let dirs = Dirs::init(config.bundled_dir.clone(), config.data_override.clone())?;
        init_logging(&dirs)?;
        startup.mark("logging");

        let settings = SettingsService::new(dirs.config.join("settings.json"))?;
        Self::register_core_settings(&settings);
        startup.mark("settings");

        let workspaces = WorkspaceManager::new(dirs.config.join("workspaces.json"))?;
        startup.mark("workspace-registry");

        let plugins = PluginManager::new(
            dirs.data.join("plugins.json"),
            dirs.bundled.clone(),
            dirs.plugins.clone(),
            dirs.dev_plugins.clone(),
            dirs.registry.clone(),
        );
        startup.mark("plugin-manager");

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;

        let kernel = Arc::new(Kernel {
            settings,
            workspaces: Mutex::new(workspaces),
            plugins,
            permissions: Evaluator::new(),
            commands: CommandRegistry::new(),
            events: EventBus::new(),
            secrets: wz_secrets::SecretsService::new(dirs.data.clone()),
            diagnostics: diagnostics::Diagnostics::new(config.app_version.clone()),
            pty: pty::PtyManager::new(),
            net: net::NetService::new(),
            mcp: mcp::McpManager::new(),
            notifications: Mutex::new(Vec::new()),
            session: session::SessionKv::new(),
            push: push::PushHub::new(sink),
            workspace_state: RwLock::new(None),
            runtime,
            safe_mode: config.safe_mode,
            app_version: config.app_version,
            token: RwLock::new(None),
            shutting_down: AtomicBool::new(false),
            dirs,
        });

        // Grant installed plugins their persisted permissions and sync
        // contribution registries.
        kernel.sync_registries();
        startup.mark("permissions");

        kernel.plugins.discover()?;
        if kernel.safe_mode {
            kernel.apply_safe_mode();
        }
        kernel.sync_registries();
        kernel
            .mcp
            .set_config_path(kernel.dirs.config.join("mcp-servers.json"));
        startup.mark("plugin-discovery");

        core_commands::register_core_commands(&kernel.commands);
        startup.mark("core-commands");

        // Forward event bus traffic to the UI.
        kernel.spawn_event_forwarder();

        // Watch plugin directories for changes (hot reload / installs).
        watcher::start(kernel.clone());

        let startup_ms = startup.finish();
        kernel
            .diagnostics
            .record_startup(startup_ms.0, startup_ms.1);
        tracing::info!(
            version = %kernel.app_version,
            safe_mode = kernel.safe_mode,
            startup_ms = startup_ms.1,
            "Workbench Zero kernel ready"
        );
        Ok(kernel)
    }

    pub fn shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.pty.kill_all();
        if let Err(e) = self.mcp.shutdown() {
            tracing::warn!(error = %e, "mcp shutdown error");
        }
        self.push.flush();
        tracing::info!("kernel shutdown complete");
    }

    fn register_core_settings(settings: &SettingsService) {
        let descriptors = vec![
            SettingDescriptor {
                key: "core.appearance.theme".into(),
                r#type: SettingType::Enum,
                title: "Theme".into(),
                description: Some("Light, dark, or follow the system.".into()),
                default: Some(serde_json::json!("system")),
                enum_values: vec!["system".into(), "light".into(), "dark".into()],
                scope: Scope::Global,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.startup.openLastWorkspace".into(),
                r#type: SettingType::Boolean,
                title: "Reopen last workspace on launch".into(),
                description: None,
                default: Some(serde_json::json!(true)),
                enum_values: vec![],
                scope: Scope::Global,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.behavior.restoreLayout".into(),
                r#type: SettingType::Boolean,
                title: "Restore layout per workspace".into(),
                description: Some("Tabs, sidebar, bottom panel and dashboard widgets are saved per workspace.".into()),
                default: Some(serde_json::json!(true)),
                enum_values: vec![],
                scope: Scope::Workspace,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.capture.globalShortcut".into(),
                r#type: SettingType::Boolean,
                title: "Global Quick Capture shortcut (Alt+Space)".into(),
                description: Some("Register Alt+Space system-wide so Quick Capture opens even when Workbench Zero is not focused.".into()),
                default: Some(serde_json::json!(false)),
                enum_values: vec![],
                scope: Scope::Global,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.terminal.shell".into(),
                r#type: SettingType::String,
                title: "Terminal shell".into(),
                description: Some("Leave empty to use $SHELL.".into()),
                default: Some(serde_json::json!("")),
                enum_values: vec![],
                scope: Scope::Global,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.plugin.autoDisableAfterFailures".into(),
                r#type: SettingType::Number,
                title: "Auto-disable plugins after N failures".into(),
                description: Some("Repeated plugin crashes are isolated (spec §37).".into()),
                default: Some(serde_json::json!(5)),
                enum_values: vec![],
                scope: Scope::Global,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.plugin.autoUpdate".into(),
                r#type: SettingType::Boolean,
                title: "Plugin auto-update".into(),
                description: Some("Check the local registry for newer plugin versions and offer updates.".into()),
                default: Some(serde_json::json!(true)),
                enum_values: vec![],
                scope: Scope::Global,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.diagnostics.logLevel".into(),
                r#type: SettingType::Enum,
                title: "Log level".into(),
                description: None,
                default: Some(serde_json::json!("info")),
                enum_values: vec!["error".into(), "warn".into(), "info".into(), "debug".into()],
                scope: Scope::Global,
                plugin_id: None,
            },
            SettingDescriptor {
                key: "core.search.liveProviders".into(),
                r#type: SettingType::Boolean,
                title: "Query live search providers".into(),
                description: Some("Include plugin-provided search providers in Universal Search.".into()),
                default: Some(serde_json::json!(true)),
                enum_values: vec![],
                scope: Scope::Global,
                plugin_id: None,
            },
        ];
        settings.register_descriptors(descriptors);
    }

    /// (Re)apply persisted permission grants of installed plugins.
    pub fn sync_permission_grants(&self) {
        for info in self.plugins.list() {
            if let Some(rec) = self.plugins.get(&info.manifest.id) {
                if rec.is_installed() && rec.state != wz_plugin_runtime::PluginState::Uninstalled {
                    self.permissions
                        .set_grants(&info.manifest.id, rec.granted.clone());
                } else {
                    self.permissions.remove_grants(&info.manifest.id);
                }
            }
        }
    }

    /// Synchronize kernel-side registries (commands, settings descriptors,
    /// AI tools, permission grants) with current plugin states. Called after
    /// any install/enable/disable/uninstall/approval.
    pub fn sync_registries(&self) {
        self.sync_permission_grants();
        let infos = self.plugins.list();
        let enabled: Vec<&wz_plugin_runtime::PluginInfo> = infos
            .iter()
            .filter(|i| {
                i.state == wz_plugin_runtime::PluginState::Enabled
                    || i.state == wz_plugin_runtime::PluginState::Active
            })
            .collect();
        // Commands from enabled plugins.
        for info in &infos {
            let _ = self.commands.unregister_by_plugin(&info.manifest.id);
        }
        for info in &enabled {
            for cmd in &info.manifest.contributes.commands {
                self.commands.upsert(wz_commands::CommandDef {
                    id: cmd.id.clone(),
                    title: cmd.title.clone(),
                    category: cmd.category.clone(),
                    keywords: cmd.keywords.clone(),
                    when: cmd.when.clone(),
                    plugin_id: Some(info.manifest.id.clone()),
                    default_keybinding: cmd.keybinding.clone(),
                    takes_args: cmd.takes_args,
                    hidden: cmd.hidden,
                    opens_view: cmd.opens_view.clone(),
                });
            }
        }
        // Settings descriptors from enabled plugins.
        for info in &infos {
            self.settings.unregister_by_plugin(&info.manifest.id);
        }
        for info in &enabled {
            self.settings
                .register_descriptors(info.manifest.setting_descriptors());
        }
        // AI tools from enabled plugins.
        for info in &infos {
            AI_TOOLS.unregister_by_plugin(&info.manifest.id);
        }
        for info in &enabled {
            AI_TOOLS
                .register_manifest_tools(&info.manifest.id, &info.manifest.contributes.ai_tools);
        }
    }

    fn apply_safe_mode(&self) {
        // Safe mode disables third-party plugins for the session (spec §85-86).
        for info in self.plugins.list() {
            if info.trusted {
                continue;
            }
            if info.state == wz_plugin_runtime::PluginState::Enabled
                || info.state == wz_plugin_runtime::PluginState::Active
            {
                let _ = self
                    .plugins
                    .set_state(&info.manifest.id, wz_plugin_runtime::PluginState::Disabled);
            }
        }
        tracing::warn!("safe mode active: third-party plugins disabled for this session");
    }

    fn spawn_event_forwarder(self: &Arc<Self>) {
        let kernel = self.clone();
        let mut sub = self.events.subscribe(None);
        self.runtime.spawn(async move {
            while let Some(event) = sub.receiver.recv().await {
                kernel.push.push(
                    "event",
                    None,
                    serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                );
            }
        });
    }

    // -- token & rpc ---------------------------------------------------------

    /// Issue the RPC token to the first caller (the application shell, which
    /// calls this during bootstrap before any plugin iframe exists).
    pub fn issue_token(&self) -> KResult<String> {
        let mut guard = self.token.write().unwrap();
        if let Some(existing) = guard.as_ref() {
            return Err(KernelError::Unauthorized(format!(
                "token already issued ({existing})"
            )));
        }
        let token = uuid::Uuid::new_v4().to_string();
        *guard = Some(token.clone());
        Ok(token)
    }

    pub fn check_token(&self, token: &str) -> KResult<()> {
        let guard = self.token.read().unwrap();
        match guard.as_deref() {
            Some(t) if t == token => Ok(()),
            Some(_) => Err(KernelError::Unauthorized("invalid token".into())),
            None => Err(KernelError::Unauthorized(
                "token not issued yet; call app.issueToken first".into(),
            )),
        }
    }

    pub fn rpc(self: &Arc<Self>, token: &str, req: RpcRequest) -> RpcResponse {
        let start = std::time::Instant::now();
        let resp = match self.rpc_inner(token, req.clone()) {
            Ok(result) => RpcResponse::ok(req.id, result),
            Err(e) => {
                let mut r = RpcResponse::err(req.id, e.code(), e.to_string());
                if let KernelError::Permission(msg) = &e {
                    if let Some(code) = r.error.as_mut() {
                        code.data = Some(serde_json::json!({ "permission": msg }));
                    }
                }
                r
            }
        };
        self.diagnostics
            .record_rpc(req.method.as_str(), start.elapsed().as_secs_f64() * 1000.0);
        resp
    }

    fn rpc_inner(self: &Arc<Self>, token: &str, req: RpcRequest) -> KResult<serde_json::Value> {
        if req.method != "app.issueToken" {
            self.check_token(token)?;
        }
        if req.params.to_string().len() > RPC_MAX_PARAMS_BYTES {
            return Err(KernelError::TooLarge(RPC_MAX_PARAMS_BYTES));
        }
        let caller = CallerCtx {
            plugin_id: req
                .params
                .get("__plugin")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        };
        rpc::dispatch(self, &req.method, req.params, caller)
    }

    // -- workspace helpers ----------------------------------------------------

    pub fn current_workspace(&self) -> Option<Arc<WorkspaceState>> {
        self.workspace_state.read().unwrap().clone()
    }

    pub fn require_workspace(&self) -> KResult<Arc<WorkspaceState>> {
        self.current_workspace().ok_or(KernelError::NoWorkspace)
    }

    /// Open (or switch) the workspace. Resets session-scoped state.
    pub fn open_workspace(&self, id: &str) -> KResult<Arc<WorkspaceState>> {
        // Tear down session-scoped resources (spec §9: terminal process is
        // session scoped).
        self.pty.kill_all();
        self.session.clear();
        self.net.abort_all();

        let ws = {
            let mut mgr = self.workspaces.lock().unwrap();
            mgr.open(id)
                .map_err(|e| KernelError::Message(e.to_string()))?
        };
        let conn = rusqlite::Connection::open(&ws.sqlite_path)
            .map_err(|e| KernelError::Message(format!("cannot open index.sqlite: {e}")))?;
        wz_workspace::apply_sqlite_migrations(&conn)
            .map_err(|e| KernelError::Message(format!("migration failed: {e}")))?;
        wz_artifacts::ArtifactRegistry::init(&conn)
            .map_err(|e| KernelError::Message(e.to_string()))?;
        wz_search::SearchIndex::init(&conn).map_err(|e| KernelError::Message(e.to_string()))?;
        let state = Arc::new(WorkspaceState {
            workspace: ws,
            conn: Mutex::new(conn),
            plugin_stores: Mutex::new(std::collections::HashMap::new()),
        });

        self.settings
            .set_workspace(Some(state.workspace.settings_path.clone()))
            .map_err(|e| KernelError::Message(e.to_string()))?;

        *self.workspace_state.write().unwrap() = Some(state.clone());
        self.events.emit(
            "workspace.opened",
            None,
            serde_json::json!({ "id": state.workspace.record.id, "name": state.workspace.record.name }),
        );
        tracing::info!(workspace = %state.workspace.record.name, "workspace opened");
        Ok(state)
    }

    pub fn close_workspace(&self) -> KResult<()> {
        self.pty.kill_all();
        self.session.clear();
        self.net.abort_all();
        self.settings
            .set_workspace(None)
            .map_err(|e| KernelError::Message(e.to_string()))?;
        *self.workspace_state.write().unwrap() = None;
        self.events
            .emit("workspace.closed", None, serde_json::json!({}));
        Ok(())
    }

    pub fn plugin_store(&self, plugin_id: &str) -> KResult<Arc<PluginStateStore>> {
        let ws = self.require_workspace()?;
        let mut stores = ws.plugin_stores.lock().unwrap();
        if let Some(existing) = stores.get(plugin_id) {
            return Ok(existing.clone());
        }
        let store = PluginStateStore::open(&ws.workspace.state_root, plugin_id)
            .map_err(|e| KernelError::Message(e.to_string()))?;
        let store = Arc::new(store);
        stores.insert(plugin_id.to_string(), store.clone());
        Ok(store)
    }

    /// Resolve a filesystem path (relative paths anchor at the workspace root).
    pub fn resolve_path(&self, path: &str) -> KResult<PathBuf> {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            Ok(p)
        } else {
            let ws = self.require_workspace()?;
            Ok(ws.workspace.root().join(p))
        }
    }
}

static LOGGING_INIT: AtomicBool = AtomicBool::new(false);

fn init_logging(dirs: &Dirs) -> anyhow::Result<()> {
    if LOGGING_INIT.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let appender = tracing_appender::rolling::daily(dirs.logs.join("workbench-zero"), "app.log");
    let level = std::env::var("WORKBENCH_ZERO_LOG").unwrap_or_else(|_| "info".to_string());
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&level))
        .with_writer(appender)
        .with_ansi(false)
        .json()
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}
