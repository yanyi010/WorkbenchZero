# Permissions

Closed set, deny-by-default, enforced per call (ADR-0005).

| permission | gates | user-facing meaning |
|---|---|---|
| `workspace:read` | fs reads scoped to workspace root | "Read files in this workspace" |
| `workspace:write` | fs writes scoped to workspace root | "Create/modify files in this workspace" |
| `filesystem:read` | fs reads (optionally root-restricted) | "Read files on your computer" |
| `filesystem:write` | fs writes | "Write files on your computer" |
| `network` | `network.fetch`, `network.fetchStream` | "Make network requests" |
| `process:spawn` | `pty.*` | "Run terminal processes" |
| `clipboard:read` | clipboard read | "Read your clipboard" |
| `clipboard:write` | clipboard write | "Write your clipboard" |
| `notification` | `notify.show` | "Show notifications" |
| `secrets:read` | `secrets.*` (flag, stronger than grant) | "Access the secrets vault" |
| `ai:invoke` | `ai.*` registration/invocation | "Expose or call AI tools" |
| `mcp:connect` | `mcp.*` | "Connect to MCP servers" |
| `system:open` | `system.reveal`, `system.openUrl` | "Open files/URLs in system apps" |

## Rules

1. Declared in the manifest, approved at install, re-approved when an
   update drifts (tested).
2. Unknown permission names fail manifest validation.
3. Filesystem paths are canonicalized (lexical + symlink resolution)
   before scope matching — `..` and symlink escapes are rejected.
4. High-risk AI tools additionally require explicit user confirmation
   per invocation (spec §54).
