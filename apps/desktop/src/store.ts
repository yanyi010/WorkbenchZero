/**
 * Shell state (zustand). The store is only ever mutated from:
 *  - the bootstrap sequence in main.tsx,
 *  - kernel pushes via window.__kernelInbox,
 *  - user interactions.
 *
 * All kernel reads go through refresh helpers so push-triggered refreshes
 * and manual refreshes share one code path.
 */
import { create } from 'zustand';
import type {
  BootstrapInfo,
  CommandDef,
  PluginInfo,
  PushMessage,
  SearchHit,
  SettingDescriptor,
  ShellNotification,
  WorkspaceInfo,
} from '@workbench-zero/protocol';
import { Methods } from '@workbench-zero/protocol';
import { rpc } from './kernel';
import { pluginHost } from './pluginHost';

export interface ViewTab {
  /** Unique tab id. */
  id: string;
  kind: 'dashboard' | 'settings' | 'store' | 'plugin-view' | 'search';
  title: string;
  pluginId?: string;
  viewId?: string;
}

export interface Toast {
  id: string;
  title: string;
  body?: string;
  tone: 'info' | 'success' | 'warning' | 'danger';
  createdAt: number;
}

interface AppState {
  booted: boolean;
  bootError: string | null;
  info: BootstrapInfo | null;

  workspaces: WorkspaceInfo[];
  currentWorkspace: WorkspaceInfo | null;

  plugins: PluginInfo[];
  commands: CommandDef[];
  descriptors: SettingDescriptor[];
  settings: Record<string, unknown>;

  notifications: ShellNotification[];
  toasts: Toast[];

  tabs: ViewTab[];
  activeTab: string | null;
  sidebarVisible: boolean;
  bottomPanelVisible: boolean;
  theme: 'light' | 'dark';

  overlay: null | 'palette' | 'capture' | 'search' | 'shortcuts' | 'welcome';
  searchResults: SearchHit[] | null;
  searchTookMs: number | null;

  // -- actions ------------------------------------------------------------
  boot(): Promise<void>;
  refreshPlugins(): Promise<void>;
  refreshCommands(): Promise<void>;
  refreshSettings(): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  openTab(tab: Omit<ViewTab, 'id'> & { id?: string }): string;
  closeTab(id: string): void;
  setActiveTab(id: string): void;
  setOverlay(overlay: AppState['overlay']): void;
  toggleTheme(): Promise<void>;
  pushToast(t: Omit<Toast, 'id' | 'createdAt'>): void;
  dismissToast(id: string): void;
  handleKernelPush(batch: PushMessage[]): void;
  executeCommand(id: string, args?: string): Promise<void>;
}

function tabIdFor(tab: Omit<ViewTab, 'id'>): string {
  if (tab.kind === 'plugin-view') return `view:${tab.pluginId}:${tab.viewId}`;
  return tab.kind;
}

