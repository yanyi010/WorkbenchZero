# ADR-0002: Plugin sandbox = cross-origin `edp://` iframes

- Status: Accepted
- Date: 2026-09-23
- Deciders: EigenDesk core

## Context

Spec §36 requires plugins to be isolated from the host and from each
other: no shared DOM, no ambient filesystem access, crash containment.
Spec §50 (WASM runtime) is a later milestone; v0.1 ships frontend-only
plugins. We need isolation **now** without blocking a future WASM
backend.

## Decision

Each plugin surface (view, widget, hidden logic frame) runs in an
`<iframe>` loaded from the synthetic origin:

```
edp://<pluginId>/<entry>?surface=<surface>&plugin=<pluginId>
```

- The Tauri shell serves `edp://` via a custom protocol handler backed
  by the on-disk plugin package directory.
- Frames get `sandbox="allow-scripts"` **without** `allow-same-origin`:
  each `edp://<pluginId>` origin is distinct, so the browser engine
  itself enforces plugin↔plugin and plugin↔host isolation. No
  `postMessage` target origin check is needed for spoofing *identity*,
  because the main frame stamps identity (ADR-0004).
- All capability access goes through the bridge; the frame has zero
  ambient authority beyond its manifest grants.

## Alternatives considered

- **Web Workers**: no DOM; widgets/views need layout.
- **Separate WebViews per plugin**: heavy (one WebKit process per
  plugin), poor widget density.
- **WASM only**: blocks v0.1 frontend plugins; revisit for compute-heavy
  or zero-DOM extensions.

## Consequences

- One WebKit process hosts all frames; a plugin's JS exception cannot
  take down the shell.
- The kernel must treat every bridge call as untrusted input (params
  size caps, identity stamping).
- Future WASM runtime slots in as another *surface kind* without
  changing the permission or bridge model.
