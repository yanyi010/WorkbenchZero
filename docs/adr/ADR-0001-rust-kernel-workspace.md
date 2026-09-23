# ADR-0001: Rust kernel as a workspace of small crates

- Status: Accepted
- Date: 2026-09-23
- Deciders: EigenDesk core

## Context

EigenDesk's desktop shell is Tauri 2. The kernel (settings, workspaces,
permissions, plugin runtime, search, artifacts, PTY, network, MCP, AI
tool routing) must be testable without a window manager and reusable by
a future standalone MCP server binary. Spec §115 sequences delivery so
the kernel is the foundation.

## Decision

Implement the kernel as a Cargo workspace of focused crates under
`crates/`:

```
kernel            dispatch + orchestration (the only crate the Tauri
                  shell links)
plugin-runtime    manifest, discovery, lifecycle, catalog, packages
permissions       grant evaluation, filesystem scopes
settings          descriptors + global/workspace stores
workspace         registry, open/close, watchers
storage           app dirs, plugin state (quota'd KV)
secrets           keychain with documented fallback (ADR-0007)
artifacts         cross-plugin artifact index
search            FTS index and query
events            bus with retention
commands          registry with activation matching
```

Rules:

1. `eigendesk-kernel` is the only crate that depends on all others; the
   others stay independent of each other where possible.
2. Every crate is unit-tested headlessly (`cargo test -p <crate>`); no
   test requires a display, D-Bus, or network.
3. The single JSON-RPC-style dispatch (`Kernel::rpc`) is the only entry
   point; there is no crate-to-crate HTTP or channel plumbing.
4. Wire types use `#[serde(rename_all = "camelCase")]` and are mirrored
   in `packages/protocol` with a drift-guard test (see ADR-0003).

## Consequences

- The Tauri binary stays thin (`apps/desktop/src-tauri` is transport
  only).
- A standalone `eigendesk-mcp` server binary can reuse the kernel.
- Compile times stay acceptable: crates compile in parallel; the shell
  rebuilds only when the kernel changes.
