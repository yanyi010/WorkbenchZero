/**
 * @workbench-zero/plugin-sdk — the public API plugins code against (spec §30).
 *
 * A plugin runs inside a sandboxed `wzp://` iframe (spec §36) with no direct
 * kernel channel. Every call travels over `postMessage` to the trusted main
 * frame, which stamps the caller's plugin id before forwarding to the
 * kernel — plugins cannot spoof another plugin's identity.
 *
 * Every wire shape here is audited against `crates/kernel/src/rpc.rs`.
 */

import type {
  AiToolCallPush,
  HostBridgeCommand,
  ArtifactRecord,
  CommandContribution,
  HostBridgeMessage,
  IndexDocument,
  McpToolInfo,
  NetPush,
  NotificationAction,
  PluginManifest,
  PtySessionInfo,
  PushTopic,
  ReadDirResult,
  ReadFileResult,
  StatInfo,
} from '@workbench-zero/protocol';
import { KernelRpcError } from '@workbench-zero/protocol';

// ---------------------------------------------------------------------------
// base64 helpers (kernel PTY payload encoding is RFC 4648)
// ---------------------------------------------------------------------------

function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

function utf8ToBase64(text: string): string {
  return bytesToBase64(new TextEncoder().encode(text));
}

function base64ToUtf8(b64: string): string {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

// ---------------------------------------------------------------------------
// Bridge transport
// ---------------------------------------------------------------------------

interface Pending {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
}

class Bridge {
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private initResolve: (() => void) | null = null;
  private initDone: Promise<void>;
  private pushHandlers = new Set<(topic: PushTopic, data: unknown) => void>();
  private manifestResolve: ((m: PluginManifest) => void) | null = null;
  commandHandlers = new Set<(msg: HostBridgeCommand) => void>();

  private pluginIdField: string;
  private surfaceField: string;

  /** Immutable after `wz-init` (query string is the fallback source). */
  get pluginId(): string {
    return this.pluginIdField;
  }

  get surface(): string {
    return this.surfaceField;
  }

  constructor() {
    const params = new URLSearchParams(window.location.search);
    this.surfaceField = params.get('surface') ?? 'logic';
    this.pluginIdField = params.get('plugin') ?? '';

    this.initDone = new Promise<void>((resolve) => {
      this.initResolve = resolve;
    });

    window.addEventListener('message', (ev: MessageEvent) => {
      if (ev.source !== window.parent) return;
      const msg = ev.data as HostBridgeMessage;
      if (!msg || typeof msg !== 'object') return;
      switch (msg.type) {
        case 'wz-init':
          if (typeof msg.pluginId === 'string' && msg.pluginId) {
            this.pluginIdField = msg.pluginId;
          }
          if (typeof msg.surface === 'string' && msg.surface) {
            this.surfaceField = msg.surface;
          }
          this.initResolve?.();
          this.initResolve = null;
          break;
        case 'wz-rpc-result': {
          const p = this.pending.get(msg.id);
          if (!p) return;
          this.pending.delete(msg.id);
          if (msg.ok) p.resolve(msg.result);
          else p.reject(new KernelRpcError(msg.error.code, msg.error.message));
          break;
        }
        case 'wz-push':
          for (const fn of this.pushHandlers) fn(msg.topic, msg.data);
          break;
        case 'wz-manifest':
          this.manifestResolve?.(msg.manifest);
          break;
        case 'wz-command':
          this.commandHandlers.forEach((fn) => fn(msg));
          break;
      }
    });

    window.parent.postMessage({ type: 'wz-ready' }, '*');
  }

  waitReady(): Promise<void> {
    return this.initDone;
  }

  onPush(fn: (topic: PushTopic, data: unknown) => void): () => void {
    this.pushHandlers.add(fn);
    return () => this.pushHandlers.delete(fn);
  }

  requestManifest(): Promise<PluginManifest> {
    return new Promise<PluginManifest>((resolve) => {
      this.manifestResolve = resolve;
      window.parent.postMessage({ type: 'wz-manifest-request' }, '*');
    });
  }

  log(level: 'debug' | 'info' | 'warn' | 'error', message: string): void {
    void this.call('plugin.log', { level, message }).catch(() => {});
  }

  async call<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    await this.initDone;
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      });
      window.parent.postMessage({ type: 'wz-rpc', id, method, params }, '*');
      // Safety net: a dangling promise would freeze the plugin silently
      // (spec §37 — crash isolation must be observable).
      setTimeout(() => {
        if (this.pending.delete(id)) {
          reject(new KernelRpcError('kernel/timeout', `no response for ${method}`));
        }
      }, 60_000);
    });
  }
}

