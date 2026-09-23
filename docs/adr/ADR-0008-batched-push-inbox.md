# ADR-0008: Kernel→shell push channel — batched eval inbox

- Status: Accepted
- Date: 2026-09-23
- Deciders: Workbench Zero core

## Context

The kernel must notify the shell (plugin state changes, notifications,
plugin-targeted streams like PTY/net, broadcast events) without giving
the webview a listening socket, and without one Tauri event per byte of
terminal output.

## Decision

1. The kernel takes a `PushSink: Arc<dyn Fn(&[PushMessage]) + Send +
   Sync>` at bootstrap. The Tauri shell installs a sink that serializes
   the batch and evaluates:

   ```js
   window.__kernelInbox(batch)
   ```

2. **Batching**: the kernel coalesces queued pushes (up to
   `MAX_BATCH`) on a short flush interval — terminal output and search
   index updates arrive as batches, not per-byte events.
3. **Sequencing**: every message carries a monotonically increasing
   `seq`; the shell can detect gaps (used by diagnostics).
4. Topics: `event` (broadcast), `plugin-state`, `notification`,
   `plugin-push` (AI tool calls), `net` (stream chunks), `pty`,
   `mcp-status`, `shortcut`. Plugin-targeted topics carry `plugin`;
   the shell routes those to the plugin's frames (ADR-0004).
5. `window.__kernelInbox` and `window.__quickCapture` are the only
   kernel-reachable globals; both are installed by `main.tsx` before
   any plugin frame exists.

## Consequences

- One IPC primitive covers low-rate notifications and high-rate
  streams; per-message overhead is amortized by batching.
- The webview remains pull-only for RPC and push-only for inbox — easy
  to reason about for security review.
- E2E tests can drive the full push path by calling
  `window.__kernelInbox` directly.
