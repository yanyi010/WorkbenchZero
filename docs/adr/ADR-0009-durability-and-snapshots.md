# ADR-0009: Durability primitives, quarantine recovery, and workspace snapshots

- Status: accepted (1.0.0)
- Date: 2026-09-23
- Supersedes: none; operationalizes ADR-0006 (local-first data layout)

## Context

Closing the gap from "v0.1 platform kernel" to a product that can hold
long-lived personal data surfaced a class of defects that are invisible
until the day they destroy data: torn writes after power loss, unparseable
state files silently reset to defaults, a corrupt derived index bricking a
workspace, and no point-in-time recovery at all.

## Decision

1. **One durability primitive per language tier, used everywhere.**
   Rust: `wz-common::atomic_write` (unique tmp sibling → flush+fsync →
   rename → parent-dir fsync). TS/plugins: route content writes through
   kernel `fs.writeFile`, which uses the same primitive. No crate may
   hand-roll its own "write settings to disk" path.

2. **Quarantine, never discard.** Unparseable canonical files are moved to
   `*.corrupt-<ts>` next to the original; a fresh default takes their place;
   the user is notified where their data went. This applies kernel-side
   (settings, registry, plugin state, secrets, MCP config) and plugin-side
   (`tasks.json`) alike.

3. **Derived state is rebuilt, canonical state is restored.**
   `index.sqlite` is derived by definition: it is WAL + integrity-checked
   and automatically rebuilt from a snapshot or from scratch. Canonical
   state is what snapshots exist for.

4. **Snapshots are plain directories inside the workspace.**
   `.workbench/backups/<ts>/` holds workspace metadata, settings, layout,
   plugin state and a consistent index copy (via `VACUUM INTO`, which also
   acts as an integrity gate). Rotation keeps N newest. Restore swaps
   `.workbench` by directory rename after moving the current state aside —
   no in-place overwrites, no deletion of the pre-restore state.

5. **Backups are best-effort, never a gate.** A failed snapshot logs,
   emits `backup.failed`, and retries at the next interval; it must never
   block opening or closing a workspace.

6. **Locks may not take the process down.** Poisoned lock recovery with a
   warning is the rule (wz-common traits); a dead plugin frame must not
   wedge the kernel.

## Consequences

- State writes are slightly slower (fsync), which is the correct trade for
  a data-holding product; hot paths (session KV) remain in-memory.
- Restore requires a quiet point — implemented as
  close-workspace → swap → reopen, which also guarantees no SQLite handle
  survives across the swap.
- Snapshots are not machine backups; docs/data-safety is explicit about
  what is and is not covered.
