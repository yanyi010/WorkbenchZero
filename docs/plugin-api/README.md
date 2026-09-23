# Plugin API

The public API plugins code against: `@workbench-zero/plugin-sdk`.

```ts
import { definePlugin, h, render } from '@workbench-zero/plugin-sdk';
import { renderMarkdown } from '@workbench-zero/plugin-sdk/markdown';

definePlugin({
  async activate(ctx) {
    // ctx.surface: 'logic' | 'view:<id>' | 'widget:<id>'
    ctx.commands.onCommand((id, args) => { … });
    ctx.events.on('memo.createFromText', (data) => { … });
    await ctx.fs.readFile(`${(await ctx.workspace.current())!.root}/x.md`);
    await ctx.search.upsert([{ uri, title, body }]);
    await ctx.notify.show({ title: 'Done' });
  },
  deactivate() { … },
});
```

## Context namespaces

| namespace | notes |
|---|---|
| `commands` | `onCommand(id, args)` routed from palette/capture; `register` for dynamic commands |
| `events` | `emit`/`on` — cross-plugin, loosely coupled by convention |
| `fs` | permission-gated; `workspace:read/write` scopes to the workspace root |
| `storage` / `session` | quota'd KV (persistent / per-app-run) |
| `network` | `fetch` and `fetchStream` (SSE-friendly, chunked pushes) |
| `pty` | PTY sessions; requires `process:spawn` |
| `notify` | desktop notifications with actions |
| `artifacts` | cross-plugin artifact index (open from palette/search) |
| `search` | FTS upsert/removeByPlugin/query |
| `secrets` | owner-scoped vault (ADR-0007) |
| `ai` | `registerTool`, `onToolCall`, `listTools`, `callTool` |
| `mcp` | MCP server tool calls |
| `settings` | read any key; write only `<pluginId>.*` |
| `workspace` | `current()`, `reveal()` |

## Bridge protocol

See ADR-0004 for the wire (`wz-ready`/`wz-init`/`wz-rpc`/
`wz-command`/`wz-push`). The SDK is the only sanctioned client; the
protocol is versioned via the manifest `apiVersion: "1"`.

## Testing plugins

- Pure logic goes in `src/*.ts` modules with vitest colocated
  (`*.test.ts`) — see `plugins/memo/src/frontmatter.test.ts`.
- The SDK itself has bridge integration tests using a FakeHost
  (`packages/plugin-sdk/src/index.test.ts`) — reuse that pattern for
  plugin-level integration tests.
