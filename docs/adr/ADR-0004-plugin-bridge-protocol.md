# ADR-0004: Plugin bridge — postMessage with trusted identity stamping

- Status: Accepted
- Date: 2026-09-23
- Deciders: EigenDesk core

## Context

Plugin frames (ADR-0002) have no kernel channel. They must call kernel
methods, receive pushes, and answer commands routed from the palette /
Quick Capture — without being able to impersonate another plugin or
forge a caller.

## Decision

The main frame (`apps/desktop/src/pluginHost.ts`) owns the bridge:

```
plugin frame                      main frame                kernel
    │  edp-ready                     │                        │
    │───────────────────────────────▶│                        │
    │  edp-init {pluginId, surface}  │                        │
    │◀───────────────────────────────│                        │
    │  edp-rpc {id, method, params}  │  kernel_rpc(token,     │
    │───────────────────────────────▶│    {…, __plugin: id}) ─▶│
    │  edp-rpc-result                │                        │
    │◀───────────────────────────────│◀───────────────────────│
```

1. **Identity stamping**: the main frame injects `__plugin:
   <frameOwnerId>` into every `edp-rpc` before forwarding. A plugin
   cannot override its identity — the kernel reads the stamped field,
   not the payload.
2. **Handshake**: `edp-ready` → `edp-init` (authoritative pluginId +
   surface) → optional `edp-manifest-request`/`edp-manifest`.
3. **Commands**: shell → plugin as `edp-command {requestId, id, args}`;
   the plugin answers `edp-command-result {requestId, ok, result|
   error}`. The shell enforces a 30 s ack deadline (spec §37 — a hung
   plugin must be observable, not silent).
4. **Pushes**: kernel pushes targeting a plugin are forwarded as
   `edp-push {topic, data}` to all of that plugin's frames; broadcast
   events go to every frame and the SDK filters by name.
5. Every message is validated for shape; unknown types are dropped
   silently (forward compatibility).

## Consequences

- The SDK surface (`@eigendesk/plugin-sdk`) is the only sanctioned way
  to talk; the wire is stable and versioned via `apiVersion`.
- A compromised frame can at worst issue RPCs *as itself*, within its
  granted permissions.
- Command routing needs no kernel round trip for metadata — manifests
  are synced kernel-side (spec §13) and the shell routes by table.
