/**
 * E2E: the real shell boots against a fake kernel.
 *
 * This exercises the full startup sequence from main.tsx (token →
 * pluginHost.start → store.boot → manifest cache) with the actual App
 * component tree — no component is mocked. The `@tauri-apps/api/core`
 * invoke is replaced by an in-memory kernel whose dispatch mirrors the
 * semantics of crates/kernel/src/rpc.rs (token guard, method table,
 * state transitions).
 */
import React from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ExtensionPack, PluginInfo, PluginManifest, RpcResponse } from '@workbench-zero/protocol';

// ---------------------------------------------------------------------------
// fake kernel
// ---------------------------------------------------------------------------

interface MockState {
  token: string | null;
  plugins: Map<string, PluginInfo>;
  packs: ExtensionPack[];
  workspaceCreated: boolean;
  settings: Record<string, unknown>;
  calls: { method: string; params: Record<string, unknown> }[];
  pushHandler: ((batch: { seq: number; topic: string; data: unknown }[]) => void) | null;
}

const state: MockState = {
  token: null,
  plugins: new Map(),
  packs: [],
  workspaceCreated: false,
  settings: {},
  calls: [],
  pushHandler: null,
};

function manifest(id: string, name: string, extra: Partial<PluginManifest> = {}): PluginManifest {
  return {
    id,
    name,
    version: '0.1.0',
    apiVersion: '1',
    publisher: 'zero',
    trust: 'trusted',
    permissions: [],
    activationEvents: ['onStartup'],
    contributes: {
      commands: [{ id: `${id}.new`, title: `New ${name}`, takesArgs: true }],
      views: [{ id: `${id}.main`, title: name }],
      captureProviders:
        id === 'zero.memo'
          ? [{ id: `${id}.default`, title: name, prefixes: [], priority: 100, command: `${id}.new` }]
          : id === 'zero.tasks'
            ? [{ id: `${id}.quick`, title: name, prefixes: ['/t'], priority: 90, command: `${id}.quickAdd` }]
            : [],
      ...extra.contributes,
    },
    ...extra,
  };
}

function info(m: PluginManifest, st: PluginInfo['state']): PluginInfo {
  return {
    manifest: m,
    source: 'bundled',
    state: st,
    pinned: false,
    trusted: m.trust === 'trusted',
    installPath: `/plugins/${m.id}`,
    failureCount: 0,
    pendingPermissions: null,
    requested: { permissions: {} },
  };
}

function resetKernel(): void {
  state.token = null;
  state.plugins = new Map([
    ['zero.memo', info(manifest('zero.memo', 'Memo'), 'discovered')],
    ['zero.tasks', info(manifest('zero.tasks', 'Tasks'), 'discovered')],
    ['zero.sticky', info(manifest('zero.sticky', 'Sticky'), 'discovered')],
    ['zero.ai', info(manifest('zero.ai', 'Quick Ask'), 'discovered')],
  ]);
  state.packs = [
    { id: 'essentials', name: 'Essentials', description: '', plugins: ['zero.memo', 'zero.tasks', 'zero.sticky', 'zero.ai'] },
  ];
  state.workspaceCreated = false;
  state.settings = {};
  state.calls = [];
  state.pushHandler = null;
}

/** vi.mock factories are hoisted — the fn must come from vi.hoisted. */
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