const bridge = new Bridge();

// ---------------------------------------------------------------------------
// Network stream dispatcher (topic `net`) — subscribes before the RPC
// returns so chunks that race ahead of the response are buffered and
// replayed, never dropped.
// ---------------------------------------------------------------------------

interface NetStreamState {
  onChunk?: (delta: string, event: string) => void;
  onEnd?: () => void;
  onError?: (message: string) => void;
  buffered: string[];
  done: boolean;
}

const netStreams = new Map<string, NetStreamState>();
const MAX_BUFFERED_CHUNKS = 2000;

bridge.onPush((topic, data) => {
  if (topic !== 'net') return;
  const msg = data as NetPush;
  const payload = msg?.payload;
  if (!payload?.streamId) return;
  let state = netStreams.get(payload.streamId);
  if (!state) {
    state = { buffered: [], done: false };
    netStreams.set(payload.streamId, state);
  }
  switch (msg.kind) {
    case 'net-chunk': {
      const delta = payload.data ?? '';
      if (state.onChunk) {
        state.onChunk(delta, payload.event ?? '');
      } else if (state.buffered.length < MAX_BUFFERED_CHUNKS) {
        state.buffered.push(delta);
      }
      break;
    }
    case 'net-end':
      state.done = true;
      state.onEnd?.();
      netStreams.delete(payload.streamId);
      break;
    case 'net-error':
      state.done = true;
      state.onError?.(payload.message ?? 'network error');
      netStreams.delete(payload.streamId);
      break;
    case 'net-abort':
      state.done = true;
      netStreams.delete(payload.streamId);
      break;
  }
});

// ---------------------------------------------------------------------------
// Plugin context API
// ---------------------------------------------------------------------------

export interface ArtifactUpsert {
  uri: string;
  type: string;
  title: string;
  metadata?: Record<string, unknown>;
}

export interface PluginContext {
  /** Immutable plugin id (`publisher.name`). */
  readonly pluginId: string;
  /** Surface this instance renders on: `logic`, `view:<id>` or `widget:<id>`. */
  readonly surface: string;
  /** The plugin's own manifest (resolved before `activate` runs). */
  readonly manifest: PluginManifest;

  /** Append to the plugin log (visible in Plugin Store → Logs). */
  log(level: 'debug' | 'info' | 'warn' | 'error', message: string): void;

