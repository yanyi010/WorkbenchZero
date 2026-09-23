/**
 * Plugin host: manages plugin iframes and the trusted main-frame bridge
 * (ADR-0004). Plugin iframes live on the `wzp://` origin with no direct
 * kernel channel; every RPC is stamped with the caller's plugin id here,
 * in the trusted frame — plugins cannot spoof each other.
 */
import type {
  CommandDef,
  PluginManifest,
  PushMessage,
  PushTopic,
  RpcResponse,
} from '@workbench-zero/protocol';
import { Methods } from '@workbench-zero/protocol';
import { rpc } from './kernel';

interface FrameEntry {
  iframe: HTMLIFrameElement;
  pluginId: string;
  surface: string;
  ready: boolean;
  /** Pending manifest/… handshakes waiting for wz-ready. */
  initSent: boolean;
  /** Last attach time (LRU eviction of parked view frames). */
  lastTouched: number;
}

/** Bridge API versions this host can serve (manifest `apiVersion`). */
const SUPPORTED_API_VERSIONS = new Set(['1']);
/** Parked (hidden, kept-alive) view frames beyond this count get evicted,
 * least-recently-used first. Bounds renderer memory for long sessions. */
const MAX_PARKED_VIEW_FRAMES = 6;

interface PendingCommand {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

const COMMAND_TIMEOUT_MS = 30_000;

class PluginHost {
  private frames = new Map<string, FrameEntry>();
  /** iframe.contentWindow → frame key. */
  private windowIndex = new Map<Window, string>();
  private pendingCommands = new Map<number, PendingCommand>();
  private nextCommandId = 1;
  private manifestCache = new Map<string, PluginManifest>();
  /** In-flight logic-frame activations — concurrent ensure calls must
   * wait on the same frame instead of double-activating a plugin. */
  private logicActivations = new Map<string, Promise<void>>();
  /** Wired by the store: surfaced plugin-level failures. */
  onPluginError: ((pluginId: string, message: string) => void) | null = null;
  private started = false;

  start() {
    if (this.started) return;
    this.started = true;
    window.addEventListener('message', (ev: MessageEvent) => {
      const key = ev.source ? this.windowIndex.get(ev.source as Window) : undefined;
      if (!key) return;
      const frame = this.frames.get(key);
      if (!frame) return;
      this.handlePluginMessage(frame, ev.data);
    });
  }

  /** Entry document for a plugin (manifest `entry`, default entry.html). */
  private entryFor(pluginId: string): string {
    return this.manifestCache.get(pluginId)?.entry || 'entry.html';
  }

  private frameSrc(pluginId: string, surface: string): string {
    const entry = this.entryFor(pluginId);
    return `wzp://${pluginId}/${entry}?surface=${encodeURIComponent(surface)}&plugin=${encodeURIComponent(pluginId)}`;
  }

  cacheManifest(manifest: PluginManifest) {
    this.manifestCache.set(manifest.id, manifest);
  }

  private createFrame(pluginId: string, surface: string, hidden: boolean): HTMLIFrameElement {
    const iframe = document.createElement('iframe');
    const key = `${pluginId}::${surface}`;
    iframe.setAttribute('data-wz-frame', key);
    iframe.setAttribute('title', `${pluginId} (${surface})`);
    if (hidden) {
      iframe.style.display = 'none';
    } else {
      iframe.style.border = '0';
      iframe.style.width = '100%';
      iframe.style.height = '100%';
    }
    // Cross-origin isolation: sandbox without allow-same-origin so the
    // frame cannot reach the parent DOM or localStorage. `allow-modals`
    // keeps plugin confirm/alert dialogs functional (a headless plugin
    // UX needs *some* user question channel).
    iframe.setAttribute('sandbox', 'allow-scripts allow-modals');
    this.frames.set(key, {
      iframe,
      pluginId,
      surface,
      ready: false,
      initSent: false,
      lastTouched: Date.now(),
    });
    // The contentWindow is only addressable after insertion; index lazily
    // on the first message via the windowIndex built at load time.
    iframe.addEventListener('load', () => {
      if (iframe.contentWindow) this.windowIndex.set(iframe.contentWindow, key);
    });
    iframe.src = this.frameSrc(pluginId, surface);
    return iframe;
  }

