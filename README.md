# EigenDesk

A local-first, extensible personal workbench. Rust kernel, TypeScript
shell, sandboxed plugins — your memos, tasks, stickies, files, terminal
and AI assistant in one desktop app that never phones home.

```
┌─────────────────────────────────────────────────────────┐
│  TitleBar          ⌘K palette  Alt+Space capture        │
├────┬────────────────────────────────────────┬───────────┤
│ A  │  Dashboard / Memos / Tasks / Terminal  │  Status   │
│ c  │  (tabs, plugin views in edp:// frames) │  Bar      │
│ t  │                                        │           │
└────┴────────────────────────────────────────┴───────────┘
        ▲ plugin iframes (sandboxed, permission-gated)
        ▲ single kernel_rpc IPC (token-guarded)
┌─────────────────────────────────────────────────────────┐
│  Rust kernel: workspace · permissions · plugins · FTS   │
│  artifacts · events · PTY · net · MCP · secrets · AI    │
└─────────────────────────────────────────────────────────┘
```

## Highlights

- **Local-first** — product data is plain Markdown/JSON inside a
  workspace folder you own (ADR-0006). Backup = `cp -r`.
- **Real plugin sandbox** — plugins run on synthetic `edp://` origins
  with `sandbox="allow-scripts"`; every capability call is stamped and
  permission-checked in the trusted frame (ADR-0002/0004).
- **Closed permission set** with per-install approval and
  drift-reapproval on updates (ADR-0005).
- **Quick Capture** (`Alt+Space`) — prefix-routed (`/t` task, `/s`
  sticky, `?` ask, `=` calc); core never depends on a specific plugin.
- **Command palette** (`Ctrl+K`), universal search (`Ctrl+P`) over a
  cross-plugin FTS index.
- **First-party plugins**: Memo, Tasks, Sticky, Files, Terminal (PTY),
  Quick Ask (any OpenAI-compatible endpoint, keys in the OS keychain).
- **Developer CLI** `wb` — scaffold, build, hot-reload into the running
  app, deterministic `.edplugin.zip` packaging.

## Repository layout

```
apps/desktop/          Tauri 2 shell (Rust transport + React UI)
crates/                Rust kernel workspace (see ADR-0001)
packages/protocol/     TypeScript wire types + Methods (drift-tested)
packages/plugin-sdk/   Plugin API: definePlugin, bridge, h(), markdown
packages/ui-kit/       Design tokens + shared React components
packages/devtools/     wb CLI
plugins/               First-party plugins (memo, tasks, sticky, files,
                       terminal, ai)
examples/              hello-plugin, pomodoro, quickcalc
scripts/build.mjs      Plugin bundling + registry/packs generation
tests/e2e/             Full-shell E2E against a fake kernel
docs/                  ADRs, architecture, plugin API docs
```

## Quick start (development)

Requirements: Node ≥ 20, Rust stable, and on Linux
`webkit2gtk4.1-devel` (+ `libappindicator`/`librsvg` as your distro
requires for Tauri 2).

```bash
npm install

# everything: plugin bundles → registry/packs → vite build
npm run build

# run the app in dev mode (vite + tauri)
npm run dev

# strict gates (all must be green before merge)
npm run typecheck       # tsc across packages, plugins, examples, tests
npm run lint            # eslint (type-checked rules), 0 errors
npm test                # 59 vitest tests incl. E2E shell boot
cargo fmt --all --check
cargo clippy --workspace --all-targets   # zero warnings
cargo test --workspace  # 58 Rust tests
```

### Writing a plugin

```bash
npm run wb -- plugin create my-plugin
cd my-plugin
npm run wb -- plugin dev .     # hot-reloads into the running app
npm run wb -- plugin pack .    # → my-plugin-0.1.0.edplugin.zip
```

A minimal plugin is one command in under 100 lines (see
`examples/hello-plugin`). See `docs/plugin-api` and
`docs/contribution-points`.

## Security model (short version)

1. One privileged IPC (`kernel_rpc`), claimed by the main frame at
   startup via a bootstrap token (ADR-0003).
2. Plugin frames cannot call it — they `postMessage` the trusted frame,
   which stamps their identity (ADR-0004).
3. The kernel enforces the closed permission set on every call;
   filesystem paths are canonicalized before scope checks (ADR-0005).
4. Secrets live in the OS keychain; headless fallback is `0600`-file
   with a surfaced warning, never plain config (ADR-0007).

Full reasoning: `docs/adr/`.

## License

TBD — all rights reserved for now.
