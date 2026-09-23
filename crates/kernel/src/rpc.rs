//! Kernel RPC dispatch. Every method is documented in
//! docs/plugin-api/rpc-methods.md and mirrored in packages/protocol.
//! Plugin-originated calls carry `__plugin` (stamped by the trusted shell
//! bridge); the kernel enforces permissions per method.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::fs as fs_cap;
use crate::{CallerCtx, KResult, Kernel, KernelError};

pub fn dispatch(
    kernel: &Arc<Kernel>,
    method: &str,
    params: Value,
    caller: CallerCtx,
) -> KResult<Value> {
    match method {
        // -- app -------------------------------------------------------------
        "app.issueToken" => Ok(json!(kernel.issue_token()?)),
        "app.bootstrapInfo" => Ok(json!({
            "version": kernel.app_version,
            "safeMode": kernel.safe_mode,
            "platform": std::env::consts::OS,
            "dirs": {
                "config": kernel.dirs.config.display().to_string(),
                "data": kernel.dirs.data.display().to_string(),
                "logs": kernel.dirs.logs.display().to_string(),
                "cache": kernel.dirs.cache.display().to_string(),
            },
            "secrets": kernel.secrets.status(),
        })),
        "app.diagnostics" => Ok(kernel.diagnostics.snapshot()),
        "app.exportDiagnostics" => {
            let snapshot = kernel.diagnostics.snapshot();
            let path = kernel.dirs.data.join(format!(
                "diagnostics-{}.json",
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            ));
            std::fs::write(&path, serde_json::to_string_pretty(&snapshot)?)
                .map_err(|e| KernelError::Message(format!("cannot write diagnostics: {e}")))?;
            Ok(json!({ "path": path.display().to_string() }))
        }
        "app.logFile" => {
            let file = kernel.dirs.logs.join("eigendesk").join(format!(
                "app.log.{}",
                chrono::Local::now().format("%Y-%m-%d")
            ));
            Ok(json!({
                "dir": kernel.dirs.logs.display().to_string(),
                "file": if file.exists() { file.display().to_string() } else { kernel.dirs.logs.display().to_string() },
            }))
        }
        "system.reveal" => {
            let path = str_param(&params, "path")?;
            if caller.plugin_id.is_some() {
                check_flag(kernel, &caller, "system:open")?;
            }
            reveal(&kernel.resolve_path(path)?);
            Ok(Value::Null)
        }
        "system.openUrl" => {
            let url = str_param(&params, "url")?;
            if caller.plugin_id.is_some() {
                check_flag(kernel, &caller, "system:open")?;
            }
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err(KernelError::Message(
                    "only http(s) urls can be opened".into(),
                ));
            }
            open_url(url);
            Ok(Value::Null)
        }

        // -- workspace ---------------------------------------------------------
        "workspace.list" => {
            let mgr = kernel.workspaces.lock().unwrap();
            Ok(json!(mgr.list()))
        }
        "workspace.create" => {
            let name = str_param(&params, "name")?;
            let root = str_param(&params, "root")?;
            let create_root = params["createRoot"].as_bool().unwrap_or(false);
            let ws = {
                let mut mgr = kernel.workspaces.lock().unwrap();
                mgr.create(name, PathBuf::from(root), create_root)
                    .map_err(|e| KernelError::Message(e.to_string()))?
            };
            Ok(workspace_json(&ws))
        }
        "workspace.register" => {
            let root = str_param(&params, "root")?;
            let ws = {
                let mut mgr = kernel.workspaces.lock().unwrap();
                mgr.register_existing(PathBuf::from(root))
                    .map_err(|e| KernelError::Message(e.to_string()))?
            };
            Ok(workspace_json(&ws))
        }
        "workspace.open" => {
            let id = str_param(&params, "id")?;
            let state = kernel.open_workspace(id)?;
            Ok(json!({
                "workspace": workspace_json(&state.workspace),
                "layout": state.workspace.load_layout(),
            }))
        }
        "workspace.current" => match kernel.current_workspace() {
            Some(ws) => Ok(workspace_json(&ws.workspace)),
            None => Ok(Value::Null),
        },
        "workspace.close" => {
            kernel.close_workspace()?;
            Ok(Value::Null)
        }
        "workspace.saveLayout" => {
            let layout = params["layout"].clone();
            if !layout.is_object() {
                return Err(KernelError::Message("layout must be an object".into()));
            }
            let ws = kernel.require_workspace()?;
            ws.workspace
                .save_layout(&layout)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }
        "workspace.remove" => {
            let id = str_param(&params, "id")?;
            let mut mgr = kernel.workspaces.lock().unwrap();
            mgr.remove(id)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }

        // -- settings ------------------------------------------------------------
        "settings.describe" => Ok(json!(kernel.settings.descriptors())),
        "settings.getAll" => Ok(json!(kernel.settings.get_all())),
        "settings.get" => {
            let key = str_param(&params, "key")?;
            Ok(kernel.settings.get(key))
        }
        "settings.set" => {
            let scope = str_param(&params, "scope")?;
            let key = str_param(&params, "key")?;
            let value = params["value"].clone();
            // Plugins may only write their own settings.
            if let Some(plugin_id) = &caller.plugin_id {
                let prefix = format!("{plugin_id}.");
                if !key.starts_with(&prefix) {
                    return Err(KernelError::Permission(format!(
                        "plugin `{plugin_id}` cannot write setting `{key}`"
                    )));
                }
            }
            let scope = parse_scope(scope)?;
            kernel
                .settings
                .set(scope, key, value)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }
        "settings.reset" => {
            let scope = str_param(&params, "scope")?;
            let key = str_param(&params, "key")?;
            let scope = parse_scope(scope)?;
            kernel
                .settings
                .reset(scope, key)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }

        // -- commands ------------------------------------------------------------
        "commands.list" => Ok(json!(kernel.commands.list())),
        "commands.register" => {
            let def_value = params
                .get("command")
                .cloned()
                .ok_or_else(|| KernelError::Message("missing `command` parameter".into()))?;
            let def = parse_command_def(&def_value, caller.plugin_id.clone())?;
            kernel
                .commands
                .register(def)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }
        "commands.unregister" => {
            let id = str_param(&params, "id")?;
            if let Some(plugin_id) = &caller.plugin_id {
                if let Some(def) = kernel.commands.get(id) {
                    if def.plugin_id.as_deref() != Some(plugin_id.as_str()) {
                        return Err(KernelError::Permission(format!(
                            "plugin `{plugin_id}` cannot unregister command `{id}`"
                        )));
                    }
                }
            }
            kernel.commands.unregister(id);
            Ok(Value::Null)
        }

        // -- events ---------------------------------------------------------------
        "events.emit" => {
            let name = str_param(&params, "name")?;
            let data = params["data"].clone();
            kernel.events.emit(name, caller.plugin_id.clone(), data);
            Ok(Value::Null)
        }

        // -- plugins ---------------------------------------------------------------
        "plugins.list" => Ok(json!(kernel.plugins.list())),
        "plugins.get" => {
            let id = str_param(&params, "id")?;
            kernel
                .plugins
                .get(id)
                .map(|rec| eigendesk_plugin_runtime::PluginInfo::from(&rec))
                .map(|i| json!(i))
                .ok_or_else(|| KernelError::Message(format!("plugin `{id}` not found")))
        }
        "plugins.install" => {
            let id = str_param(&params, "id")?;
            // Bundled trusted plugins install with auto-grants; everything
            // else resolves through the catalog and needs approval.
            if let Some(rec) = kernel.plugins.get(id) {
                if rec.trusted
                    && matches!(
                        rec.state,
                        eigendesk_plugin_runtime::PluginState::Discovered
                            | eigendesk_plugin_runtime::PluginState::Uninstalled
                    )
                {
                    let info = kernel
                        .plugins
                        .install_trusted(id)
                        .map_err(|e| KernelError::Message(e.to_string()))?;
                    kernel.sync_registries();
                    kernel
                        .events
                        .emit("plugin.installed", None, json!({ "id": id }));
                    return Ok(json!(info));
                }
            }
            let entry = kernel
                .plugins
                .load_catalog()
                .into_iter()
                .find(|c| c.id == id)
                .ok_or_else(|| {
                    KernelError::Message(format!("plugin `{id}` not found in catalog"))
                })?;
            let info = kernel
                .plugins
                .install_from(&entry.package_path)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.sync_registries();
            kernel
                .events
                .emit("plugin.installed", None, json!({ "id": id }));
            Ok(json!(info))
        }
        "plugins.installFromPath" => {
            let path = str_param(&params, "path")?;
            let info = kernel
                .plugins
                .install_from(path)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.sync_registries();
            kernel
                .events
                .emit("plugin.installed", None, json!({ "id": info.manifest.id }));
            Ok(json!(info))
        }
        "plugins.approvePermissions" => {
            let id = str_param(&params, "id")?;
            let approve = params["approve"].as_bool().unwrap_or(false);
            let info = kernel
                .plugins
                .approve_permissions(id, approve)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.sync_registries();
            Ok(json!(info))
        }
        "plugins.enable" => {
            let id = str_param(&params, "id")?;
            let info = kernel
                .plugins
                .enable(id)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.sync_registries();
            kernel
                .events
                .emit("plugin.enabled", None, json!({ "id": id }));
            Ok(json!(info))
        }
        "plugins.disable" => {
            let id = str_param(&params, "id")?;
            let info = kernel
                .plugins
                .disable(id)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.sync_registries();
            kernel
                .events
                .emit("plugin.disabled", None, json!({ "id": id }));
            Ok(json!(info))
        }
        "plugins.uninstall" => {
            let id = str_param(&params, "id")?;
            kernel
                .plugins
                .uninstall(id)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.sync_registries();
            kernel
                .events
                .emit("plugin.uninstalled", None, json!({ "id": id }));
            Ok(Value::Null)
        }
        "plugins.setPinned" => {
            let id = str_param(&params, "id")?;
            let pinned = params["pinned"].as_bool().unwrap_or(false);
            kernel
                .plugins
                .set_pinned(id, pinned)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }
        "plugins.markActive" => {
            let id = str_param(&params, "id")?;
            let ms = params["activationMs"].as_f64().unwrap_or(0.0);
            kernel
                .plugins
                .set_state(id, eigendesk_plugin_runtime::PluginState::Active)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.diagnostics.record_activation(id, ms);
            Ok(Value::Null)
        }
        "plugins.reportFailure" => {
            let id = str_param(&params, "id")?;
            let reason = params["reason"].as_str().unwrap_or("unknown").to_string();
            let count = kernel
                .plugins
                .record_failure(id)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            let limit = kernel
                .settings
                .get("core.plugin.autoDisableAfterFailures")
                .as_u64()
                .unwrap_or(5) as u32;
            tracing::warn!(plugin = id, count, %reason, "plugin failure reported");
            if count >= limit {
                let _ = kernel
                    .plugins
                    .set_state(id, eigendesk_plugin_runtime::PluginState::Disabled);
                kernel.sync_registries();
                kernel.events.emit(
                    "plugin.auto-disabled",
                    None,
                    json!({ "id": id, "failures": count }),
                );
                crate::notify::push_notification(
                    kernel,
                    "Plugin disabled",
                    &format!("`{id}` failed {count} times and was disabled automatically."),
                    Some(id.to_string()),
                    vec![
                        json!({
                            "id": "restart",
                            "title": "Restart Plugin",
                            "command": "core.pluginRestart",
                            "args": { "id": id },
                        }),
                        json!({
                            "id": "logs",
                            "title": "Open Logs",
                            "command": "core.openLogsFolder",
                        }),
                    ],
                );
            }
            Ok(json!({ "failures": count }))
        }
        "plugins.resetFailures" => {
            let id = str_param(&params, "id")?;
            kernel
                .plugins
                .set_state(id, eigendesk_plugin_runtime::PluginState::Enabled)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }
        "plugins.logs" => {
            let id = str_param(&params, "id")?;
            Ok(json!(crate::PLUGIN_LOGS.get(id)))
        }
        "plugin.log" => {
            let Some(plugin_id) = &caller.plugin_id else {
                return Err(KernelError::Unauthorized(
                    "plugin.log is plugin-only".into(),
                ));
            };
            let level = params["level"].as_str().unwrap_or("info");
            let message = params["message"].as_str().unwrap_or("");
            crate::PLUGIN_LOGS.push(plugin_id, level, message);
            match level {
                "error" => tracing::error!(plugin = plugin_id, "{message}"),
                "warn" => tracing::warn!(plugin = plugin_id, "{message}"),
                "debug" => tracing::debug!(plugin = plugin_id, "{message}"),
                _ => tracing::info!(plugin = plugin_id, "{message}"),
            }
            Ok(Value::Null)
        }
        "plugins.registry" => Ok(json!(kernel.plugins.load_catalog())),
        "plugins.packs" => Ok(json!(kernel.plugins.load_packs())),
        "plugins.checkUpdates" => {
            let mut updates = vec![];
            for entry in kernel.plugins.load_catalog() {
                if let Some(rec) = kernel.plugins.get(&entry.id) {
                    if !rec.is_installed() || rec.pinned {
                        continue;
                    }
                    if semver_gt(&entry.version, &rec.manifest.version) {
                        updates.push(json!({
                            "id": entry.id,
                            "currentVersion": rec.manifest.version,
                            "availableVersion": entry.version,
                        }));
                    }
                }
            }
            Ok(json!(updates))
        }
        "plugins.update" => {
            let id = str_param(&params, "id")?;
            let rec = kernel
                .plugins
                .get(id)
                .ok_or_else(|| KernelError::Message(format!("plugin `{id}` not installed")))?;
            if rec.pinned {
                return Err(KernelError::Message(format!(
                    "plugin `{id}` is pinned to version {}",
                    rec.manifest.version
                )));
            }
            let entry = kernel
                .plugins
                .load_catalog()
                .into_iter()
                .find(|c| c.id == id)
                .ok_or_else(|| KernelError::Message(format!("plugin `{id}` not in catalog")))?;
            if !semver_gt(&entry.version, &rec.manifest.version) {
                return Ok(json!({ "updated": false }));
            }
            let info = kernel
                .plugins
                .install_from(&entry.package_path)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            kernel.sync_registries();
            kernel.events.emit(
                "plugin.updated",
                None,
                json!({ "id": id, "version": entry.version }),
            );
            Ok(json!({ "updated": true, "info": info }))
        }
        "plugins.installPack" => {
            let pack_id = str_param(&params, "packId")?;
            let pack = kernel
                .plugins
                .load_packs()
                .into_iter()
                .find(|p| p.id == pack_id)
                .ok_or_else(|| KernelError::Message(format!("pack `{pack_id}` not found")))?;
            let mut results = vec![];
            for plugin_id in &pack.plugins {
                let outcome = (|| -> KResult<Value> {
                    dispatch(
                        kernel,
                        "plugins.install",
                        json!({ "id": plugin_id }),
                        CallerCtx { plugin_id: None },
                    )?;
                    Ok(json!({ "id": plugin_id, "ok": true }))
                })()
                .unwrap_or_else(
                    |e| json!({ "id": plugin_id, "ok": false, "error": e.to_string() }),
                );
                results.push(outcome);
            }
            Ok(json!(results))
        }

        // -- artifacts ---------------------------------------------------------------
        "artifacts.upsert" => {
            let Some(plugin_id) = caller.plugin_id.clone() else {
                return Err(KernelError::Unauthorized(
                    "artifacts.upsert is plugin-only".into(),
                ));
            };
            let records = params["artifacts"].as_array().cloned().unwrap_or_default();
            let allowed_schemes = allowed_artifact_schemes(kernel, &plugin_id);
            let mut typed = Vec::with_capacity(records.len());
            for mut record in records {
                let uri = record["uri"].as_str().unwrap_or_default().to_string();
                let scheme = uri.split("://").next().unwrap_or_default().to_string();
                if !allowed_schemes.contains(&scheme) {
                    return Err(KernelError::Permission(format!(
                        "plugin `{plugin_id}` cannot register `{scheme}://` artifacts (not a contributed type)"
                    )));
                }
                record["pluginId"] = json!(plugin_id);
                typed.push(
                    serde_json::from_value(record).map_err(|e| {
                        KernelError::Message(format!("invalid artifact record: {e}"))
                    })?,
                );
            }
            let ws = kernel.require_workspace()?;
            crate::db::with_conn(&ws, |conn| {
                crate::db::artifacts(conn)
                    .upsert_many(&typed)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!({ "count": typed.len() }))
        }
        "artifacts.remove" => {
            let uri = str_param(&params, "uri")?;
            require_artifact_owner(kernel, &caller, uri)?;
            let ws = kernel.require_workspace()?;
            let removed = crate::db::with_conn(&ws, |conn| {
                crate::db::artifacts(conn)
                    .remove(uri)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!({ "removed": removed }))
        }
        "artifacts.removeByPlugin" => {
            let Some(plugin_id) = &caller.plugin_id else {
                return Err(KernelError::Unauthorized(
                    "artifacts.removeByPlugin is plugin-only".into(),
                ));
            };
            let ws = kernel.require_workspace()?;
            let removed = crate::db::with_conn(&ws, |conn| {
                crate::db::artifacts(conn)
                    .remove_by_plugin(plugin_id)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!({ "removed": removed }))
        }
        "artifacts.describe" => {
            let uri = str_param(&params, "uri")?;
            let ws = kernel.require_workspace()?;
            let record = crate::db::with_conn(&ws, |conn| {
                crate::db::artifacts(conn)
                    .describe(uri)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!(record))
        }
        "artifacts.listRecent" => {
            let limit = params["limit"].as_u64().unwrap_or(20) as u32;
            let ws = kernel.require_workspace()?;
            let records = crate::db::with_conn(&ws, |conn| {
                crate::db::artifacts(conn)
                    .list_recent(limit)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!(records))
        }
        "artifacts.listByType" => {
            let ty = str_param(&params, "type")?;
            let ws = kernel.require_workspace()?;
            let records = crate::db::with_conn(&ws, |conn| {
                crate::db::artifacts(conn)
                    .list_by_type(ty)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!(records))
        }
        "artifacts.markOpened" => {
            let uri = str_param(&params, "uri")?;
            let ws = kernel.require_workspace()?;
            crate::db::with_conn(&ws, |conn| {
                crate::db::artifacts(conn)
                    .mark_opened(uri)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(Value::Null)
        }

        // -- search --------------------------------------------------------------------
        "search.upsert" => {
            let Some(plugin_id) = caller.plugin_id.clone() else {
                return Err(KernelError::Unauthorized(
                    "search.upsert is plugin-only".into(),
                ));
            };
            let docs = params["documents"].as_array().cloned().unwrap_or_default();
            let mut typed = Vec::with_capacity(docs.len());
            for mut doc in docs {
                doc["pluginId"] = json!(plugin_id);
                typed.push(
                    serde_json::from_value(doc).map_err(|e| {
                        KernelError::Message(format!("invalid index document: {e}"))
                    })?,
                );
            }
            let ws = kernel.require_workspace()?;
            crate::db::with_conn(&ws, |conn| {
                crate::db::search(conn)
                    .upsert_many(&typed)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!({ "count": typed.len() }))
        }
        "search.removeByPlugin" => {
            let Some(plugin_id) = &caller.plugin_id else {
                return Err(KernelError::Unauthorized(
                    "search.removeByPlugin is plugin-only".into(),
                ));
            };
            let ws = kernel.require_workspace()?;
            let removed = crate::db::with_conn(&ws, |conn| {
                crate::db::search(conn)
                    .remove_by_plugin(plugin_id)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            Ok(json!({ "removed": removed }))
        }
        "search.query" => {
            let query = str_param(&params, "query")?;
            let limit = params["limit"].as_u64().unwrap_or(50) as u32;
            let start = std::time::Instant::now();
            let ws = kernel.require_workspace()?;
            let hits = crate::db::with_conn(&ws, |conn| {
                crate::db::search(conn)
                    .query(query, limit)
                    .map_err(|e| KernelError::Message(e.to_string()))
            })?;
            let took = start.elapsed().as_secs_f64() * 1000.0;
            kernel.diagnostics.record_search(took);
            Ok(json!({ "results": hits, "tookMs": took }))
        }

        // -- storage (plugin state) -----------------------------------------------------
        "storage.get" => {
            let plugin_id = require_plugin(&caller)?;
            let key = str_param(&params, "key")?;
            let store = kernel.plugin_store(&plugin_id)?;
            Ok(store.get(key).unwrap_or(Value::Null))
        }
        "storage.set" => {
            let plugin_id = require_plugin(&caller)?;
            let key = str_param(&params, "key")?;
            let value = params["value"].clone();
            let store = kernel.plugin_store(&plugin_id)?;
            store
                .set(key, value)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }
        "storage.delete" => {
            let plugin_id = require_plugin(&caller)?;
            let key = str_param(&params, "key")?;
            let store = kernel.plugin_store(&plugin_id)?;
            let removed = store
                .delete(key)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(json!({ "removed": removed }))
        }
        "storage.keys" => {
            let plugin_id = require_plugin(&caller)?;
            let store = kernel.plugin_store(&plugin_id)?;
            Ok(json!(store.keys()))
        }
        "storage.dataDir" => {
            let plugin_id = require_plugin(&caller)?;
            let store = kernel.plugin_store(&plugin_id)?;
            Ok(json!({ "path": store.data_dir().display().to_string() }))
        }
        "session.get" => {
            let plugin_id = require_plugin(&caller)?;
            let key = str_param(&params, "key")?;
            Ok(kernel.session.get(&plugin_id, key).unwrap_or(Value::Null))
        }
        "session.set" => {
            let plugin_id = require_plugin(&caller)?;
            let key = str_param(&params, "key")?;
            kernel.session.set(&plugin_id, key, params["value"].clone());
            Ok(Value::Null)
        }
        "session.delete" => {
            let plugin_id = require_plugin(&caller)?;
            let key = str_param(&params, "key")?;
            Ok(json!({ "removed": kernel.session.delete(&plugin_id, key) }))
        }
        "session.keys" => {
            let plugin_id = require_plugin(&caller)?;
            Ok(json!(kernel.session.keys(&plugin_id)))
        }

        // -- filesystem (permission-gated) -----------------------------------------------
        "fs.stat" => {
            let path = kernel.resolve_path(str_param(&params, "path")?)?;
            let canonical = fs_cap::check(kernel, &caller, false, &path)?;
            fs_cap::stat_entry(&canonical)
        }
        "fs.readDir" => {
            let path = kernel.resolve_path(str_param(&params, "path")?)?;
            let canonical = fs_cap::check(kernel, &caller, false, &path)?;
            fs_cap::read_dir_entries(&canonical)
        }
        "fs.readFile" => {
            let path = kernel.resolve_path(str_param(&params, "path")?)?;
            let canonical = fs_cap::check(kernel, &caller, false, &path)?;
            match params["encoding"].as_str().unwrap_or("utf8") {
                "base64" => fs_cap::read_base64(&canonical),
                _ => fs_cap::read_text(
                    &canonical,
                    params["maxSize"].as_u64().unwrap_or(READ_FILE_DEFAULT),
                ),
            }
        }
        "fs.writeFile" => {
            let path = kernel.resolve_path(str_param(&params, "path")?)?;
            let canonical = fs_cap::check(kernel, &caller, true, &path)?;
            let content = params["content"].as_str().unwrap_or_default();
            let create_dirs = params["createDirs"].as_bool().unwrap_or(true);
            match params["encoding"].as_str().unwrap_or("utf8") {
                "base64" => {
                    let bytes = fs_cap::b64_decode(content)?;
                    fs_cap::write_bytes(&canonical, &bytes, create_dirs)
                }
                _ => fs_cap::write_bytes(&canonical, content.as_bytes(), create_dirs),
            }?;
            Ok(Value::Null)
        }
        "fs.appendFile" => {
            let path = kernel.resolve_path(str_param(&params, "path")?)?;
            let canonical = fs_cap::check(kernel, &caller, true, &path)?;
            let content = params["content"].as_str().unwrap_or_default();
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&canonical)
                .map_err(|e| {
                    KernelError::Message(format!("cannot open `{}`: {e}", canonical.display()))
                })?;
            file.write_all(content.as_bytes())
                .map_err(|e| KernelError::Message(format!("append failed: {e}")))?;
            Ok(Value::Null)
        }
        "fs.move" => {
            let from = kernel.resolve_path(str_param(&params, "from")?)?;
            let to = kernel.resolve_path(str_param(&params, "to")?)?;
            let from_c = fs_cap::check(kernel, &caller, false, &from)?;
            let to_c = fs_cap::check(kernel, &caller, true, &to)?;
            if let Some(parent) = to_c.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    KernelError::Message(format!("cannot create `{}`: {e}", parent.display()))
                })?;
            }
            if std::fs::rename(&from_c, &to_c).is_err() {
                fs_cap::copy_path(&from_c, &to_c)?;
                let ws_root = kernel
                    .current_workspace()
                    .map(|ws| ws.workspace.root().to_path_buf());
                fs_cap::delete(&from_c, true, ws_root.as_deref())?;
            }
            Ok(Value::Null)
        }
        "fs.copy" => {
            let from = kernel.resolve_path(str_param(&params, "from")?)?;
            let to = kernel.resolve_path(str_param(&params, "to")?)?;
            let from_c = fs_cap::check(kernel, &caller, false, &from)?;
            let to_c = fs_cap::check(kernel, &caller, true, &to)?;
            fs_cap::copy_path(&from_c, &to_c)?;
            Ok(Value::Null)
        }
        "fs.delete" => {
            let path = kernel.resolve_path(str_param(&params, "path")?)?;
            let canonical = fs_cap::check(kernel, &caller, true, &path)?;
            let ws_root = kernel
                .current_workspace()
                .map(|ws| ws.workspace.root().to_path_buf());
            fs_cap::delete(
                &canonical,
                params["recursive"].as_bool().unwrap_or(false),
                ws_root.as_deref(),
            )?;
            Ok(Value::Null)
        }
        "fs.mkdir" => {
            let path = kernel.resolve_path(str_param(&params, "path")?)?;
            let canonical = fs_cap::check(kernel, &caller, true, &path)?;
            std::fs::create_dir_all(&canonical).map_err(|e| {
                KernelError::Message(format!("mkdir `{}` failed: {e}", canonical.display()))
            })?;
            Ok(Value::Null)
        }

        // -- network (permission-gated) ----------------------------------------------
        "network.fetch" => {
            let url = str_param(&params, "url")?;
            let method = params["method"].as_str().unwrap_or("GET");
            let headers = params["headers"].clone();
            let body = params["body"].as_str();
            let timeout = params["timeoutMs"].as_u64();
            crate::net::fetch(kernel, &caller, url, method, &headers, body, timeout)
        }
        "network.fetchStream" => {
            let url = str_param(&params, "url")?;
            let method = params["method"].as_str().unwrap_or("POST");
            let headers = params["headers"].clone();
            let body = params["body"].as_str();
            crate::net::fetch_stream(kernel, &caller, url, method, &headers, body)
        }
        "network.abort" => {
            let stream_id = str_param(&params, "streamId")?;
            Ok(json!({ "aborted": kernel.net.abort(stream_id) }))
        }

        // -- secrets (owner-scoped) ------------------------------------------------------
        "secrets.set" => {
            let plugin_id = require_plugin(&caller)?;
            check_flag(kernel, &caller, "secrets:read")?;
            let key = str_param(&params, "key")?;
            let value = str_param(&params, "value")?;
            kernel
                .secrets
                .set(&plugin_id, key, value)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(Value::Null)
        }
        "secrets.get" => {
            let plugin_id = require_plugin(&caller)?;
            check_flag(kernel, &caller, "secrets:read")?;
            let key = str_param(&params, "key")?;
            let value = kernel
                .secrets
                .get(&plugin_id, key)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(value.map(Value::String).unwrap_or(Value::Null))
        }
        "secrets.delete" => {
            let plugin_id = require_plugin(&caller)?;
            check_flag(kernel, &caller, "secrets:read")?;
            let key = str_param(&params, "key")?;
            let removed = kernel
                .secrets
                .delete(&plugin_id, key)
                .map_err(|e| KernelError::Message(e.to_string()))?;
            Ok(json!({ "removed": removed }))
        }
        "secrets.list" => {
            let plugin_id = require_plugin(&caller)?;
            check_flag(kernel, &caller, "secrets:read")?;
            Ok(json!(kernel.secrets.list(&plugin_id)))
        }
        "secrets.status" => Ok(kernel.secrets.status()),

        // -- pty (owner-scoped, process:spawn) ---------------------------------------------
        "pty.create" => {
            let shell = params["shell"].as_str();
            let cwd = params["cwd"].as_str();
            let cols = params["cols"].as_u64().unwrap_or(80) as u16;
            let rows = params["rows"].as_u64().unwrap_or(24) as u16;
            let env = params["env"].as_object().map(|m| Value::Object(m.clone()));
            crate::pty::create(kernel, &caller, shell, cwd, cols, rows, env.as_ref())
        }
        "pty.write" => {
            let session_id = str_param(&params, "sessionId")?;
            let data = str_param(&params, "data")?;
            let session = require_pty(kernel, &caller, session_id)?;
            let bytes = fs_cap::b64_decode(data)?;
            session.write(&bytes)?;
            Ok(Value::Null)
        }
        "pty.resize" => {
            let session_id = str_param(&params, "sessionId")?;
            let cols = params["cols"].as_u64().unwrap_or(80) as u16;
            let rows = params["rows"].as_u64().unwrap_or(24) as u16;
            let session = require_pty(kernel, &caller, session_id)?;
            session.resize(cols, rows)?;
            Ok(Value::Null)
        }
        "pty.kill" => {
            let session_id = str_param(&params, "sessionId")?;
            let session = require_pty(kernel, &caller, session_id)?;
            session.kill();
            Ok(Value::Null)
        }
        "pty.list" => {
            if let Some(plugin_id) = &caller.plugin_id {
                return Ok(json!(kernel
                    .pty
                    .list()
                    .into_iter()
                    .filter(|s| pty_owner(kernel, s["sessionId"].as_str().unwrap_or(""))
                        == Some(plugin_id.clone()))
                    .collect::<Vec<_>>()));
            }
            Ok(json!(kernel.pty.list()))
        }

        // -- notifications ---------------------------------------------------------------
        "notify.show" => {
            if caller.plugin_id.is_some() {
                check_flag(kernel, &caller, "notification")?;
            }
            let title = str_param(&params, "title")?;
            let body = params["body"].as_str().unwrap_or_default();
            let actions = params["actions"].as_array().cloned().unwrap_or_default();
            crate::notify::push_notification(
                kernel,
                title,
                body,
                caller.plugin_id.clone(),
                actions,
            );
            Ok(Value::Null)
        }
        "notify.list" => Ok(json!(*kernel.notifications.lock().unwrap())),

        // -- ai tools registry ---------------------------------------------------------
        "ai.registerTool" => {
            let plugin_id = require_plugin(&caller)?;
            let tool = params["tool"].clone();
            crate::AI_TOOLS.register(&plugin_id, tool)?;
            Ok(Value::Null)
        }
        "ai.unregisterTools" => {
            let plugin_id = require_plugin(&caller)?;
            crate::AI_TOOLS.unregister_by_plugin(&plugin_id);
            Ok(Value::Null)
        }
        "ai.listTools" => Ok(json!(crate::AI_TOOLS.list())),
        "ai.callTool" => {
            let name = str_param(&params, "name")?;
            let args = params
                .get("args")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let tool = crate::AI_TOOLS.tool(name).ok_or_else(|| {
                KernelError::Message(format!("ai tool `{name}` is not registered"))
            })?;
            let owner = tool.plugin_id.clone();
            let (request_id, rx) = crate::PENDING_TOOL_CALLS.begin(&owner);
            kernel.push.push_to_plugin(
                &owner,
                "plugin-push",
                json!({
                    "kind": "ai-tool-call",
                    "tool": name,
                    "args": args,
                    "requestId": request_id,
                }),
            );
            match rx.recv_timeout(std::time::Duration::from_secs(60)) {
                Ok(result) => {
                    if result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        Ok(result.get("result").cloned().unwrap_or(Value::Null))
                    } else {
                        Err(KernelError::Message(
                            result
                                .get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("tool failed")
                                .to_string(),
                        ))
                    }
                }
                Err(_) => Err(KernelError::Message(format!(
                    "ai tool `{name}` did not answer within 60s"
                ))),
            }
        }
        "ai.toolResult" => {
            let plugin_id = require_plugin(&caller)?;
            let request_id = params["requestId"].as_u64().unwrap_or(0);
            let ok = params["ok"].as_bool().unwrap_or(false);
            let result = params.get("result").cloned().unwrap_or(Value::Null);
            let error = params
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("tool failed");
            let payload = if ok {
                json!({ "ok": true, "result": result })
            } else {
                json!({ "ok": false, "error": error })
            };
            if !crate::PENDING_TOOL_CALLS.complete(request_id, &plugin_id, payload) {
                return Err(KernelError::Message(format!(
                    "no pending ai tool call {request_id} for `{plugin_id}`"
                )));
            }
            Ok(Value::Null)
        }

        // -- mcp --------------------------------------------------------------------------
        "mcp.status" => {
            crate::mcp::check_permission(kernel, &caller)?;
            Ok(json!(kernel.mcp.status()))
        }
        "mcp.addServer" => {
            app_only(&caller)?;
            let name = str_param(&params, "name")?;
            let command = str_param(&params, "command")?;
            let args = params["args"].as_array().cloned().unwrap_or_default();
            let args: Vec<String> = args
                .iter()
                .filter_map(|a| a.as_str().map(|s| s.to_string()))
                .collect();
            kernel.mcp.add_server(name, command, &args)?;
            Ok(Value::Null)
        }
        "mcp.removeServer" => {
            app_only(&caller)?;
            let name = str_param(&params, "name")?;
            kernel.mcp.remove_server(name)?;
            Ok(Value::Null)
        }
        "mcp.setServerEnabled" => {
            app_only(&caller)?;
            let name = str_param(&params, "name")?;
            let enabled = params["enabled"].as_bool().unwrap_or(true);
            kernel.mcp.set_server_enabled(name, enabled)?;
            Ok(Value::Null)
        }
        "mcp.connect" => {
            crate::mcp::check_permission(kernel, &caller)?;
            let name = str_param(&params, "name")?;
            let result = kernel.mcp.connect(name)?;
            kernel
                .push
                .push("mcp-status", None, json!({ "server": name }));
            Ok(result)
        }
        "mcp.listTools" => {
            crate::mcp::check_permission(kernel, &caller)?;
            Ok(json!(kernel.mcp.list_tools()))
        }
        "mcp.callTool" => {
            crate::mcp::check_permission(kernel, &caller)?;
            let server = str_param(&params, "server")?;
            let tool = str_param(&params, "tool")?;
            let arguments = params["arguments"].clone();
            kernel.mcp.call_tool(server, tool, &arguments)
        }

        _ => Err(KernelError::UnknownMethod(method.to_string())),
    }
}

const READ_FILE_DEFAULT: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

pub fn str_param<'a>(params: &'a Value, key: &str) -> KResult<&'a str> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| KernelError::Message(format!("missing string parameter `{key}`")))
}

fn parse_scope(s: &str) -> KResult<eigendesk_settings::Scope> {
    match s {
        "global" => Ok(eigendesk_settings::Scope::Global),
        "workspace" => Ok(eigendesk_settings::Scope::Workspace),
        other => Err(KernelError::Message(format!("invalid scope `{other}`"))),
    }
}

fn parse_command_def(
    v: &Value,
    plugin_id: Option<String>,
) -> KResult<eigendesk_commands::CommandDef> {
    let mut def: eigendesk_commands::CommandDef = serde_json::from_value(v.clone())
        .map_err(|e| KernelError::Message(format!("invalid command definition: {e}")))?;
    def.plugin_id = plugin_id;
    if def.id.trim().is_empty() {
        return Err(KernelError::Message("command id is required".into()));
    }
    Ok(def)
}

fn require_plugin(caller: &CallerCtx) -> KResult<String> {
    caller.plugin_id.clone().ok_or(KernelError::Unauthorized(
        "this method is only available to plugins".into(),
    ))
}

fn app_only(caller: &CallerCtx) -> KResult<()> {
    if caller.plugin_id.is_some() {
        return Err(KernelError::Unauthorized(
            "this method is restricted to the application shell".into(),
        ));
    }
    Ok(())
}

fn check_flag(kernel: &Kernel, caller: &CallerCtx, permission: &str) -> KResult<()> {
    let Some(plugin_id) = &caller.plugin_id else {
        return Ok(());
    };
    kernel
        .permissions
        .check_flag(plugin_id, permission)
        .map_err(|e| KernelError::Permission(e.to_string()))
}

fn pty_owner(kernel: &Kernel, session_id: &str) -> Option<String> {
    // Owner is tracked in the session map (set at creation).
    kernel.pty.owner_of(session_id)
}

fn require_pty(
    kernel: &Kernel,
    caller: &CallerCtx,
    session_id: &str,
) -> KResult<Arc<crate::pty::PtySession>> {
    let session = kernel
        .pty
        .get(session_id)
        .ok_or_else(|| KernelError::Message(format!("unknown pty session `{session_id}`")))?;
    if let Some(plugin_id) = &caller.plugin_id {
        if kernel.pty.owner_of(session_id).as_deref() != Some(plugin_id.as_str()) {
            return Err(KernelError::Permission(format!(
                "plugin `{plugin_id}` does not own pty session `{session_id}`"
            )));
        }
    }
    Ok(session)
}

fn workspace_json(ws: &eigendesk_workspace::Workspace) -> Value {
    json!({
        "id": ws.record.id,
        "name": ws.record.name,
        "root": ws.record.root.display().to_string(),
        "createdAt": ws.record.created_at,
        "lastOpenedAt": ws.record.last_opened_at,
    })
}

fn allowed_artifact_schemes(kernel: &Kernel, plugin_id: &str) -> Vec<String> {
    kernel
        .plugins
        .get(plugin_id)
        .map(|rec| {
            rec.manifest
                .contributes
                .artifact_types
                .iter()
                .map(|t| t.r#type.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn require_artifact_owner(kernel: &Kernel, caller: &CallerCtx, uri: &str) -> KResult<()> {
    let Some(plugin_id) = &caller.plugin_id else {
        return Ok(());
    };
    let ws = kernel.require_workspace()?;
    let record = crate::db::with_conn(&ws, |conn| {
        crate::db::artifacts(conn)
            .describe(uri)
            .map_err(|e| KernelError::Message(e.to_string()))
    })?;
    if record.plugin_id != *plugin_id {
        return Err(KernelError::Permission(format!(
            "plugin `{plugin_id}` does not own artifact `{uri}`"
        )));
    }
    Ok(())
}

fn semver_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (av, bv) = (parse(a), parse(b));
    for i in 0..3 {
        let x = av.get(i).copied().unwrap_or(0);
        let y = bv.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn reveal(path: &std::path::Path) {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(target).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(target).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(target).spawn();
    if let Err(e) = result {
        tracing::warn!(path = %target.display(), error = %e, "failed to reveal path");
    }
}

fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/c", "start", url])
        .spawn();
    if let Err(e) = result {
        tracing::warn!(url, error = %e, "failed to open url");
    }
}
