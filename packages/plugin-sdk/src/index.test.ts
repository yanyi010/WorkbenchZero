/**
 * Bridge integration tests: a fake host window (window.parent) that
 * speaks the wz-* protocol, driving the real SDK module in jsdom.
 *
 * Each test re-imports the SDK (`vi.resetModules()` + dynamic import) so
 * module-level bridge state is fresh. The fake host dispatches messages
 * synchronously with `source: window` because jsdom's own postMessage
 * does not populate `source`.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

type HostMessage = { type: string } & Record<string, unknown>;

type HostListener = (msg: HostMessage) => void;

/** Window listeners installed by FakeHosts, removed between tests. */
const cleanups: (() => void)[] = [];

/** A controllable stand-in for the trusted main frame. */
class FakeHost {
  received: HostMessage[] = [];
  private listeners = new Set<HostListener>();

  install() {
    // The SDK posts to window.parent; make the window its own parent.
    Object.defineProperty(window, 'parent', { value: window, configurable: true });

    // eslint-disable-next-line @typescript-eslint/no-this-alias -- FakeHost methods need a stable self reference
    const self = this;
    // Intercept plugin → host traffic synchronously.
    window.postMessage = ((msg: unknown) => {
      self.received.push(msg as HostMessage);
      window.dispatchEvent(new MessageEvent('message', { data: msg, source: window }));
    }) as typeof window.postMessage;

    // Host brain: route every plugin-originated message to listeners.
    const listener = (ev: MessageEvent) => {
      if (ev.source !== window) return;
      const msg = ev.data as HostMessage;
      if (!msg || typeof msg !== 'object') return;
      for (const fn of [...self.listeners]) fn(msg);
    };
    window.addEventListener('message', listener);
    cleanups.push(() => window.removeEventListener('message', listener));
  }

  /** Send a host → plugin message as if from window.parent. */
  send(msg: unknown) {
    window.dispatchEvent(new MessageEvent('message', { data: msg, source: window }));
  }

