# ADR-0006: Local-first data layout — workspace owns product data

- Status: Accepted
- Date: 2026-09-23
- Deciders: EigenDesk core

## Context

Spec §1–4: local-first, user-owned data, no cloud dependency, easy
backup and inspection. Plugin state and product data have different
lifecycles: memos belong to the *user's workspace*, plugin scratch state
belongs to the *app install*.

## Decision

```
<workspace root>/            user-owned, plain formats
  Memos/*.md                 frontmatter + Markdown (memo plugin)
  Stickies/*.md              sticky plugin
  Tasks/tasks.json           tasks plugin
  .eigendesk/                workspace-scoped settings/index caches

~/.local/share/eigendesk/    app-owned (XDG data)
  plugins.json               install/enabled state
  plugins/<id>/              installed user plugins
  dev-plugins/<id>/          `wb plugin dev` hot-reload target
  registry/                  local catalog index
  plugin-state/<id>.json     quota'd plugin KV (64 MiB default)
  secrets-fallback.json      only when no OS keychain (ADR-0007)

~/.config/eigendesk/         global settings, workspace registry
~/.cache/eigendesk/          derived caches (search index shards)
~/.local/share/eigendesk/logs/  structured app log
```

Rules:

1. **Product data is plain files in the workspace** — Markdown, JSON.
   No opaque blobs a user cannot diff, grep, or take to another tool.
   A plugin that loses interest can be uninstalled without losing the
   memos.
2. Plugin KV storage is quota-capped and lives in app data; it is
   scratch, not a product-data substitute.
3. `.edplugin.zip` packages are deterministic (stored zip, fixed
   timestamps) so package hashes are meaningful (spec §90).

## Consequences

- Backing up a workspace = copying a directory.
- Workspace portability (another machine, another app) is a file copy
  away; app state never blocks it.
- Plugins that want product data must write through the filesystem
  capability into the workspace, inheriting its permissions.
