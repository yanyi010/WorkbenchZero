# Security Policy

## Supported versions

| Version | Supported          |
| ------- | ------------------ |
| 1.0.x   | :white_check_mark: |

## Security model (summary)

The kernel is the sole trust boundary. Plugin code runs in sandboxed,
cross-origin `wzp://` iframes with no direct kernel channel; the shell
stamps the caller identity (`__plugin`) in the trusted main frame so plugins
cannot spoof each other. Highlights:

- **Permissions**: declared in the manifest, approved by the user at install
  or when they change (an upgrade that widens permissions requires
  re-approval; a scoped declaration becoming unscoped counts as widening).
- **Privilege separation**: plugin lifecycle, workspace management and
  diagnostics export are shell-only RPCs. `ai.callTool` requires `ai:invoke`;
  secret mutation requires `secrets:write` (distinct from `secrets:read`).
- **Tokens & ids**: the RPC token is issued once to the shell, compared in
  constant time, and never echoed in error messages. AI tool-call ids are
  unguessable UUIDs, and a result is only accepted from the owning plugin.
- **Filesystem**: plugin `fs.*` operations are scoped by permission to the
  workspace or explicit paths; path traversal is contained with canonical
  resolution (also for MCP servers). Writes are atomic (tmp + fsync + rename).
- **Network**: plugin requests are routed through the kernel; streamed
  responses are delivered only to the owning plugin's frames. Error messages
  redact URLs.
- **Sandbox**: iframes use `allow-scripts allow-modals` (no same-origin
  access, no top navigation); plugin HTML gets a strict CSP (`connect-src
  'none'`).

For the full model see `docs/permissions/`, `docs/adr/ADR-0005-permission-model.md`
and `docs/adr/ADR-0002-plugin-sandbox-edp-iframes.md`.

## Data safety

See `docs/data-safety/README.md` for the durability story: atomic writes,
corrupt-state quarantine, automatic workspace snapshots, and what snapshots
cover (and deliberately do not).

## Reporting a vulnerability

Please **do not** open a public issue for security vulnerabilities.

Report privately via GitHub's
[private vulnerability reporting](https://github.com/yanyi010/WorkbenchZero/security/advisories/new)
(“Report a vulnerability” on the Security tab).

Include: affected version, reproduction steps or PoC, impact assessment, and
whether the issue is exploitable from plugin code. You can expect an
acknowledgement within 72 hours and a triage decision within a week.
