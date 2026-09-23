# Architecture

- [ADR index](../adr/) — decision records (authoritative)
- [Kernel crates](../../crates/) — one crate per concern (ADR-0001)
- `crates/kernel/src/rpc.rs` — the single method dispatcher; the TS
  mirror in `packages/protocol` is drift-tested against it
- `apps/desktop/src-tauri/src/lib.rs` — Tauri transport: `kernel_rpc`,
  `set_global_capture`, the `wzp://` protocol handler, push sink

## Data flow at a glance

```
user ─▶ shell (React) ─kernel_rpc(token)─▶ kernel dispatch ─▶ crates
              ▲                                   │
              └──── window.__kernelInbox(batch) ──┘ (ADR-0008)

plugin frame ─postMessage─▶ pluginHost (identity stamp) ─kernel_rpc─▶ kernel
```

## Startup order (ADR-0003)

1. shell calls `app.issueToken` (first caller wins)
2. installs `window.__kernelInbox` / `window.__quickCapture`
3. `store.boot()`: bootstrapInfo → workspaces, plugins, settings,
   **commands** (keybindings resolve from the command table)
4. manifests cached; first-run detection opens the Welcome overlay
5. kernel logs "ready" with startup timing (`logs/workbench-zero/app.log…`)
