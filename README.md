<div align="center">

<img src="docs/assets/logo.png" width="110" alt="Workbench Zero logo" />

# Workbench Zero

**Your workbench. From zero.**

A local-first personal workbench built entirely around plugins —\
durable enough to hold years of your data. AI included, if you want it.

[![Version](https://img.shields.io/badge/release-1.0.0-5b8cff)](https://github.com/yanyi010/WorkbenchZero/releases)
[![CI](https://github.com/yanyi010/WorkbenchZero/actions/workflows/ci.yml/badge.svg)](https://github.com/yanyi010/WorkbenchZero/actions/workflows/ci.yml)
[![Release build](https://github.com/yanyi010/WorkbenchZero/actions/workflows/release.yml/badge.svg)](https://github.com/yanyi010/WorkbenchZero/actions/workflows/release.yml)
![License](https://img.shields.io/badge/license-MIT-blue)
![Platform](https://img.shields.io/badge/platform-Linux-informational)
![Rust](https://img.shields.io/badge/Rust-1.85%2B-dea584)
![TypeScript](https://img.shields.io/badge/TypeScript-strict-3178c6)

<img src="docs/assets/hero.png" width="860" alt="Workbench Zero shell: workspace with memos, tasks, a sticky card and a terminal" />

[Why it exists](#why-it-exists) · [Quick start](#quick-start) · [Data safety](#data-safety) · [Plugins](#first-party-plugins) · [Write a plugin](#write-a-plugin-in-30-seconds) · [Architecture](#architecture) · [Security](#security-model) · [Docs](#documentation)

</div>

---

## Why it exists

Notes apps die, sync services change their terms, and "run this script" tools
scatter your work across dotfiles. Workbench Zero takes the other bet: a
**boring, inspectable core** — a directory of Markdown and JSON, coordinated by
a small Rust kernel — where every feature, including memo and AI chat, is a
sandboxed plugin you could replace tomorrow.

It's 1.0 because the contract below is kept, tested, and enforced in CI.

## Data safety

Your data lives in a plain folder you own. The kernel treats it like a
database would:

- **Atomic, fsynced writes** for every state file — a crash mid-write heals
  itself on next read.
- **Quarantine, never discard** — an unparseable state file is moved aside
  (`*.corrupt-<timestamp>`) with a visible notice, never silently reset.
- **Automatic snapshots** — `.workbench/backups/` keeps point-in-time copies
  of workspace metadata and plugin state (configurable, rotated, restorable
  with one command).
- **The derived index repairs itself** — `index.sqlite` is WAL-mode,
  integrity-checked, and rebuilt automatically if it ever goes bad.

The full contract is in [docs/data-safety](docs/data-safety/README.md) and
[ADR-0009](docs/adr/ADR-0009-durability-and-snapshots.md). Every guarantee
above has a regression test.

## First-party plugins

| Plugin | What it does | Data it owns |
| --- | --- | --- |
| `zero.memo` | Markdown memos with front matter, full-text search | `Memos/*.md` |
| `zero.tasks` | Task list with `!prio @project #tag ~due` quick syntax | `Tasks/tasks.json` |
| `zero.sticky` | Desktop stickies, convertible to memos and tasks | `Stickies/*.md` |
| `zero.files` | Workspace file tree with rename / move / delete | your files |
| `zero.terminal` | Real terminals (xterm.js) in tabs | — |
| `zero.ai` | Streaming chat, tool calls, save-as-memo | `AI/*.md` |

Three example plugins (`community.hello-plugin`, `community.pomodoro`,
`community.quickcalc`) double as templates and test fixtures.

## Quick start

> Pre-built `.deb` / `.AppImage` artifacts (with `SHA256SUMS.txt`) are
> attached to every [release](https://github.com/yanyi010/WorkbenchZero/releases).
> Linux for now (WebKitGTK); the bundler config is architecture-clean and a
> macOS/Windows matrix is planned.

From source (Node 22+, Rust 1.85+, `libwebkit2gtk-4.1-dev`):

```bash
git clone https://github.com/yanyi010/WorkbenchZero.git
cd WorkbenchZero
npm install
npm run build        # plugins → registry → vite
npm run dev          # or: cargo tauri dev
```

First launch asks for a workspace folder and a starter pack — that's the
whole onboarding. `Ctrl+K` opens the command palette, `Alt+Space` is quick
capture, `Ctrl+P` searches the workspace.

## Write a plugin in 30 seconds

```bash
npx wb plugin create my-first-plugin    # scaffold
node packages/devtools/src/wb.mjs plugin dev my-first-plugin    # hot-reload into the running app
npx wb plugin pack my-first-plugin      # → my-first-plugin.wzplugin.zip
```

A plugin is four files: `plugin.json` (manifest + declared permissions),
`entry.html`, `style.css`, and `src/main.ts`:

```ts
import { definePlugin } from '@workbench-zero/plugin-sdk'

definePlugin({
  activate(ctx) {
    ctx.commands.onCommand((id) => {
      if (id !== 'example.first.hello') return undefined
      void ctx.notify.show({ title: 'Hello', body: 'Hello from my first plugin!' })
      return 'ok'
    })
  },
})
```

The full API — views, settings, workspace files, streaming network, AI tools —
is documented in [docs/plugin-api](docs/plugin-api/README.md). Contribution
points and packaging: [docs/contribution-points](docs/contribution-points/README.md).

## Architecture

```
┌────────────────────────────────────────────────────────┐
│ Workbench Zero shell (TypeScript / React, Tauri webview)│
│   palette · quick capture · search · settings · welcome │
└──────────────▲─────────────────────────▲───────────────┘
       kernel_rpc(token, payload)   window.__kernelInbox(batch)
┌──────────────┴─────────────────────────┴───────────────┐
│ wz-kernel (Rust): settings · workspaces · commands ·    │
│ events · search · artifacts · secrets · plugin runtime  │
└──────────────▲─────────────────────────▲───────────────┘
   single JSON-RPC-ish dispatcher   batched push (seq)
┌──────────────┴─────────────────────────┴───────────────┐
│ plugin iframes (wzp://<id>/…, cross-origin sandbox)    │
│   zero.memo · zero.tasks · zero.files · zero.terminal … │
└─────────────────────────────────────────────────────────┘
```

One command (`kernel_rpc`), a first-caller token, an audited TypeScript
protocol mirror, and a monotonically-sequenced batched inbox — the whole wire
surface. Decisions are recorded in [ADRs](docs/adr/); start with
[ADR-0001](docs/adr/ADR-0001-rust-kernel-workspace.md).

## Security model

- Plugins are cross-origin iframes served from `wzp://` with a strict CSP
  (`connect-src 'none'`); the shell is unreachable from plugin DOM, and the
  kernel stamps the caller identity — plugins cannot spoof each other.
- Permissions are a closed, versioned set, deny-by-default, approved at
  install time; an upgrade that widens a permission requires re-approval
  ([docs/permissions](docs/permissions/README.md)).
- Shell-only RPCs cover plugin lifecycle, workspace management, and
  diagnostics export; AI invocation and secret writes have their own grants.
- Secrets live in the OS keychain, with a permission-locked (`0600`) file
  fallback.
- Path traversal is contained with canonical resolution — enforced in Rust,
  covered by tests.

Report vulnerabilities privately (see [SECURITY.md](SECURITY.md)).

## Stability guarantees (1.0)

Within a major version: the kernel RPC method set is append-only; the bridge
protocol between shell and plugins is versioned (`apiVersion: "1"`), and
plugins built for it keep working; the on-disk workspace format
(`schema_version` in `workspace.json`) only ever moves forward with explicit,
backed-up migrations. See [docs/plugin-api](docs/plugin-api/README.md#stability).

## Development

```bash
npm run typecheck && npx eslint . && npx vitest run   # TS gates
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                 # Rust gates
```

Every gate runs on every PR, plus MSRV (1.85), `cargo audit`, `npm audit`, a
deterministic plugin-build check, and CLI smoke tests
([CI](.github/workflows/ci.yml)). See [CONTRIBUTING.md](CONTRIBUTING.md) for
the full workflow.

## Documentation

- [Architecture guide](docs/architecture/README.md)
- [Plugin API reference](docs/plugin-api/README.md)
- [Contribution points](docs/contribution-points/README.md)
- [Permission model](docs/permissions/README.md)
- [Data safety contract](docs/data-safety/README.md)
- [ADRs 0001–0009](docs/adr/) — every structural decision, with context

## License

[MIT](LICENSE) © Workbench Zero contributors
