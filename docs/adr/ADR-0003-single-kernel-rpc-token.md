# ADR-0003: Single `kernel_rpc` IPC with a bootstrap token

- Status: Accepted
- Date: 2026-09-23
- Deciders: EigenDesk core

## Context

Tauri commands are individually exported globals in the webview — every
`invoke('…')` name is callable by any JS that runs in the main frame.
With plugin iframes in the same webview (ADR-0002), we must guarantee
that only the trusted shell can call privileged kernel methods, and we
want one typed, versioned, audit-able surface instead of dozens of Tauri
commands.

## Decision

1. The Tauri shell exposes exactly **one** privileged command:

   ```ts
   invoke('kernel_rpc', { token, payload: RpcRequest }): RpcResponse
   ```

2. On startup the shell calls `app.issueToken` with an empty token; the
   kernel hands the token to the **first caller** and rejects everyone
   else afterwards. The token lives in a module-scoped variable inside
   `apps/desktop/src/kernel.ts`, never on `window`.
3. `RpcRequest = { id, method, params }`; `RpcResponse` echoes `id` and
   is either `{ ok: true, result }` or `{ ok: false, error: { code,
   message, data? } }`. Error codes mirror `KernelError::code()`.
4. The method table (`crates/kernel/src/rpc.rs`) is the single
   dispatcher; `packages/protocol/src/index.ts` mirrors it as the
   `Methods` map and the **drift-guard test** parses the Rust match arms
   and fails on any method present in one and missing in the other.
5. Kernel→shell pushes travel as `eval` of `window.__kernelInbox(batch)`
   (ADR-0008); the global Quick Capture hook is
   `window.__quickCapture()` — both are shell-owned globals.

## Consequences

- Plugin frames cannot call `invoke` at all (sandboxed cross-origin;
  no access to the Tauri API bundle in the main frame).
- Adding a kernel method requires touching exactly two files, and the
  drift test forces them to stay in sync.
- Params are size-capped (`RPC_MAX_PARAMS_BYTES`) at the dispatcher.