  on(fn: HostListener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  init(pluginId = 'test.demo', surface = 'view:demo.main') {
    this.send({ type: 'wz-init', pluginId, surface, apiVersion: '1' });
  }

  sendManifest() {
    this.send({
      type: 'wz-manifest',
      manifest: {
        id: 'test.demo',
        name: 'Demo',
        version: '1.0.0',
        apiVersion: '1',
        publisher: 'test',
      },
    });
  }

  rpcResult(id: number, ok: boolean, payload: unknown) {
    if (ok) this.send({ type: 'wz-rpc-result', id, ok: true, result: payload });
    else
      this.send({
        type: 'wz-rpc-result',
        id,
        ok: false,
        error: payload as { code: string; message: string },
      });
  }

  /** Auto-respond to plugin RPCs with a canned result. */
  autoReply(handler: (method: string, params: Record<string, unknown>, id: number) => unknown) {
    return this.on((msg) => {
      if (msg.type !== 'wz-rpc') return;
      const result = handler(
        msg.method as string,
        msg.params as Record<string, unknown>,
        msg.id as number,
      );
      this.rpcResult(msg.id as number, true, result);
    });
  }
}

/** Fresh SDK module wired to a fresh FakeHost. */
async function freshPlugin() {
  const host = new FakeHost();
  host.install();
  host.on((msg) => {
    if (msg.type === 'wz-manifest-request') host.sendManifest();
  });
  const mod = await import('./index');
  return { host, mod };
}

describe('plugin-sdk bridge', () => {
  beforeEach(() => {
    vi.resetModules();
    for (const fn of cleanups.splice(0)) fn();
  });

  it('definePlugin completes the handshake, manifest fetch and activation', async () => {
    const { host, mod } = await freshPlugin();
    host.autoReply(() => null);
    const activated = vi.fn();

    mod.definePlugin({
      activate(ctx) {
        expect(ctx.pluginId).toBe('test.demo');
        expect(ctx.surface).toBe('view:demo.main');
        expect(ctx.manifest.id).toBe('test.demo');
        activated();
      },
    });

    host.init();
    await vi.waitFor(() => expect(activated).toHaveBeenCalled());
    // plugin.log is issued by the SDK after successful activation.
    await vi.waitFor(() =>
      expect(
        host.received.some(
          (m) =>
            m.type === 'wz-rpc' &&
            m.method === 'plugin.log' &&
            String((m.params as { message?: string })?.message ?? '').includes('activated'),
        ),
      ).toBe(true),
    );
  });

  it('reports failure to the kernel with id + reason when activate throws', async () => {
    const { host, mod } = await freshPlugin();
    host.autoReply(() => null);

    mod.definePlugin({
      activate() {
        throw new Error('boom');
      },
    });
    host.init();

    await vi.waitFor(() =>
      expect(
        host.received.find((m) => m.type === 'wz-rpc' && m.method === 'plugins.reportFailure'),
      ).toBeTruthy(),
    );
    const call = host.received.find(
      (m) => m.type === 'wz-rpc' && m.method === 'plugins.reportFailure',
    )!;
    expect(call.params).toMatchObject({
      id: 'test.demo',
      reason: expect.stringContaining('boom'),
    });
  });

  it('routes kernel pushes: events, pty (base64) and ai tool calls', async () => {
    const { host, mod } = await freshPlugin();
    host.autoReply(() => null);

    let ctx: import('./index').PluginContext | null = null;
    mod.definePlugin({
      activate(c) {
        ctx = c;
      },
    });
    host.init();
    await vi.waitFor(() => expect(ctx).toBeTruthy());
    const c = ctx!;

    // events
    const seen: unknown[] = [];
    c.events.on('demo.ping', (d) => seen.push(d));
    host.send({ type: 'wz-push', topic: 'event', data: { name: 'demo.ping', data: 42 } });
    host.send({ type: 'wz-push', topic: 'event', data: { name: 'other', data: 'nope' } });
    expect(seen).toEqual([42]);

    // pty data arrives base64-encoded and is decoded
    const ptyOut: string[] = [];
    c.pty.onData('pty-1', (d) => ptyOut.push(d));
    const b64 = btoa(String.fromCharCode(...new TextEncoder().encode('héllo ✓')));
    host.send({ type: 'wz-push', topic: 'pty', data: { sessionId: 'pty-1', data: b64 } });
    expect(ptyOut).toEqual(['héllo ✓']);

    // pty exit
    let exit: { success: boolean } | null = null;
    c.pty.onExit('pty-1', (info) => (exit = info));
    host.send({
      type: 'wz-push',
      topic: 'pty',
      data: { sessionId: 'pty-1', kind: 'exit', success: true },
    });
    expect(exit).toEqual({ success: true });

    // pty.write sends base64
    await c.pty.write('pty-1', 'ls -la\n');
    const write = host.received.find((m) => m.type === 'wz-rpc' && m.method === 'pty.write');
    expect(write).toBeTruthy();
    expect((write!.params as { sessionId: string }).sessionId).toBe('pty-1');
    const sent = (write!.params as { data: string }).data;
    expect(atob(sent)).toBe('ls -la\n');

    // ai tool call round-trip
    let toolResultParams: Record<string, unknown> | null = null;
    host.on((msg) => {
      if (msg.type === 'wz-rpc' && msg.method === 'ai.toolResult') {
        toolResultParams = msg.params as Record<string, unknown>;
        host.rpcResult(msg.id as number, true, null);
      }
    });
    c.ai.onToolCall(async (name, args) => ({ name, args, echoed: true }));
    host.send({
      type: 'wz-push',
      topic: 'plugin-push',
      data: { kind: 'ai-tool-call', tool: 'demo.tool', args: { x: 1 }, requestId: 7 },
    });
    await vi.waitFor(() => expect(toolResultParams).toBeTruthy());
    expect(toolResultParams).toMatchObject({ requestId: 7, ok: true });
    expect((toolResultParams!.result as { echoed: boolean }).echoed).toBe(true);
  });

  it('buffers net chunks that arrive before fetchStream resolves and replays them', async () => {
    const { host, mod } = await freshPlugin();

    // Delay the streamId response so chunks arrive "early".
    host.on((msg) => {
      if (msg.type !== 'wz-rpc' || msg.method !== 'network.fetchStream') return;
      const id = msg.id as number;
      host.send({
        type: 'wz-push',
        topic: 'net',
        data: { kind: 'net-chunk', payload: { streamId: 'net-1', data: 'hello ' } },
      });
      host.send({
        type: 'wz-push',
        topic: 'net',
        data: { kind: 'net-chunk', payload: { streamId: 'net-1', data: 'world' } },
      });
      setTimeout(() => host.rpcResult(id, true, { streamId: 'net-1' }), 10);
    });

    let ctx: import('./index').PluginContext | null = null;
    mod.definePlugin({
      activate(c) {
        ctx = c;
      },
    });
    host.init();
    await vi.waitFor(() => expect(ctx).toBeTruthy());

    const chunks: string[] = [];
    let ended = false;
    const res = await ctx!.network.fetchStream(
      'https://example.test/sse',
      { method: 'GET' },
      { onChunk: (d) => chunks.push(d), onEnd: () => (ended = true) },
    );
    expect(res.streamId).toBe('net-1');
    // Buffered chunks replayed in order…
    expect(chunks).toEqual(['hello ', 'world']);
    expect(ended).toBe(false);
    // …and live chunks keep flowing.
    host.send({
      type: 'wz-push',
      topic: 'net',
      data: { kind: 'net-chunk', payload: { streamId: 'net-1', data: '!' } },
    });
    expect(chunks).toEqual(['hello ', 'world', '!']);
    host.send({
      type: 'wz-push',
      topic: 'net',
      data: { kind: 'net-end', payload: { streamId: 'net-1' } },
    });
    expect(ended).toBe(true);
  });

  it('replays a stream that fully ended before fetchStream resolved', async () => {
    const { host, mod } = await freshPlugin();

    // The whole stream (chunks + net-end) arrives before the RPC returns —
    // regression: the terminal callback must still fire exactly once.
    host.on((msg) => {
      if (msg.type !== 'wz-rpc' || msg.method !== 'network.fetchStream') return;
      const id = msg.id as number;
      host.send({
        type: 'wz-push',
        topic: 'net',
        data: { kind: 'net-chunk', payload: { streamId: 'net-e', data: 'a' } },
      });
      host.send({
        type: 'wz-push',
        topic: 'net',
        data: { kind: 'net-end', payload: { streamId: 'net-e' } },
      });
      setTimeout(() => host.rpcResult(id, true, { streamId: 'net-e' }), 10);
    });

    let ctx: import('./index').PluginContext | null = null;
    mod.definePlugin({
      activate(c) {
        ctx = c;
      },
    });
    host.init();
    await vi.waitFor(() => expect(ctx).toBeTruthy());

    const chunks: string[] = [];
    let ended = 0;
    let errored: string | null = null;
    await ctx!.network.fetchStream(
      'https://example.test/sse',
      { method: 'GET' },
      {
        onChunk: (d) => chunks.push(d),
        onEnd: () => ended++,
        onError: (m) => (errored = m),
      },
    );
    expect(chunks).toEqual(['a']);
    expect(ended).toBe(1);
    expect(errored).toBeNull();
  });

  it('RPC errors surface as KernelRpcError rejections', async () => {
    const { host, mod } = await freshPlugin();
    host.on((msg) => {
      if (msg.type === 'wz-rpc' && msg.method === 'storage.set') {
        host.rpcResult(
          msg.id as number,
          false,
          { code: 'kernel/permission-denied', message: 'denied!' },
        );
      }
    });

    let ctx: import('./index').PluginContext | null = null;
    mod.definePlugin({
      activate(c) {
        ctx = c;
      },
    });
    host.init();
    await vi.waitFor(() => expect(ctx).toBeTruthy());

    let denied: unknown = null;
    try {
      await ctx!.storage.set('k', 1);
    } catch (err) {
      denied = err;
    }
    expect(denied).toBeInstanceOf(mod.KernelRpcError);
    expect((denied as { code: string }).code).toBe('kernel/permission-denied');
  });
});