  /** Attach a view frame (or un-park its kept-alive instance). */
  attachViewFrame(container: HTMLElement, pluginId: string, viewId: string): void {
    const surface = `view:${viewId}`;
    const key = `${pluginId}::${surface}`;
    const existing = this.frames.get(key);
    if (existing) {
      existing.lastTouched = Date.now();
      existing.iframe.style.display = '';
      existing.iframe.style.border = '0';
      existing.iframe.style.width = '100%';
      existing.iframe.style.height = '100%';
      if (existing.iframe.parentElement !== container) {
        container.replaceChildren(existing.iframe);
      }
      return;
    }
    const frame = this.createFrame(pluginId, surface, false);
    container.replaceChildren(frame);
    this.evictParkedFrames();
  }

  /** Detach a view frame by *parking* it: the iframe is hidden but kept
   * alive so switching tabs never reloads plugin state. Older parked
   * frames are evicted past MAX_PARKED_VIEW_FRAMES. */
  detachViewFrame(pluginId: string, viewId: string): void {
    const key = `${pluginId}::view:${viewId}`;
    const frame = this.frames.get(key);
    if (!frame) return;
    const parking = document.getElementById('wz-logic-frames');
    frame.iframe.style.display = 'none';
    if (parking && frame.iframe.parentElement !== parking) {
      parking.appendChild(frame.iframe);
    }
    this.evictParkedFrames();
  }

  /** Permanently destroy a frame (state change, eviction). */
  private destroyFrame(key: string): void {
    const frame = this.frames.get(key);
    if (!frame) return;
    frame.iframe.remove();
    if (frame.iframe.contentWindow) this.windowIndex.delete(frame.iframe.contentWindow);
    this.frames.delete(key);
  }

  private evictParkedFrames(): void {
    const parked = [...this.frames.entries()]
      .filter(
        ([key, f]) =>
          f.surface.startsWith('view:') &&
          key !== undefined &&
          f.iframe.parentElement?.id === 'wz-logic-frames',
      )
      .sort((a, b) => a[1].lastTouched - b[1].lastTouched);
    for (const [key] of parked.slice(0, Math.max(0, parked.length - MAX_PARKED_VIEW_FRAMES))) {
      this.destroyFrame(key);
    }
  }

  /** Ensure the hidden logic frame for a plugin exists (activates it).
   * Concurrent calls share one activation — two parallel activations of
   * the same plugin would double-execute plugin code. */
  ensureLogicFrame(pluginId: string): Promise<void> {
    const key = `${pluginId}::logic`;
    if (this.frames.get(key)?.ready) return Promise.resolve();
    const inFlight = this.logicActivations.get(pluginId);
    if (inFlight) return inFlight;
    const activation = (async () => {
      if (this.frames.get(key)?.ready) return;
      const host = document.getElementById('wz-logic-frames');
      if (!host) throw new Error('logic frame container missing');
      const frame = this.createFrame(pluginId, 'logic', true);
      host.appendChild(frame);
      await this.waitReady(pluginId, 'logic');
    })().finally(() => this.logicActivations.delete(pluginId));
    this.logicActivations.set(pluginId, activation);
    return activation;
  }