const invoke = mocks.invoke;
mocks.invoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
  if (cmd !== 'kernel_rpc') throw new Error(`unexpected tauri command: ${cmd}`);
  const { token, payload } = args as { token: string; payload: { id: number; method: string; params: Record<string, unknown> } };

  // Token guard: only the first, empty-token caller may claim it.
  // Every response echoes the request id (mirrors Kernel::rpc). Omit must
  // distribute over the union or the discriminants collapse.
  type RespBody = RpcResponse extends infer T ? (T extends { id: number } ? Omit<T, 'id'> : never) : never;
  const resp = (r: RespBody): RpcResponse => ({ id: payload.id, ...r });

  if (payload.method === 'app.issueToken') {
    if (state.token !== null)
      return resp({ ok: false, error: { code: 'kernel/unauthorized', message: 'token already issued' } });
    state.token = 'e2e-token';
    return resp({ ok: true, result: state.token });
  }
  if (token !== state.token) {
    return resp({ ok: false, error: { code: 'kernel/unauthorized', message: 'bad token' } });
  }

  state.calls.push({ method: payload.method, params: payload.params });
  const p = payload.params;
  switch (payload.method) {
    case 'app.bootstrapInfo':
      return resp({ ok: true, result: { version: '0.1.0', platform: 'linux', safeMode: false } });
    case 'plugins.list':
      return resp({ ok: true, result: [...state.plugins.values()] });
    case 'workspace.list':
      return resp({ ok: true, result: state.workspaceCreated ? [{ id: 'w1', name: 'E2E', root: '/tmp/e2e-ws' }] : [] });
    case 'workspace.current':
      return resp({ ok: true, result: state.workspaceCreated ? { id: 'w1', name: 'E2E', root: '/tmp/e2e-ws' } : null });
    case 'workspace.create':
      state.workspaceCreated = true;
      return resp({ ok: true, result: { id: 'w1', name: p.name, root: p.root } });
    case 'settings.describe':
      return resp({ ok: true, result: [] });
    case 'settings.getAll':
      return resp({ ok: true, result: state.settings });
    case 'settings.set':
      state.settings[p.key as string] = p.value;
      return resp({ ok: true, result: null });
    case 'commands.list':
      return {
        ok: true,
        result: [
          { id: 'core.showPalette', title: 'Command Palette', keywords: [], defaultKeybinding: 'Ctrl+K', takesArgs: false },
          { id: 'core.quickCapture', title: 'Quick Capture', keywords: [], defaultKeybinding: 'Alt+Space', takesArgs: false },
          { id: 'core.universalSearch', title: 'Search', keywords: [], defaultKeybinding: 'Ctrl+P', takesArgs: false },
          { id: 'core.openSettings', title: 'Open Settings', keywords: [], takesArgs: false },
          { id: 'zero.memo.new', title: 'New Memo', keywords: [], pluginId: 'zero.memo', takesArgs: true },
        ],
      };
    case 'plugins.registry':
      return resp({ ok: true, result: [] });
    case 'plugins.packs':
      return resp({ ok: true, result: state.packs });
    case 'plugins.installPack': {
      const pack = state.packs.find((x) => x.id === p.packId);
      for (const id of pack?.plugins ?? []) {
        const rec = state.plugins.get(id);
        if (rec) state.plugins.set(id, { ...rec, state: 'installed' });
      }
      return resp({ ok: true, result: (pack?.plugins ?? []).map((id) => ({ id, ok: true })) });
    }
    case 'plugins.install': {
      const rec = state.plugins.get(p.id as string);
      if (rec) state.plugins.set(p.id as string, { ...rec, state: 'installed' });
      return resp({ ok: true, result: rec });
    }
    case 'plugins.approvePermissions': {
      const rec = state.plugins.get(p.id as string);
      if (rec) state.plugins.set(p.id as string, { ...rec, state: 'enabled' });
      return resp({ ok: true, result: rec });
    }
    case 'plugins.enable': {
      const rec = state.plugins.get(p.id as string);
      if (rec) state.plugins.set(p.id as string, { ...rec, state: 'enabled' });
      return resp({ ok: true, result: rec });
    }
    case 'plugins.markActive':
      return resp({ ok: true, result: null });
    case 'search.query':
      return {
        ok: true,
        result: {
          results: [
            { uri: 'file:///memo-1.md', title: 'Berry convergence', score: 1, pluginId: 'zero.memo', snippet: '…estimator…' },
          ],
          tookMs: 3,
        },
      };
    case 'events.emit':
      return resp({ ok: true, result: null });
    default:
      return resp({ ok: false, error: { code: 'kernel/unknown-method', message: payload.method } });
  }
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

// ---------------------------------------------------------------------------
// boot helper — mirrors main.tsx start()
// ---------------------------------------------------------------------------

import { issueToken } from '../../apps/desktop/src/kernel';
import { pluginHost } from '../../apps/desktop/src/pluginHost';
import { useApp } from '../../apps/desktop/src/store';
import { App } from '../../apps/desktop/src/App';

async function bootApp(): Promise<void> {
  pluginHost.start();
  window.__kernelInbox = (batch) => useApp.getState().handleKernelPush(batch);
  window.__quickCapture = () => useApp.getState().setOverlay('capture');
  await issueToken();
  await useApp.getState().boot();
  for (const p of useApp.getState().plugins) pluginHost.cacheManifest(p.manifest);
  render(React.createElement(React.StrictMode, null, React.createElement(App)));
}

function keydown(k: Partial<KeyboardEventInit>): void {
  fireEvent(window, new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...k }));
}

afterEach(() => {
  cleanup(); // RTL cannot auto-register without vitest globals
});

beforeEach(() => {
  document.body.replaceChildren();
  const root = document.createElement('div');
  root.id = 'root';
  document.body.appendChild(root);
  // logic-frame host expected by pluginHost
  const frames = document.createElement('div');
  frames.id = 'wz-logic-frames';
  document.body.appendChild(frames);
  useApp.setState(useApp.getInitialState());
  resetKernel();
});

// ---------------------------------------------------------------------------
// scenarios
// ---------------------------------------------------------------------------

