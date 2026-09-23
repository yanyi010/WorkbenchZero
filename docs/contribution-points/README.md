# Contribution points

What a plugin manifest can contribute (`contributes` in `plugin.json`).
All ids must be prefixed with the plugin id.

| point | effect |
|---|---|
| `commands[]` | palette entries; `takesArgs` for Quick Capture; `keybinding` default; `opensView` for declarative view opening |
| `views[]` | main-area views (`location: "main"`), openable as tabs |
| `widgets[]` | dashboard grid cells (`minWidth`/`minHeight`) |
| `captureProviders[]` | Quick Capture routes: `prefixes[]`, `priority`, optional `command` (else `__capture:<id>` is sent) |
| `searchProviders[]` | participates in universal search (index via `ctx.search.upsert`) |
| `settings[]` | typed descriptors shown in Settings → plugin tab |
| `statusItems[]` | status bar entries |
| `fileHandlers[]` | declarative `mimeType` handlers |
| `artifactTypes[]` | artifact kind metadata |

## Activation events

`onCommand:<id>`, `onView:<id>`, `onStartup`, `onWorkspaceOpen`,
`onEvent:<name>` (wildcards per spec §33, tested in `plugin-runtime`).

## Conventions

- Product data lives in the **workspace** as plain files (ADR-0006).
- Cross-plugin handoffs use **events by convention**
  (`memo.createFromText`, `task.createFromText`) — never direct plugin
  calls.
- Capture routing is core-owned; core must work with zero providers.
