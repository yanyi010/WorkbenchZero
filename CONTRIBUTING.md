# Contributing to Workbench Zero

Thanks for helping build a workbench worth owning. This project is small,
opinionated and heavily tested — this document is the shortest path to a
merged PR.

## Ground rules

1. **The kernel stays small.** End-user features belong in plugins, even ours.
   If your change adds a capability, ask first whether it is a *coordination*
   concern (kernel) or a *feature* (plugin).
2. **Deny-by-default.** New permissions require an ADR and tests proving both
   enforcement and rejection paths.
3. **No protocol drift.** The Rust dispatcher and the TypeScript mirror
   (`@workbench-zero/protocol`) must stay in lockstep; tests parse the Rust
   source to enforce this.
4. **Local-first means local.** No telemetry, no network calls outside
   user-configured AI endpoints, no data outside the workspace and XDG dirs.

## Workflow

```bash
git clone https://github.com/yanyi010/WorkbenchZero.git
cd WorkbenchZero
npm install
```

Develop on a feature branch off `main`, keep commits focused, and open a PR
with a description that explains *why*. Draft PRs early are welcome.

## Gates (all must pass, locally and in CI)

```bash
# TypeScript
npm run typecheck
npx eslint .
npx vitest run          # unit + full-shell E2E (jsdom)

# Rust
cargo fmt --all --check
cargo clippy --workspace --all-targets   # zero warnings
cargo test --workspace

# Full product build
npm run build           # plugins → registry → vite
```

- Tests must be **deterministic**: no wall-clock-derived identifiers, no
  shared scratch dirs, no sleeps without a condition.
- New plugin-facing APIs need SDK docs (`docs/plugin-api/`) and at least one
  example exercising them.
- Behavioral changes to the wire protocol need an ADR.

## Writing plugins

You usually don't need to touch this repo at all — plugins are loaded from
`~/.local/share/workbench-zero/plugins` or installed from
`.wzplugin.zip` archives. See the [README](README.md#write-a-plugin-in-30-seconds)
and [`wb` CLI](packages/devtools/src/wb.mjs).

## Reporting bugs

Open an issue with: OS, Workbench Zero version, steps to reproduce, and the
log tail from `~/.local/share/workbench-zero/logs/workbench-zero/`.
