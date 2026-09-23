//! EigenDesk desktop shell: wires the UI-framework-agnostic kernel into
//! Tauri 2. Responsibilities kept intentionally thin (spec §122: the shell
//! coordinates, plugins implement products):
//!
//! - one JSON-RPC command (`kernel_rpc`) guarded by a bootstrap token;
//! - a push sink that evaluates batches into the main frame only (plugin
//!   iframes have no direct kernel channel);
//! - the `edp://` scheme serving plugin packages from disk with path
//!   containment and a strict CSP;
//! - global Quick Capture shortcut + updater wiring.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eigendesk_kernel::{Kernel, KernelConfig, PushMessage, RpcRequest, RpcResponse};
use tauri::http::{Request, Response};
use std::borrow::Cow;
use tauri::Manager;

struct KernelState(Arc<Kernel>);

const EDP_SCHEME: &str = "edp";

#[tauri::command]
fn kernel_rpc(
    state: tauri::State<'_, KernelState>,
    token: String,
    payload: RpcRequest,
) -> RpcResponse {
    state.0.rpc(&token, payload)
}

/// Toggle the global Quick Capture shortcut (Alt+Space).
#[tauri::command]
async fn set_global_capture(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let shortcuts = app.global_shortcut();
    const ACCEL: &str = "Alt+Space";
    if enabled {
        if !shortcuts.is_registered(ACCEL) {
            let app_handle = app.clone();
            shortcuts
                .on_shortcut(ACCEL, move |_app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        if let Some(webview) = app_handle.get_webview_window("main") {
                            let _ = webview.eval("window.__quickCapture && window.__quickCapture();");
                        }
                    }
                })
                .map_err(|e| format!("cannot register global shortcut: {e}"))?;
        }
    } else if shortcuts.is_registered(ACCEL) {
        let _ = shortcuts.unregister(ACCEL);
    }
    Ok(())
}

fn push_sink(app: tauri::AppHandle) -> Arc<dyn Fn(&[PushMessage]) + Send + Sync + 'static> {
    Arc::new(move |batch: &[PushMessage]| {
        if batch.is_empty() {
            return;
        }
        let Ok(json) = serde_json::to_string(batch) else { return };
        if let Some(webview) = app.get_webview_window("main") {
            let script = format!("window.__kernelInbox&&window.__kernelInbox({json});");
            if let Err(e) = webview.eval(&script) {
                tracing::warn!(error = %e, "push eval failed");
            }
        }
    })
}

/// Serve plugin package files: `edp://<pluginId>/<path>?surface=...`.
/// Path containment: the resolved file must live inside the plugin's install
/// directory; symlinks are resolved before the check.
fn serve_edp(kernel: &Kernel, uri: &str) -> Response<Cow<'static, [u8]>> {
    let bad_request = |msg: &str| -> Response<Cow<'static, [u8]>> {
        Response::builder()
            .status(400)
            .header("Content-Type", "text/plain")
            .body(Cow::Owned(msg.as_bytes().to_vec()))
            .unwrap()
    };
    let not_found = || -> Response<Cow<'static, [u8]>> {
        Response::builder()
            .status(404)
            .header("Content-Type", "text/plain")
            .body(Cow::Owned(b"not found".to_vec()))
            .unwrap()
    };

    let rest = uri.strip_prefix(&format!("{EDP_SCHEME}://")).unwrap_or("");
    let (host, path_query) = match rest.split_once('/') {
        Some((h, p)) => (h, p),
        None => (rest, ""),
    };
    let path = path_query.split(['?', '#']).next().unwrap_or("");
    let plugin_id = host.to_string();

    if !eigendesk_plugin_runtime::valid_plugin_id(&plugin_id) {
        return bad_request("invalid plugin id");
    }
    let Some(record) = kernel.plugins.get(&plugin_id) else {
        return not_found();
    };
    let base = record.install_path;
    let rel = path.trim_start_matches('/');
    if rel.is_empty() {
        return bad_request("path required");
    }
    // Manual component validation before joining (defense in depth; the
    // canonical check below is the authoritative containment test).
    for seg in rel.split('/') {
        if seg == ".." || seg.is_empty() && rel.contains("//") {
            return bad_request("invalid path");
        }
    }
    let target = base.join(rel);
    let canonical = match eigendesk_permissions::resolve(&target) {
        Ok(c) => c,
        Err(_) => return bad_request("invalid path"),
    };
    let canonical_base = match eigendesk_permissions::resolve(&base) {
        Ok(c) => c,
        Err(_) => return not_found(),
    };
    if !canonical.starts_with(&canonical_base) || !canonical.is_file() {
        return not_found();
    }
    let Ok(bytes) = std::fs::read(&canonical) else {
        return not_found();
    };
    let mime = mime_for(&canonical);
    let mut builder = Response::builder()
        .status(200)
        .header("Content-Type", mime)
        .header("Cache-Control", "no-cache");
    if mime.starts_with("text/html") {
        // Strict plugin sandbox: scripts only from the plugin origin, no
        // direct network from plugin frames (bridged through the kernel).
        builder = builder.header(
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; font-src 'self'; connect-src 'none'; \
             object-src 'none'; base-uri 'none'; form-action 'none'",
        );
    }
    builder.body(Cow::Owned(bytes)).unwrap()
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn resource_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().resource_dir().ok()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let safe_mode = std::env::args().any(|a| a == "--safe-mode" || a == "--safe");
    let version = env!("CARGO_PKG_VERSION").to_string();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        if let Some(webview) = app.get_webview_window("main") {
                            let script = r#"window.__kernelInbox&&window.__kernelInbox([{topic:"shortcut",payload:{id:"quick-capture"}}]);"#;
                            let _ = webview.eval(script);
                        }
                    }
                })
                .build(),
        )
        .register_uri_scheme_protocol(EDP_SCHEME, |ctx, request: Request<Vec<u8>>| {
            let app = ctx.app_handle();
            let state = app.state::<KernelState>();
            let uri = request.uri().to_string();
            serve_edp(&state.0, &uri)
        })
        .invoke_handler(tauri::generate_handler![kernel_rpc, set_global_capture])
        .setup(move |app| {
            let bundled_dir = resource_dir(&app.handle()).map(|r| r.join("plugins"));
            let kernel = Kernel::bootstrap(
                KernelConfig {
                    app_version: version,
                    bundled_dir,
                    data_override: None,
                    safe_mode,
                },
                push_sink(app.handle().clone()),
            )
            .map_err(|e| {
                eprintln!("fatal: kernel bootstrap failed: {e:#}");
                e
            })?;
            app.manage(KernelState(kernel.clone()));

            // Register the global Quick Capture shortcut if enabled.
            let global_capture = kernel
                .settings
                .get("core.capture.globalShortcut")
                .as_bool()
                .unwrap_or(false);
            if global_capture {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = set_global_capture(app_handle, true).await;
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.app_handle().try_state::<KernelState>() {
                    state.0.shutdown();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running EigenDesk");
}