export const useApp = create<AppState>((set, get) => ({
  booted: false,
  bootError: null,
  info: null,

  workspaces: [],
  currentWorkspace: null,

  plugins: [],
  commands: [],
  descriptors: [],
  settings: {},

  notifications: [],
  toasts: [],

  tabs: [{ id: 'dashboard', kind: 'dashboard', title: 'Dashboard' }],
  activeTab: 'dashboard',
  sidebarVisible: true,
  bottomPanelVisible: false,
  theme: 'light',

  overlay: null,
  searchResults: null,
  searchTookMs: null,

  async boot() {
    try {
      const info = await rpc<BootstrapInfo>(Methods.app.bootstrapInfo);
      set({ info });
      await Promise.all([
        get().refreshWorkspaces(),
        get().refreshPlugins(),
        get().refreshSettings(),
        // Keybindings resolve from the command table — without this the
        // palette/capture/search shortcuts stay dead until a plugin-state
        // push happens to trigger a refresh.
        get().refreshCommands(),
      ]);
      const theme = (get().settings['core.appearance.theme'] as string) === 'dark'
        ? 'dark'
        : 'light';
      const firstRun = get().plugins.every((p) => !p.trusted || p.state === 'discovered');
      set({
        booted: true,
        theme,
        overlay: firstRun ? 'welcome' : null,
      });
    } catch (err) {
      set({ bootError: String(err), booted: true });
    }
  },

  async refreshPlugins() {
    const plugins = await rpc<PluginInfo[]>(Methods.plugins.list);
    set({ plugins });
  },

  async refreshCommands() {
    const commands = await rpc<CommandDef[]>(Methods.commands.list);
    set({ commands });
  },

  async refreshSettings() {
    const [descriptors, settings] = await Promise.all([
      rpc<SettingDescriptor[]>(Methods.settings.describe),
      rpc<Record<string, unknown>>(Methods.settings.getAll),
    ]);
    set({ descriptors, settings });
  },

  async refreshWorkspaces() {
    const [workspaces, current] = await Promise.all([
      rpc<WorkspaceInfo[]>(Methods.workspace.list),
      rpc<WorkspaceInfo | null>(Methods.workspace.current),
    ]);
    set({ workspaces, currentWorkspace: current });
  },

  openTab(tab) {
    const id = tab.id ?? tabIdFor(tab);
    const existing = get().tabs.find((t) => t.id === id);
    if (!existing) {
      set({ tabs: [...get().tabs, { ...tab, id }], activeTab: id });
    } else {
      set({ activeTab: id });
    }
    if (tab.kind === 'plugin-view' && tab.pluginId) {
      void rpc(Methods.plugins.markActive, { id: tab.pluginId, activationMs: 0 }).catch(
        () => {},
      );
    }
    return id;
  },

  closeTab(id) {
    const tabs = get().tabs.filter((t) => t.id !== id);
    const activeTab = get().activeTab === id ? (tabs.at(-1)?.id ?? null) : get().activeTab;
    set({ tabs, activeTab });
  },

  setActiveTab(id) {
    set({ activeTab: id });
  },

  setOverlay(overlay) {
    set({ overlay });
  },

  async toggleTheme() {
    const theme = get().theme === 'dark' ? 'light' : 'dark';
    set({ theme });
    await rpc(Methods.settings.set, {
      scope: 'global',
      key: 'core.appearance.theme',
      value: theme,
    }).catch(() => {});
  },

  pushToast(t) {
    const toast: Toast = {
      ...t,
      id: `toast-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      createdAt: Date.now(),
    };
    set({ toasts: [...get().toasts.slice(-4), toast] });
    setTimeout(() => get().dismissToast(toast.id), 5000);
  },

  dismissToast(id) {
    set({ toasts: get().toasts.filter((t) => t.id !== id) });
  },

  handleKernelPush(batch) {
    for (const msg of batch) {
      switch (msg.topic) {
        case 'event':
          // Route to plugin frames; the shell itself reacts to a few
          // lifecycle events below.
          pluginHost.broadcastEvent(msg.data as { name: string; data: unknown });
          break;
        case 'plugin-state': {
          const data = msg.data as { id?: string; state?: string };
          if (data?.id) pluginHost.onPluginStateChanged(data.id);
          void get().refreshPlugins().then(() => get().refreshCommands());
          break;
        }
        case 'notification': {
          const n = msg.data as ShellNotification;
          set({ notifications: [n, ...get().notifications].slice(0, 100) });
          get().pushToast({
            title: n.title,
            body: n.body,
            tone: n.source ? 'info' : 'success',
          });
          break;
        }
        case 'plugin-push':
        case 'net':
        case 'pty':
        case 'mcp-status':
          pluginHost.routePush(msg);
          break;
        case 'shortcut':
          break;
      }
    }
  },

  async executeCommand(id, args) {
    const cmd = get().commands.find((c) => c.id === id);
    if (!cmd) {
      get().pushToast({ title: `Unknown command: ${id}`, tone: 'danger' });
      return;
    }
    const started = performance.now();
    try {
      if (cmd.opensView && cmd.pluginId) {
        // Declarative view opening needs no iframe.
        const view = get()
          .plugins.find((p) => p.manifest.id === cmd.pluginId)
          ?.manifest.contributes?.views?.find((v) => v.id === cmd.opensView);
        get().openTab({
          kind: 'plugin-view',
          title: view?.title ?? cmd.title,
          pluginId: cmd.pluginId,
          viewId: cmd.opensView,
        });
        return;
      }
      if (!cmd.pluginId) {
        await runCoreCommand(id, args);
        return;
      }
      // Plugin command: activate + route to the logic iframe.
      await pluginHost.ensureLogicFrame(cmd.pluginId);
      const result = await pluginHost.sendCommand(cmd.pluginId, id, args);
      if (result && typeof result === 'object' && 'toast' in (result as Record<string, unknown>)) {
        const t = (result as { toast?: string; tone?: 'info' | 'success' | 'warning' | 'danger' }).toast;
        if (t) get().pushToast({ title: t, tone: (result as { tone?: Toast['tone'] }).tone ?? 'success' });
      }
      const ms = performance.now() - started;
      void rpc(Methods.plugins.markActive, { id: cmd.pluginId, activationMs: ms }).catch(() => {});
    } catch (err) {
      get().pushToast({ title: `Command failed: ${cmd.title}`, body: String(err), tone: 'danger' });
    }
  },
}));

/** Core commands are executed by the shell itself. */
async function runCoreCommand(id: string, _args?: string): Promise<void> {
  const app = useApp.getState();
  switch (id) {
    case 'core.showPalette':
      app.setOverlay('palette');
      break;
    case 'core.quickCapture':
      app.setOverlay('capture');
      break;
    case 'core.universalSearch':
      app.setOverlay('search');
      break;
    case 'core.toggleSidebar':
      useApp.setState({ sidebarVisible: !app.sidebarVisible });
      break;
    case 'core.toggleBottomPanel':
      useApp.setState({ bottomPanelVisible: !app.bottomPanelVisible });
      break;
    case 'core.closeTab':
      if (app.activeTab) app.closeTab(app.activeTab);
      break;
    case 'core.nextTab': {
      const i = app.tabs.findIndex((t) => t.id === app.activeTab);
      const next = app.tabs[(i + 1) % app.tabs.length];
      if (next) app.setActiveTab(next.id);
      break;
    }
    case 'core.prevTab': {
      const i = app.tabs.findIndex((t) => t.id === app.activeTab);
      const prev = app.tabs[(i - 1 + app.tabs.length) % app.tabs.length];
      if (prev) app.setActiveTab(prev.id);
      break;
    }
    case 'core.openDashboard':
      app.openTab({ kind: 'dashboard', title: 'Dashboard' });
      break;
    case 'core.openSearch':
      app.setOverlay('search');
      break;
    case 'core.openPlugins':
      app.openTab({ kind: 'store', title: 'Plugin Store' });
      break;
    case 'core.openSettings':
      app.openTab({ kind: 'settings', title: 'Settings' });
      break;
    case 'core.showKeyboardShortcuts':
      app.setOverlay('shortcuts');
      break;
    case 'core.toggleTheme':
      await app.toggleTheme();
      break;
    case 'core.openLogsFolder':
      await rpc(Methods.system.reveal, { path: (await rpc<{ dir: string }>(Methods.app.logFile)).dir });
      break;
    case 'core.exportDiagnostics': {
      const res = await rpc<{ path: string }>(Methods.app.exportDiagnostics);
      app.pushToast({ title: 'Diagnostics exported', body: res.path, tone: 'success' });
      break;
    }
    case 'core.reloadWindow':
      window.location.reload();
      break;
    case 'core.quit':
      window.close();
      break;
    case 'core.newWorkspace':
    case 'core.openWorkspace':
    case 'core.switchWorkspace':
    case 'core.closeWorkspace':
      app.openTab({ kind: 'settings', title: 'Settings' });
      break;
    default:
      app.pushToast({ title: `Core command not implemented: ${id}`, tone: 'warning' });
  }
}