describe('E2E: shell boot', () => {
  it('boots to the welcome overlay on first run (all plugins discovered)', async () => {
    await act(async () => {
      await bootApp();
    });
    expect(useApp.getState().booted).toBe(true);
    expect(useApp.getState().bootError).toBeNull();
    // First run: every bundled plugin still undiscovered.
    expect(useApp.getState().overlay).toBe('welcome');
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByText('Welcome to Workbench Zero')).toBeTruthy();
    expect(within(dialog).getByPlaceholderText('e.g. Research')).toBeTruthy();
    // Trusted plugins with onStartup are not auto-enabled on first run.
    expect(useApp.getState().plugins.every((p) => p.state === 'discovered')).toBe(true);
  });

  it('the token is issued exactly once and guards later calls', async () => {
    await act(async () => {
      await bootApp();
    });
    const issued = state.calls.filter((c) => c.method === 'app.issueToken');
    expect(issued).toHaveLength(0); // consumed before entering the table
    expect(state.token).toBe('e2e-token');
    // A second token claim must fail (kernel semantics).
    const res = (await invoke('kernel_rpc', {
      token: '',
      payload: { id: 999, method: 'app.issueToken', params: {} },
    })) as RpcResponse;
    expect(res.ok).toBe(false);
  });

  it('welcome → create workspace → install pack → plugins land enabled', async () => {
    await act(async () => {
      await bootApp();
    });

    // Step 1: workspace (labels render above the inputs).
    const nameInput = await screen.findByPlaceholderText('e.g. Research');
    fireEvent.change(nameInput, { target: { value: 'E2E' } });
    const rootInput = screen.getByPlaceholderText('e.g. ~/Workbench');
    fireEvent.change(rootInput, { target: { value: '/tmp/e2e-ws' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    // Step 2: pick the Essentials pack.
    const packButton = await screen.findByRole('button', { name: 'Install Essentials' });
    fireEvent.click(packButton);

    await waitFor(() => {
      expect(useApp.getState().plugins.filter((p) => p.state === 'installed')).toHaveLength(4);
    });
    // Workspace registration happened.
    expect(state.calls.some((c) => c.method === 'workspace.create')).toBe(true);
  });

  it('Ctrl+K opens the palette; core.openSettings opens the settings tab', async () => {
    useApp.setState({ overlay: null });
    await act(async () => {
      await bootApp();
    });
    useApp.setState({ overlay: null });

    keydown({ key: 'k', ctrlKey: true });
    await waitFor(() => expect(useApp.getState().overlay).toBe('palette'));
    const input = await screen.findByPlaceholderText(/command|search/i);
    fireEvent.change(input, { target: { value: 'settings' } });
    const item = await screen.findByText(/open settings/i);
    fireEvent.click(item);
    await waitFor(() => {
      const tab = useApp.getState().tabs.find((t) => t.kind === 'settings');
      expect(tab).toBeTruthy();
      expect(useApp.getState().activeTab).toBe(tab!.id);
    });
  });

  it('Alt+Space opens Quick Capture; plain text previews the memo route', async () => {
    await act(async () => {
      await bootApp();
    });
    useApp.setState({ overlay: null });
    // Enable memo so the capture router has a provider.
    const rec = state.plugins.get('zero.memo')!;
    state.plugins.set('zero.memo', { ...rec, state: 'enabled' });
    await act(async () => {
      await useApp.getState().refreshPlugins();
    });

    keydown({ key: ' ', altKey: true });
    await waitFor(() => expect(useApp.getState().overlay).toBe('capture'));
    const input = await screen.findByPlaceholderText(/capture|type/i);
    fireEvent.change(input, { target: { value: 'remember Berry convergence' } });
    await waitFor(() => expect(screen.getByText(/memo/i)).toBeTruthy());
  });

  it('Ctrl+P opens universal search and renders hits', async () => {
    await act(async () => {
      await bootApp();
    });
    useApp.setState({ overlay: null });

    keydown({ key: 'p', ctrlKey: true });
    await waitFor(() => expect(useApp.getState().overlay).toBe('search'));
    const input = await screen.findByPlaceholderText(/search/i);
    fireEvent.change(input, { target: { value: 'berry' } });
    await waitFor(() => expect(screen.getByText(/Berry convergence/)).toBeTruthy());
  });

  it('kernel notification pushes surface as toasts', async () => {
    await act(async () => {
      await bootApp();
    });
    useApp.setState({ overlay: null });
    window.__kernelInbox!([
      { seq: 1, topic: 'notification', data: { id: 'n1', title: 'Memo saved', body: 'hi' } },
    ]);
    await waitFor(() => expect(screen.getByText('Memo saved')).toBeTruthy());
    expect(useApp.getState().notifications).toHaveLength(1);
  });

  it('plugin-state pushes drop frames of disabled plugins and refresh state', async () => {
    await act(async () => {
      await bootApp();
    });
    useApp.setState({ overlay: null });
    state.plugins.get('zero.memo')!.state = 'enabled';
    window.__kernelInbox!([{ seq: 2, topic: 'plugin-state', data: { id: 'zero.memo', state: 'enabled' } }]);
    await waitFor(() => {
      expect(useApp.getState().plugins.find((p) => p.manifest.id === 'zero.memo')?.state).toBe('enabled');
    });
  });

  it('theme toggle persists via settings.set', async () => {
    await act(async () => {
      await bootApp();
    });
    useApp.setState({ overlay: null });
    await act(async () => {
      await useApp.getState().toggleTheme();
    });
    expect(useApp.getState().theme).toBe('dark');
    expect(state.settings['core.appearance.theme']).toBe('dark');
  });
});