  commands: {
    register(cmd: CommandContribution): Promise<void>;
    unregister(id: string): Promise<void>;
    /**
     * Handle a command invocation routed from the shell (palette, Quick
     * Capture, keybindings). Return value is passed back to the shell;
     * thrown errors surface as command-failed toasts.
     */
    onCommand(fn: (id: string, args: string | undefined) => unknown): () => void;
  };
  events: {
    emit(name: string, data?: unknown): Promise<void>;
    on(name: string, fn: (data: unknown) => void): () => void;
  };
  storage: {
    get(key: string): Promise<unknown>;
    set(key: string, value: unknown): Promise<void>;
    delete(key: string): Promise<boolean>;
    keys(): Promise<string[]>;
  };
  session: {
    get(key: string): Promise<unknown>;
    set(key: string, value: unknown): Promise<void>;
    delete(key: string): Promise<boolean>;
    keys(): Promise<string[]>;
  };
  fs: {
    readFile(path: string, encoding?: 'utf8' | 'base64'): Promise<ReadFileResult>;
    writeFile(path: string, content: string, encoding?: 'utf8' | 'base64'): Promise<void>;
    appendFile(path: string, content: string): Promise<void>;
    readDir(path: string): Promise<ReadDirResult>;
    stat(path: string): Promise<StatInfo>;
    mkdir(path: string): Promise<void>;
    delete(path: string, recursive?: boolean): Promise<void>;
    copy(from: string, to: string): Promise<void>;
    move(from: string, to: string): Promise<void>;
  };
  network: {
    fetch(
      url: string,
      init?: { method?: string; headers?: Record<string, string>; body?: string; timeoutMs?: number },
    ): Promise<{ status: number; headers: Record<string, string>; body: string }>;
    /**
     * Streaming fetch (SSE-friendly). `onChunk` receives decoded text
     * deltas; `onEnd`/`onError` fire exactly once. The returned promise
     * resolves with the stream id once the request has been dispatched.
     */
    fetchStream(
      url: string,
      init: { method?: string; headers?: Record<string, string>; body?: string },
      handlers: {
        onChunk: (delta: string, event: string) => void;
        onEnd?: () => void;
        onError?: (message: string) => void;
      },
    ): Promise<{ streamId: string }>;
    abort(streamId: string): Promise<void>;
  };
  secrets: {
    list(): Promise<string[]>;
    get(key: string): Promise<string | null>;
    set(key: string, value: string): Promise<void>;
    delete(key: string): Promise<boolean>;
  };
  pty: {
    create(opts: {
      cwd?: string;
      shell?: string;
      env?: Record<string, string>;
      cols?: number;
      rows?: number;
    }): Promise<PtySessionInfo>;
    write(id: string, data: string): Promise<void>;
    resize(id: string, cols: number, rows: number): Promise<void>;
    kill(id: string): Promise<void>;
    list(): Promise<PtySessionInfo[]>;
    /** Terminal output (base64-decoded to UTF-8 text). */
    onData(id: string, fn: (data: string) => void): () => void;
    onExit(id: string, fn: (info: { success: boolean }) => void): () => void;
  };
  notify: {
    show(n: { title: string; body?: string; actions?: NotificationAction[] }): Promise<void>;
  };
  artifacts: {
    upsert(records: ArtifactUpsert | ArtifactUpsert[]): Promise<{ count: number }>;
    remove(uri: string): Promise<{ removed: boolean }>;
    listRecent(limit?: number): Promise<ArtifactRecord[]>;
    listByType(type: string, limit?: number): Promise<ArtifactRecord[]>;
    markOpened(uri: string): Promise<void>;
  };
  search: {
    upsert(documents: IndexDocument | IndexDocument[]): Promise<{ count: number }>;
    removeByPlugin(): Promise<{ removed: number }>;
    query(query: string, limit?: number): Promise<{ results: unknown[]; tookMs: number }>;
  };
  workspace: {
    current(): Promise<{ id: string; name: string; root: string } | null>;
    reveal(path: string): Promise<void>;
  };
  settings: {
    /** Effective value: workspace → global → descriptor default → null. */
    get<T = unknown>(key: string): Promise<T | null>;
    /**
     * Write a setting. The kernel only lets plugins write keys under
     * their own id prefix (`<pluginId>.…`).
     */
    set(key: string, value: unknown): Promise<void>;
  };
  ai: {
    /** Register an AI tool the Quick Ask / chat agent can call. */
    registerTool(tool: {
      name: string;
      description: string;
      parameters: Record<string, unknown>;
      highRisk?: boolean;
    }): Promise<void>;
    /** Handle AI tool invocations routed to this plugin; returns a disposer. */
    onToolCall(
      fn: (name: string, args: Record<string, unknown>) => Promise<unknown>,
    ): () => void;
    /** All AI tools registered kernel-side (any plugin may ask). */
    listTools(): Promise<
      { name: string; description: string; parameters: unknown; highRisk: boolean; pluginId: string }[]
    >;
    /**
     * Invoke a registered tool (routed to its owning plugin by the
     * kernel). Resolves with `{ ok: true, result }` or rejects.
     */
    callTool(name: string, args: Record<string, unknown>): Promise<unknown>;
  };
  mcp: {
    listTools(): Promise<McpToolInfo[]>;
    callTool(server: string, tool: string, arguments_: Record<string, unknown>): Promise<unknown>;
  };
}

