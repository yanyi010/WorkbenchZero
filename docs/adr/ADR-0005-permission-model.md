# ADR-0005: Declarative permissions with scoped filesystem roots

- Status: Accepted
- Date: 2026-09-23
- Deciders: EigenDesk core

## Context

Spec §31–33: plugins declare permissions in the manifest; the user
approves them at install (and re-approves on drift); the kernel
enforces them per-call. Filesystem access must be scopeable to the
workspace without a second permission system.

## Decision

1. Manifests declare `permissions: string[]` against a **closed set**
   of names the kernel knows (`KNOWN_PERMISSIONS`). Unknown names fail
   validation (`wb plugin validate` locally, kernel at install).
2. `workspace:read` / `workspace:write` are **aliases** for
   `filesystem:read` / `filesystem:write` whose root is bound to the
   current workspace root at check time. This keeps the permission list
   small while giving users the meaningful mental model ("this plugin
   touches my workspace").
3. `filesystem:*` grants may carry root restrictions; `${workspace}`
   inside a root is substituted at evaluation.
4. Grants are stored per plugin; **permission drift on update requires
   re-approval** (tested in `crates/plugin-runtime`).
5. Everything else is deny-by-default at the dispatcher: `net.*` needs
   `network`, `pty.*` needs `process:spawn`, `secrets.*` needs the
   `secrets:read` **flag** (deliberately stronger than a normal grant),
   `mcp.callTool` needs `mcp:connect`, `ai.*` needs `ai:invoke`.
6. Path checks canonicalize lexically **and** resolve symlinks before
   prefix-matching, defeating `..` and symlink escapes (tested in
   `crates/permissions`).

## Consequences

- The approval dialog (`PluginStore`) shows human-readable descriptions
  from `PERMISSION_DESCRIPTIONS`; no raw capability strings.
- Core commands executed by the shell bypass plugin checks (caller is
  the trusted main frame) — by design, the shell *is* the user.