  private waitReady(pluginId: string, surface: string, timeoutMs = 15_000): Promise<void> {
    const key = `${pluginId}::${surface}`;
    const existing = this.frames.get(key);
    if (existing?.ready) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const started = Date.now();
      const poll = () => {
        const f = this.frames.get(key);
        if (f?.ready) return resolve();
        if (Date.now() - started > timeoutMs) {
          return reject(new Error(`plugin ${pluginId} (${surface}) did not initialize`));
        }
        setTimeout(poll, 50);
      };
      poll();
    });
  }

  /** Handle a message from a known plugin frame (trusted routing). */
  private handlePluginMessage(frame: FrameEntry, msg: unknown) {
    if (!msg || typeof msg !== 'object') return;
    const { type } = msg as { type: string };
    const iframeWindow = frame.iframe.contentWindow;

    const post = (payload: unknown) => {
      iframeWindow?.postMessage(payload, '*');
    };

    switch (type) {
      case 'wz-ready': {
        frame.ready = true;
        frame.initSent = true;
        post({
          type: 'wz-init',
          pluginId: frame.pluginId,
          surface: frame.surface,
          apiVersion: '1',
        });
        break;
      }
      case 'wz-manifest-request': {
        const serve = (manifest: PluginManifest | null) => {
          if (!manifest) {
            post({ type: 'wz-manifest', manifest: null });
            return;
          }
          // apiVersion handshake (spec §11): a plugin built for an
          // unsupported host API must not run — it would fail in
          // unpredictable partial ways.
          const major = String(manifest.apiVersion).split('.')[0];
          if (!SUPPORTED_API_VERSIONS.has(major)) {
            this.onPluginError?.(
              frame.pluginId,
              `requires bridge API ${manifest.apiVersion} (supported: 1.x)`,
            );
            post({ type: 'wz-manifest', manifest: null });
            return;
          }
          this.manifestCache.set(manifest.id, manifest);
          post({ type: 'wz-manifest', manifest });
        };
        const cached = this.manifestCache.get(frame.pluginId);
        if (cached) serve(cached);
        else {
          void rpc<{ manifest: PluginManifest }>(Methods.plugins.get, { id: frame.pluginId })
            .then((info) => serve(info.manifest))
            .catch(() => serve(null));
        }
        break;
      }
      case 'wz-rpc': {
        const { id, method, params } = msg as {
          id: number;
          method: string;
          params: Record<string, unknown>;
        };
        // Stamp the caller identity in the trusted frame. The plugin
        // cannot override `__plugin` — unknown extra fields could be
        // dropped, so we explicitly inject it.
        const stamped = { ...params, __plugin: frame.pluginId };
        void rpc(method, stamped)
          .then((result) => post({ type: 'wz-rpc-result', id, ok: true, result }))
          .catch((err: { code?: string; message?: string }) =>
            post({
              type: 'wz-rpc-result',
              id,
              ok: false,
              error: {
                code: err?.code ?? 'kernel/error',
                message: err?.message ?? 'kernel error',
              },
            }),
          );
        break;
      }
      case 'wz-command-result': {
        const { requestId, ok, result, error } = msg as {
          requestId: number;
          ok: boolean;
          result?: unknown;
          error?: string;
        };
        const pending = this.pendingCommands.get(requestId);
        if (!pending) return;
        clearTimeout(pending.timer);
        this.pendingCommands.delete(requestId);
        if (ok) pending.resolve(result);
        else pending.reject(new Error(error ?? 'command failed'));
        break;
      }
    }
  }

  /** Send a command invocation to a plugin's logic frame and await ack. */
  async sendCommand(pluginId: string, commandId: string, args?: string): Promise<unknown> {
    await this.ensureLogicFrame(pluginId);
    const frame = this.frames.get(`${pluginId}::logic`);
    if (!frame?.iframe.contentWindow) throw new Error(`logic frame for ${pluginId} missing`);
    const requestId = this.nextCommandId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingCommands.delete(requestId);
        reject(new Error(`plugin ${pluginId} did not answer command ${commandId}`));
      }, COMMAND_TIMEOUT_MS);
      this.pendingCommands.set(requestId, { resolve, reject, timer });
      frame.iframe.contentWindow!.postMessage(
        { type: 'wz-command', requestId, id: commandId, args },
        '*',
      );
    });
  }

  /** Kernel push routing: with `plugin` → that plugin's frames only;
   * without an owner (e.g. `mcp-status`) → every frame, which filter by
   * topic/sessionId themselves. Unicast keeps pty/net streams private. */
  routePush(msg: PushMessage) {
    for (const frame of this.frames.values()) {
      if (msg.plugin && frame.pluginId !== msg.plugin) continue;
      frame.iframe.contentWindow?.postMessage(
        { type: 'wz-push', topic: msg.topic, data: msg.data },
        '*',
      );
    }
  }

  /** Broadcast an event to all frames; plugins filter by name. */
  broadcastEvent(event: { name: string; data: unknown }) {
    for (const frame of this.frames.values()) {
      frame.iframe.contentWindow?.postMessage(
        { type: 'wz-push', topic: 'event', data: event },
        '*',
      );
    }
  }

  /** A plugin's state changed: drop its frames if it went away. */
  onPluginStateChanged(pluginId: string) {
    for (const [key, frame] of [...this.frames.entries()]) {
      if (frame.pluginId !== pluginId) continue;
      this.destroyFrame(key);
    }
    this.logicActivations.delete(pluginId);
    this.manifestCache.delete(pluginId);
  }
}

export const pluginHost = new PluginHost();

/** Collect the enabled plugins that should auto-activate at startup. */
export function startupActivationPlugins(
  plugins: { manifest: PluginManifest; state: string }[],
): string[] {
  const out: string[] = [];
  for (const p of plugins) {
    if (p.state !== 'enabled' && p.state !== 'active') continue;
    const events = p.manifest.activationEvents ?? [];
    if (events.includes('onStartup') || events.includes('onWorkspaceOpen')) {
      out.push(p.manifest.id);
    }
  }
  return out;
}

export type { CommandDef, PushTopic, RpcResponse };
