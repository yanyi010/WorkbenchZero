# Data safety in Workbench Zero

Workbench Zero is a personal workbench: it must be safe to point it at a
directory you care about and leave it running for months. This document is
the contract we test against.

## The three-tier data model

| Tier | Examples | Guarantee |
| --- | --- | --- |
| **User content** | memos, tasks, stickies, documents (plain files in the workspace root) | Atomic writes; never deleted by the app; a corrupt file is quarantined aside, never overwritten |
| **Canonical state** | `settings.json`, `workspaces.json`, `plugins.json`, `layout.json`, secrets fallback, MCP config | Atomic + fsynced writes; interrupted writes heal automatically; unparseable files are quarantined to `*.corrupt-<timestamp>`, never silently reset |
| **Derived state** | `index.sqlite` (search index, artifacts table) | Rebuildable by definition; WAL + integrity-checked on open; a corrupt index is backed up and rebuilt automatically |

## Atomic writes

Every state write goes through a single primitive
(`crates/common`): write to a unique `*.tmp-<uuid>` sibling → flush + fsync
→ rename over the target → fsync the parent directory. A crash at any point
leaves either the old content or the new content, never a torn file. The
residual tmp file of an interrupted write is automatically picked up as a
recovery candidate on the next read.

`fs.appendFile` (plugin-facing) fsyncs too, so append-only logs survive
power loss.

## Corrupt-state recovery: quarantine, never discard

If a canonical file cannot be parsed, it is moved to
`<name>.corrupt-<timestamp>` next to the original and a fresh default is
created. The user is told where the file went. The app **never** silently
treats an unreadable file as empty and later overwrites it — that failure
mode destroyed user data in earlier versions and is regression-tested.

## Automatic snapshots

`.workbench/backups/<timestamp>/` contains point-in-time snapshots of
everything under `.workbench/` that is not disposable: `workspace.json`,
`settings.json`, `layout.json`, `plugin-state/`, and a consistent,
compacted copy of the index (`VACUUM INTO`).

- Taken on workspace open/close and periodically while running when the
  last snapshot is older than `core.backup.intervalHours` (default 24).
- Rotation keeps the newest `core.backup.keep` snapshots (default 10).
- Restore (`workspace.restoreBackup`) verifies the snapshot manifest,
  refuses foreign or newer-than-supported snapshots, moves the current
  `.workbench` aside (never deletes it), and swaps the snapshot in with a
  directory rename.

Snapshots of a workspace live **inside the workspace** — they are plain
directories you can inspect, diff, and zip. They are a safety net for app
bugs and crashes, not backups against disk failure; back your machine up
separately.

## What snapshots deliberately do not cover

Your content files (memos, tasks, documents) are plain files in the
workspace root — the natural way to protect them is version control or
file-level sync (`git`, `rsync`, Syncthing, your OS backup). The app never
copies them; it only guarantees it won't corrupt or silently delete them.

## Sensitive data

Secrets live in the OS keychain when available and in an `0600` fallback
file otherwise; the fallback file is permission-locked *before* data lands
in it and is never silently overwritten on corruption. Secrets are never
written to logs, diagnostics, or snapshots beyond the state tree location
they already occupy.
