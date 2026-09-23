# ADR-0004: Plugin bridge — postMessage with trusted identity stamping

- Status: Accepted
- Date: 2026-09-23
- Deciders: Workbench Zero core

## Context

Plugin frames (ADR-0002) have no kernel channel. They must call kernel
methods, receive pushes, and answer commands routed from the palette /
Quick Capture — without being able to impersonate another plugin or
forge a caller.

## Decision

The main frame (`apps/desktop/src/pluginHost.ts`) owns the bridge:

```
plugin frame                      main frame                kernel
    │  wz-ready                     │                        │
    │───────────────────────────────▶│                        │
    │  wz-init {pluginId, surface}  │                        │
    │◀───────────────────────────────│                        │
    │  wz-rpc {id, method, params}  │  kernel_rpc(token,     │
    │───────────────────────────────▶│    {…, __plugin: id}) ─▶│
    │  wz-rpc-result                │                        │
    │◀───────────────────────────────│◀───────────────────────│
```

1. **Identity stamping**: the main frame injects `__plugin:
   <frameOwnerId>` into every `wz-rpc` before forwarding. A plugin
   cannot override its identity — the kernel reads the stamped field,
   not the payload.
2. **Handshake**: `wz-ready` → `wz-init` (authoritative pluginId +
   surface) → optional `wz-manifest-request`/`wz-manifest`.
3. **Commands**: shell → plugin as `wz-command {requestId, id, args}`;
   the plugin answers `wz-command-result {requestId, ok, result|
   error}`. The shell enforces a 30 s ack deadline (spec §37 — a hung
   plugin must be observable, not silent).
4. **Pushes**: kernel pushes targeting a plugin are forwarded as
   `wz-push {topic, data}` to all of that plugin's frames; broadcast
   events go to every frame and the SDK filters by name.
5. Every message is validated for shape; unknown types are dropped
   silently (forward compatibility).

## Consequences

- The SDK surface (`@workbench-zero/plugin-sdk`) is the only sanctioned way
  to talk; the wire is stable and versioned via `apiVersion`.
- A compromised frame can at worst issue RPCs *as itself*, within its
  granted permissions.
- Command routing needs no kernel round trip for metadata — manifests
  are synced kernel-side (spec §13) and the shell routes by table.
