//! Workbench Zero desktop shell: wires the UI-framework-agnostic kernel into
//! Tauri 2. Responsibilities kept intentionally thin (spec §122: the shell
//! coordinates, plugins implement products):
//!
//! - one JSON-RPC command (`kernel_rpc`) guarded by a bootstrap token;
//! - a push sink that evaluates batches into the main frame only (plugin
//!   iframes have no direct kernel channel);
//! - the `wzp://` scheme serving plugin packages from disk with path
//!   containment and a strict CSP;
//! - global Quick Capture shortcut + updater wiring.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::borrow::Cow;
use tauri::http::{Request, Response};
use tauri::Manager;
use wz_kernel::{Kernel, KernelConfig, PushMessage, RpcRequest, RpcResponse};

struct KernelState(Arc<Kernel>);

const WZP_SCHEME: &str = "wzp";

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
async fn set_global_capture(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
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
                            let _ =
                                webview.eval("window.__quickCapture && window.__quickCapture();");
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

/// The kernel's push channel into the webview (eval of `__kernelInbox`).
type PushSinkFn = Arc<dyn Fn(&[PushMessage]) + Send + Sync + 'static>;

fn push_sink(app: tauri::AppHandle) -> PushSinkFn {
    Arc::new(move |batch: &[PushMessage]| {
        if batch.is_empty() {
            return;
        }
        let Ok(json) = serde_json::to_string(batch) else {
            return;
        };
        if let Some(webview) = app.get_webview_window("main") {
            let script = format!("window.__kernelInbox&&window.__kernelInbox({json});");
            if let Err(e) = webview.eval(&script) {
                tracing::warn!(error = %e, "push eval failed");
            }
        }
    })
}

/// Serve plugin package files: `wzp://<pluginId>/<path>?surface=...`.
/// Path containment: the resolved file must live inside the plugin's install
/// directory; symlinks are resolved before the check.
/// Builder helpers never panic: this is the security boundary between
/// plugin code and the local filesystem, and a panicking handler terminating
/// the webview's protocol thread is a worse failure than a 500.
fn response(status: u16, mime: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .body(Cow::Owned(body))
        .unwrap_or_else(|_| {
            // Invariant fallback: static status/headers cannot fail.
            Response::new(Cow::Owned(Vec::new()))
        })
}

fn serve_wzp(kernel: &Kernel, uri: &str) -> Response<Cow<'static, [u8]>> {
    let bad_request = |msg: &str| -> Response<Cow<'static, [u8]>> {
        response(400, "text/plain", msg.as_bytes().to_vec())
    };
    let not_found = || -> Response<Cow<'static, [u8]>> { response(404, "text/plain", b"not found".to_vec()) };

    let rest = uri.strip_prefix(&format!("{WZP_SCHEME}://")).unwrap_or("");
    let (host, path_query) = match rest.split_once('/') {
        Some((h, p)) => (h, p),
        None => (rest, ""),
    };
    let path = path_query.split(['?', '#']).next().unwrap_or("");
    let plugin_id = host.to_string();

    if !wz_plugin_runtime::valid_plugin_id(&plugin_id) {
        return bad_request("invalid plugin id");
    }
    let Some(record) = kernel.plugins.get(&plugin_id) else {
        return not_found();
    };
    // Only installed plugins have servable assets: a Discovered or
    // Uninstalled plugin's files on disk are inert and must not be reachable
    // through the protocol handler.
    if !record.is_installed() {
        return not_found();
    }
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
    let canonical = match wz_permissions::resolve(&target) {
        Ok(c) => c,
        Err(_) => return bad_request("invalid path"),
    };
    let canonical_base = match wz_permissions::resolve(&base) {
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
    let mut resp = response(200, mime, bytes);
    resp.headers_mut()
        .insert("Cache-Control", "no-cache".parse().unwrap_or_else(|_| tauri::http::HeaderValue::from_static("no-cache")));
    if mime.starts_with("text/html") {
        // Strict plugin sandbox: scripts only from the plugin origin, no
        // direct network from plugin frames (bridged through the kernel).
        if let Ok(v) = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline';              img-src 'self' data: blob:; font-src 'self'; connect-src 'none';              object-src 'none'; base-uri 'none'; form-action 'none'"
            .parse()
        {
            resp.headers_mut().insert("Content-Security-Policy", v);
        }
    }
    resp
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
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .register_uri_scheme_protocol(WZP_SCHEME, |ctx, request: Request<Vec<u8>>| {
            let app = ctx.app_handle();
            let state = app.state::<KernelState>();
            let uri = request.uri().to_string();
            serve_wzp(&state.0, &uri)
        })
        .invoke_handler(tauri::generate_handler![kernel_rpc, set_global_capture])
        .setup(move |app| {
            // Tauri maps the `resources/**/*` config onto
            // `<resource_dir>/resources/**` (both in dev and in the deb/
            // AppImage layouts).
            let bundled_dir =
                resource_dir(app.handle()).map(|r| r.join("resources").join("plugins"));
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
        .unwrap_or_else(|e| {
            // Event-loop teardown after an unrecoverable error. State writes
            // are write-through and fsynced, so nothing is lost by exiting.
            eprintln!("fatal: Workbench Zero terminated: {e}");
            std::process::exit(1);
        });
}

// ---------------------------------------------------------------------------
// Tests: the wzp:// protocol handler is security-critical (plugin sandbox
// boundary, ADR-0002) — path containment and CSP are tested directly.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn test_kernel() -> Arc<Kernel> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "wz-tauri-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let bundled = base.join("resources/plugins");
        let pkg = bundled.join("test.demo");
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(
            pkg.join("plugin.json"),
            r#"{"id":"test.demo","name":"Demo","version":"0.1.0","apiVersion":"1",
                "publisher":"test","trust":"sandboxed","permissions":[],
                "activationEvents":["onStartup"],"contributes":{"commands":[]}}"#,
        )
        .unwrap();
        std::fs::write(pkg.join("entry.html"), "<html><body>hi</body></html>").unwrap();
        std::fs::write(pkg.join("dist/main.js"), "console.log(1)").unwrap();
        std::fs::write(base.join("secret.txt"), "top secret").unwrap();

        Kernel::bootstrap(
            KernelConfig {
                app_version: "test".into(),
                bundled_dir: Some(bundled),
                data_override: Some(base.join("data")),
                safe_mode: false,
            },
            Arc::new(|_batch: &[PushMessage]| {}),
        )
        .unwrap()
    }

    /// Plugins only become servable once installed (spec lifecycle).
    fn installed_kernel() -> Arc<Kernel> {
        let kernel = test_kernel();
        kernel
            .plugins
            .set_state("test.demo", wz_plugin_runtime::PluginState::Enabled)
            .unwrap();
        kernel
    }

    #[test]
    fn edp_serves_entry_with_strict_csp() {
        let kernel = installed_kernel();
        let resp = serve_wzp(&kernel, "wzp://test.demo/entry.html?surface=logic");
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("Content-Type").unwrap(),
            "text/html; charset=utf-8"
        );
        let csp = resp.headers().get("Content-Security-Policy").unwrap();
        let csp = csp.to_str().unwrap();
        assert!(
            csp.contains("connect-src 'none'"),
            "CSP must block direct network"
        );
        assert!(
            csp.contains("script-src 'self'"),
            "CSP must restrict scripts"
        );
    }

    #[test]
    fn edp_serves_bundled_assets() {
        let kernel = installed_kernel();
        let resp = serve_wzp(&kernel, "wzp://test.demo/dist/main.js");
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("Content-Type").unwrap(),
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn edp_rejects_path_traversal() {
        let kernel = installed_kernel();
        // `..` must not escape the plugin package.
        for uri in [
            "wzp://test.demo/../secret.txt",
            "wzp://test.demo/dist/../../secret.txt",
            "wzp://test.demo/..%2Fsecret.txt",
        ] {
            let resp = serve_wzp(&kernel, uri);
            assert!(
                resp.status() == 400 || resp.status() == 404,
                "traversal `{uri}` must be rejected, got {}",
                resp.status()
            );
        }
    }

    #[test]
    fn edp_unknown_plugin_is_404() {
        let kernel = test_kernel();
        let resp = serve_wzp(&kernel, "wzp://other.plugin/entry.html");
        assert_eq!(resp.status(), 404);
    }

    #[test]
    fn edp_invalid_plugin_id_is_400() {
        let kernel = test_kernel();
        let resp = serve_wzp(&kernel, "wzp://../etc/entry.html");
        assert_eq!(resp.status(), 400);
    }

    #[test]
    fn edp_discovered_plugin_not_served() {
        let kernel = test_kernel();
        // Discovered (never installed) — assets must be unreachable.
        let resp = serve_wzp(&kernel, "wzp://test.demo/entry.html");
        assert_eq!(resp.status(), 404);
    }

    #[test]
    fn edp_empty_path_is_rejected() {
        let kernel = installed_kernel();
        let resp = serve_wzp(&kernel, "wzp://test.demo/");
        assert_eq!(resp.status(), 400);
    }
}