// ---------------------------------------------------------------------------
// definePlugin
// ---------------------------------------------------------------------------

export interface PluginDefinition {
  activate(ctx: PluginContext): void | Promise<void>;
  deactivate?(): void | Promise<void>;
}

let activeDefinition: PluginDefinition | null = null;
const disposers: (() => void)[] = [];
let resolvedManifest: PluginManifest | null = null;

export function definePlugin(def: PluginDefinition): void {
  activeDefinition = def;
  void boot();
}

async function boot(): Promise<void> {
  await bridge.waitReady();
  try {
    resolvedManifest ??= await bridge.requestManifest();
  } catch {
    bridge.log('warn', 'manifest handshake failed');
  }
  const ctx = buildContext();
  try {
    await activeDefinition?.activate(ctx);
    bridge.log('info', `activated on surface "${bridge.surface}"`);
  } catch (err) {
    bridge.log('error', `activation failed: ${String(err)}`);
    void bridge
      .call('plugins.reportFailure', {
        id: bridge.pluginId,
        reason: String(err),
      })
      .catch(() => {});
  }
}

function buildContext(): PluginContext {
  const ptyDataHandlers = new Map<string, Set<(data: string) => void>>();
  const ptyExitHandlers = new Map<string, Set<(info: { success: boolean }) => void>>();
  const toolCallHandlers = new Set<
    (name: string, args: Record<string, unknown>) => Promise<unknown>
  >();

  disposers.push(
    bridge.onPush((topic, data) => {
      if (topic === 'pty') {
        const msg = data as {
          sessionId?: string;
          kind?: string;
          data?: string;
          success?: boolean;
        };
        if (msg?.sessionId && msg.kind === 'exit') {
          for (const fn of ptyExitHandlers.get(msg.sessionId) ?? [])
            fn({ success: !!msg.success });
        } else if (msg?.sessionId && typeof msg.data === 'string') {
          let text: string;
          try {
            text = base64ToUtf8(msg.data);
          } catch {
            text = msg.data;
          }
          for (const fn of ptyDataHandlers.get(msg.sessionId) ?? []) fn(text);
        }
        return;
      }
      if (topic === 'plugin-push') {
        const msg = data as AiToolCallPush;
        if (msg?.kind !== 'ai-tool-call') return;
        for (const fn of toolCallHandlers) {
          void fn(msg.tool, msg.args ?? {})
            .then((result) =>
              bridge.call('ai.toolResult', { requestId: msg.requestId, ok: true, result }),
            )
            .catch((err) =>
              bridge.call('ai.toolResult', {
                requestId: msg.requestId,
                ok: false,
                error: String(err),
              }),
            );
          break; // first registered handler wins
        }
      }
    }),
  );

  const asArray = <T>(x: T | T[]): T[] => (Array.isArray(x) ? x : [x]);

  return {
    pluginId: bridge.pluginId,
    surface: bridge.surface,
    manifest: resolvedManifest as PluginManifest,
    log: (level, message) => bridge.log(level, message),
    commands: {
      register: (cmd) =>
        bridge.call('commands.register', { command: cmd }).then(() => undefined),
      unregister: (id) => bridge.call('commands.unregister', { id }).then(() => undefined),
      onCommand(fn) {
        const handler = (msg: HostBridgeCommand) => {
          void (async () => {
            try {
              const result = await fn(msg.id, msg.args);
              window.parent.postMessage(
                { type: 'wz-command-result', requestId: msg.requestId, ok: true, result },
                '*',
              );
            } catch (err) {
              window.parent.postMessage(
                {
                  type: 'wz-command-result',
                  requestId: msg.requestId,
                  ok: false,
                  error: String(err),
                },
                '*',
              );
            }
          })();
        };
        bridge.commandHandlers.add(handler);
        const off = () => bridge.commandHandlers.delete(handler);
        disposers.push(off);
        return off;
      },
    },
    events: {
      emit: (name, data) => bridge.call('events.emit', { name, data }).then(() => undefined),
      on(name, fn) {
        const off = bridge.onPush((topic, data) => {
          if (topic === 'event') {
            const msg = data as { name: string; data: unknown };
            if (msg?.name === name) fn(msg.data);
          }
        });
        disposers.push(off);
        return off;
      },
    },
    storage: {
      get: (key) => bridge.call('storage.get', { key }),
      set: (key, value) => bridge.call('storage.set', { key, value }).then(() => undefined),
      delete: (key) =>
        bridge.call<{ removed: boolean }>('storage.delete', { key }).then((r) => !!r?.removed),
      keys: () => bridge.call<string[]>('storage.keys', {}),
    },
    session: {
      get: (key) => bridge.call('session.get', { key }),
      set: (key, value) => bridge.call('session.set', { key, value }).then(() => undefined),
      delete: (key) =>
        bridge.call<{ removed: boolean }>('session.delete', { key }).then((r) => !!r?.removed),
      keys: () => bridge.call<string[]>('session.keys', {}),
    },
    fs: {
      readFile: (path, encoding = 'utf8') =>
        bridge.call<ReadFileResult>('fs.readFile', { path, encoding }),
      writeFile: (path, content, encoding = 'utf8') =>
        bridge.call('fs.writeFile', { path, content, encoding }).then(() => undefined),
      appendFile: (path, content) =>
        bridge.call('fs.appendFile', { path, content }).then(() => undefined),
      readDir: (path) => bridge.call<ReadDirResult>('fs.readDir', { path }),
      stat: (path) => bridge.call<StatInfo>('fs.stat', { path }),
      mkdir: (path) => bridge.call('fs.mkdir', { path }).then(() => undefined),
      delete: (path, recursive = false) =>
        bridge.call('fs.delete', { path, recursive }).then(() => undefined),
      copy: (from, to) => bridge.call('fs.copy', { from, to }).then(() => undefined),
      move: (from, to) => bridge.call('fs.move', { from, to }).then(() => undefined),
    },
    network: {
      fetch: (url, init = {}) =>
        bridge.call<{ status: number; headers: Record<string, string>; body: string }>(
          'network.fetch',
          { url, ...init },
        ),
      fetchStream: (url, init, handlers) => {
        // The `net` dispatcher (module scope) is already subscribed, so
        // chunks arriving before the RPC response are buffered on the
        // state entry created when they arrive.
        return bridge
          .call<{ streamId: string }>('network.fetchStream', { url, ...init })
          .then((res) => {
            const state = netStreams.get(res.streamId);
            if (state) {
              state.onChunk = handlers.onChunk;
              state.onEnd = handlers.onEnd;
              state.onError = handlers.onError;
              for (const delta of state.buffered.splice(0)) handlers.onChunk(delta, '');
              if (state.done) handlers.onEnd?.();
            }
            return res;
          });
      },
      abort: (streamId) =>
        bridge.call('network.abort', { streamId }).then(() => undefined),
    },
    secrets: {
      list: () => bridge.call<string[]>('secrets.list', {}),
      get: (key) => bridge.call<string | null>('secrets.get', { key }),
      set: (key, value) => bridge.call('secrets.set', { key, value }).then(() => undefined),
      delete: (key) =>
        bridge.call<{ removed: boolean }>('secrets.delete', { key }).then((r) => !!r?.removed),
    },
    pty: {
      create: (opts) => bridge.call<PtySessionInfo>('pty.create', opts),
      write: (id, data) =>
        bridge
          .call('pty.write', { sessionId: id, data: utf8ToBase64(data) })
          .then(() => undefined),
      resize: (id, cols, rows) =>
        bridge.call('pty.resize', { sessionId: id, cols, rows }).then(() => undefined),
      kill: (id) => bridge.call('pty.kill', { sessionId: id }).then(() => undefined),
      list: () => bridge.call<PtySessionInfo[]>('pty.list', {}),
      onData(id, fn) {
        let set = ptyDataHandlers.get(id);
        if (!set) ptyDataHandlers.set(id, (set = new Set()));
        set.add(fn);
        return () => set.delete(fn);
      },
      onExit(id, fn) {
        let set = ptyExitHandlers.get(id);
        if (!set) ptyExitHandlers.set(id, (set = new Set()));
        set.add(fn);
        return () => set.delete(fn);
      },
    },
    notify: {
      show: (n) => bridge.call('notify.show', n).then(() => undefined),
    },
    artifacts: {
      upsert: (records) => bridge.call('artifacts.upsert', { artifacts: asArray(records) }),
      remove: (uri) => bridge.call('artifacts.remove', { uri }),
      listRecent: (limit = 50) =>
        bridge.call<ArtifactRecord[]>('artifacts.listRecent', { limit }),
      listByType: (type, limit = 50) =>
        bridge.call<ArtifactRecord[]>('artifacts.listByType', { type, limit }),
      markOpened: (uri) => bridge.call('artifacts.markOpened', { uri }).then(() => undefined),
    },
    search: {
      upsert: (documents) => bridge.call('search.upsert', { documents: asArray(documents) }),
      removeByPlugin: () => bridge.call('search.removeByPlugin', {}),
      query: (query, limit = 50) => bridge.call('search.query', { query, limit }),
    },
    workspace: {
      current: () => bridge.call('workspace.current', {}),
      reveal: (path) => bridge.call('system.reveal', { path }).then(() => undefined),
    },
    settings: {
      get: async <T,>(key: string): Promise<T | null> => {
        const v = await bridge.call<unknown>('settings.get', { key });
        return (v === null || v === undefined ? null : v) as T | null;
      },
      set: (key, value) => bridge.call('settings.set', { key, value }).then(() => undefined),
    },
    ai: {
      registerTool: (tool) => bridge.call('ai.registerTool', { tool }).then(() => undefined),
      onToolCall(fn) {
        toolCallHandlers.add(fn);
        return () => toolCallHandlers.delete(fn);
      },
      listTools: () =>
        bridge.call<{ name: string; description: string; parameters: unknown; highRisk: boolean; pluginId: string }[]>(
          'ai.listTools',
          {},
        ),
      callTool: (name, args) => bridge.call('ai.callTool', { name, args }),
    },
    mcp: {
      listTools: () => bridge.call<McpToolInfo[]>('mcp.listTools', {}),
      callTool: (server, tool, arguments_) =>
        bridge.call('mcp.callTool', { server, tool, arguments: arguments_ }),
    },
  };
}

// ---------------------------------------------------------------------------
// Shutdown
// ---------------------------------------------------------------------------

window.addEventListener('beforeunload', () => {
  void shutdown();
});

async function shutdown(): Promise<void> {
  for (const off of disposers.splice(0)) {
    try {
      off();
    } catch {
      /* best effort */
    }
  }
  try {
    await activeDefinition?.deactivate?.();
  } catch {
    /* best effort */
  }
}

export { h, render } from './h';
export type { Child, Attrs } from './h';
export { KernelRpcError };
