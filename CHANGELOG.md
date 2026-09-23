# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-09-23

First production release. The v0.1 platform kernel has been hardened end to
end so Workbench Zero can hold long-lived personal data safely: durable
writes, corrupt-state recovery, automatic snapshots, explicit privilege
separation, and a tested plugin bridge.

### Added

- **Automatic workspace snapshots** (`.workbench/backups/<timestamp>/`):
  best-effort snapshot on workspace open/close and periodically while
  running (settings `core.backup.enabled`/`keep`/`intervalHours`), with
  rotation and a consistent index copy (`VACUUM INTO`). Restore keeps the
  pre-restore state aside instead of deleting it. New RPCs
  `workspace.backupNow` / `workspace.listBackups` / `workspace.restoreBackup`.
- **wz-common**: atomic write primitive (unique tmp file → fsync → rename →
  parent-dir fsync), resilient JSON loading (quarantine + heal from
  interrupted-write siblings), poison-tolerant lock helpers.
- Error boundaries around every shell surface (dashboard, settings, store,
  plugin views) so a surface crash never takes down the shell.
- Plugin view iframes stay alive across tab switches (LRU-bounded parking),
  preserving plugin state.
- `apiVersion` handshake between host and plugins; a plugin requiring an
  unknown bridge API is refused with a visible error.
- Confirmation sandbox: plugin `alert`/`confirm` dialogs now work
  (`allow-modals`).
- Kernel RPC integration test-suite covering the privilege contract
  (`crates/kernel/tests/rpc_security.rs`).
- Per-plugin state-store schema (`docs/architecture` and README) documents
  the durability model.

### Changed

- **Push protocol is canonical**: kernel, shell and SDK now agree on
  `{ seq, topic, plugin?, data }`. `AiToolCallPush.requestId` is a kernel
  -assigned unguessable UUID string.
- All canonical state files (settings, workspace registry, plugin state,
  secrets fallback, MCP config, layout, plugin-owned content via
  `fs.writeFile`) persist atomically with fsync; an interrupted write heals
  itself; a corrupt file is quarantined to `*.corrupt-<ts>` — never silently
  discarded or overwritten.
- `index.sqlite` opens with WAL + `synchronous=NORMAL` + busy timeout, runs
  `PRAGMA integrity_check`, and rebuilds automatically from a backup when
  corrupt (the index is derived data; nothing user-authored is lost).
- Shell restores the last workspace on launch (`core.startup.openLastWorkspace`)
  and its layout per workspace (`core.behavior.restoreLayout`), persisted
  debounced via `workspace.saveLayout`.
- Plugin lifecycle, workspace management and diagnostics RPCs are shell-only;
  `ai.callTool` requires the `ai:invoke` grant; secret writes require the new
  `secrets:write` permission; `fs.move` requires write permission on the
  source; `system.reveal` is filesystem-scope-checked.
- Plugin ZIP installs are capped (256 MiB, 4096 entries) and staging
  directories are cleaned up.
- Concurrent logic-frame activations of the same plugin are serialized.
- `fs.appendFile` fsyncs; event bus queues are bounded with drop accounting.
- ~120 poisoned-lock panic sites now degrade instead of crashing the app.

### Fixed

- SDK `fetchStream`: a stream ending before its RPC response no longer loses
  its terminal callback (replayed exactly once); orphan streams are capped.
- tasks plugin: a corrupt `tasks.json` is quarantined with a user-visible
  notice instead of being silently reset to an empty list.
- memo plugin: two memos created in the same second with the same title no
  longer overwrite each other.
- sticky plugin: card colors survive reload (quoted value in frontmatter was
  rejected by the parser).
- MCP server `workspace.files`: `..` and absolute subdirs can no longer
  escape the workspace.
- Kernel error messages never disclose the RPC token; token comparison is
  constant-time.

### Security

- See `SECURITY.md` for the model and reporting policy.

## [0.1.0] - 2025-xx-xx

Initial platform kernel: kernel/ADR-driven architecture, plugin sandbox via
sandboxed `wzp://` iframes, permission model, SDK, and bundled plugins
(memo, tasks, sticky, files, terminal, AI).

[1.0.0]: https://github.com/yanyi010/WorkbenchZero/releases/tag/v1.0.0
