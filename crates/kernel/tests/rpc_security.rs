//! Integration tests for the kernel RPC security contract (v1):
//! privilege separation between the shell and plugin callers, the token
//! handshake's non-disclosure properties, and permission-gated AI calls.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use wz_kernel::{Kernel, KernelConfig, PushMessage, RpcRequest};

fn test_kernel() -> Arc<Kernel> {
    static N: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "wz-kernel-sec-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&base);
    Kernel::bootstrap(
        KernelConfig {
            app_version: "test".into(),
            bundled_dir: None,
            data_override: Some(base),
            safe_mode: false,
        },
        Arc::new(|_: &[PushMessage]| {}),
    )
    .unwrap()
}

fn req(id: u64, method: &str, params: serde_json::Value) -> RpcRequest {
    RpcRequest {
        id,
        method: method.to_string(),
        params,
    }
}

/// Call as an untrusted plugin.
fn rpc_as_plugin(kernel: &Arc<Kernel>, token: &str, method: &str, params: serde_json::Value) -> wz_kernel::RpcResponse {
    let mut params = match params {
        serde_json::Value::Object(m) => serde_json::Value::Object(m),
        _ => serde_json::json!({}),
    };
    params["__plugin"] = serde_json::json!("evil.plugin");
    kernel.rpc(token, req(1, method, params))
}

#[test]
fn token_issue_twice_never_discloses() {
    let kernel = test_kernel();
    let first = kernel.issue_token().unwrap();
    let second = kernel.issue_token();
    let err = second.expect_err("second issue must fail");
    let msg = err.to_string();
    assert!(
        !msg.contains(&first),
        "token must never appear in error messages: {msg}"
    );
    // And the first token still works.
    kernel.check_token(&first).unwrap();
    assert!(kernel.check_token("wrong-token").is_err());
}

#[test]
fn plugin_cannot_run_shell_lifecycle_methods() {
    let kernel = test_kernel();
    let token = kernel.issue_token().unwrap();
    // These mutate plugin/workspace lifecycle globally; a plugin caller must
    // be refused before any state is touched.
    for (method, params) in [
        ("plugins.enable", serde_json::json!({ "id": "zero.memo" })),
        ("plugins.disable", serde_json::json!({ "id": "zero.memo" })),
        ("plugins.uninstall", serde_json::json!({ "id": "zero.memo" })),
        ("plugins.setPinned", serde_json::json!({ "id": "zero.memo", "pinned": true })),
        ("plugins.installFromPath", serde_json::json!({ "path": "/tmp/x" })),
        ("plugins.resetFailures", serde_json::json!({ "id": "zero.memo" })),
        ("plugins.markActive", serde_json::json!({ "id": "zero.memo", "activationMs": 1.0 })),
        ("plugins.checkUpdates", serde_json::json!({})),
        ("workspace.create", serde_json::json!({ "name": "x", "root": "/tmp/x", "createRoot": true })),
        ("workspace.remove", serde_json::json!({ "id": "x" })),
        ("app.exportDiagnostics", serde_json::json!({})),
    ] {
        let resp = rpc_as_plugin(&kernel, &token, method, params);
        assert!(
            !resp.ok,
            "plugin caller must be refused `{method}`: {resp:?}"
        );
        let err = resp.error.unwrap();
        assert_eq!(err.code, "kernel/unauthorized", "`{method}`: {err:?}");
    }
}

#[test]
fn ai_call_tool_requires_invoke_permission() {
    let kernel = test_kernel();
    let token = kernel.issue_token().unwrap();
    // The tool exists nowhere; the permission check happens first.
    let resp = rpc_as_plugin(
        &kernel,
        &token,
        "ai.callTool",
        serde_json::json!({ "name": "whatever", "args": {} }),
    );
    assert!(!resp.ok);
    assert_eq!(resp.error.unwrap().code, "kernel/permission-denied");
}

#[test]
fn shell_caller_is_privileged() {
    let kernel = test_kernel();
    let token = kernel.issue_token().unwrap();
    // Shell (no __plugin): lifecycle methods reachable.
    let resp = kernel.rpc(
        &token,
        req(1, "workspace.list", serde_json::json!({})),
    );
    assert!(resp.ok, "shell must list workspaces: {resp:?}");
}

#[test]
fn settings_scoping_blocks_cross_plugin_writes() {
    let kernel = test_kernel();
    let token = kernel.issue_token().unwrap();
    let resp = rpc_as_plugin(
        &kernel,
        &token,
        "settings.set",
        serde_json::json!({
            "scope": "global",
            "key": "core.appearance.theme",
            "value": "dark",
        }),
    );
    assert!(!resp.ok);
    assert_eq!(resp.error.unwrap().code, "kernel/permission-denied");
}
