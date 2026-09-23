/**
 * @eigendesk/protocol — the canonical TypeScript mirror of the EigenDesk
 * kernel RPC surface (spec §30 / ADR-0003). Every type here must stay in
 * lockstep with `crates/kernel/src/rpc.rs`; the kernel is the source of
 * truth and this package is its published contract for the shell and the
 * plugin SDK.
 */

// ---------------------------------------------------------------------------
// JSON-RPC envelope
// ---------------------------------------------------------------------------

export interface RpcRequest {
  id: number;
  method: string;
  params: Record<string, unknown>;
}

export type RpcResponse =
  | { id: number; ok: true; result: unknown }
  | { id: number; ok: false; error: { code: string; message: string; data?: unknown } };

/** Hard limit enforced by the kernel; larger params are rejected. */
export const RPC_MAX_PARAMS_BYTES = 10 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Plugin manifest (spec §11) — public API
// ---------------------------------------------------------------------------

export interface CommandContribution {
  id: string;
  title: string;
  category?: string;
  keywords?: string[];
  /** Context expression, e.g. `workspace.open && terminal.active`. */
  when?: string;
  keybinding?: string;
  /** Accepts free-text input (e.g. `memo.new <text>`). */
  takesArgs?: boolean;
  hidden?: boolean;
  /** Declarative behavior: open the named view of the owning plugin. */
  opensView?: string;
}

export type ViewLocation = 'main' | 'sidebar' | 'bottom' | 'floating';

export interface ViewContribution {
  id: string;
  title: string;
  icon?: string;
  location?: ViewLocation;
}

export interface WidgetContribution {
  id: string;
  title: string;
  minWidth: number;
  minHeight: number;
  defaultWidth?: number;
  defaultHeight?: number;
}

export interface SettingContribution {
  key: string;
  type: 'boolean' | 'string' | 'number' | 'enum' | 'path';
  title: string;
  description?: string;
  default?: unknown;
  enumValues?: string[];
  /** `global` (default) or `workspace`. */
  scope?: 'global' | 'workspace';
}

export interface SearchProviderContribution {
  id: string;
  title?: string;
}

export interface CaptureProviderContribution {
  id: string;
  title?: string;
  /** Prefixes that route a Quick Capture line to this provider. */
  prefixes: string[];
  /** Lower = weaker claim. Default providers use high numbers. */
  priority?: number;
  /** Command invoked with the captured text when this provider wins. */
  command?: string;
}

export interface ArtifactTypeContribution {
  type: string;
  title: string;
}

export interface StatusItemContribution {
  id: string;
  title: string;
  command?: string;
}

export interface FileHandlerContribution {
  id: string;
  /** MIME type, or the catch-all MIME type. */
  mimeType: string;
  title: string;
}

export interface AiToolContribution {
  name: string;
  description: string;
  parameters: unknown;
  /** High-risk tools require explicit confirmation (spec §54). */
  highRisk?: boolean;
}

export interface ServiceContribution {
  id: string;
  api: string;
}

export interface PluginContributes {
  commands?: CommandContribution[];
  views?: ViewContribution[];
  widgets?: WidgetContribution[];
  settings?: SettingContribution[];
  searchProviders?: SearchProviderContribution[];
  captureProviders?: CaptureProviderContribution[];
  artifactTypes?: ArtifactTypeContribution[];
  statusItems?: StatusItemContribution[];
  fileHandlers?: FileHandlerContribution[];
  aiTools?: AiToolContribution[];
  services?: ServiceContribution[];
}

export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  apiVersion: string;
  description?: string;
  publisher?: string;
  /** Honored only for bundled first-party plugins. */
  trust?: string;
  permissions?: PermissionDeclaration[];
  activationEvents?: string[];
  contributes?: PluginContributes;
  /** Entry document served via `edp://` (default `entry.html`). */
  entry?: string;
}

// ---------------------------------------------------------------------------
// Permissions (spec §31–§33)
// ---------------------------------------------------------------------------

/** Every permission name the kernel understands. */
export type PermissionName =
  | 'workspace:read'
  | 'workspace:write'
  | 'filesystem:read'
  | 'filesystem:write'
  | 'network'
  | 'process:spawn'
  | 'clipboard:read'
  | 'clipboard:write'
  | 'notification'
  | 'secrets:read'
  | 'ai:invoke'
  | 'mcp:connect'
  | 'system:open';

export const ALL_PERMISSIONS: PermissionName[] = [
  'workspace:read',
  'workspace:write',
  'filesystem:read',
  'filesystem:write',
  'network',
  'process:spawn',
  'clipboard:read',
  'clipboard:write',
  'notification',
  'secrets:read',
  'ai:invoke',
  'mcp:connect',
  'system:open',
];

/** Human-readable one-liners for the permission approval UI (spec §33). */
export const PERMISSION_DESCRIPTIONS: Record<PermissionName, string> = {
  'workspace:read': 'Read files in this workspace',
  'workspace:write': 'Create and modify files in this workspace',
  'filesystem:read': 'Read files outside the workspace',
  'filesystem:write': 'Modify files outside the workspace',
  network: 'Access the network',
  'process:spawn': 'Run local processes',
  'clipboard:read': 'Read the clipboard',
  'clipboard:write': 'Write the clipboard',
  notification: 'Show notifications',
  'secrets:read': 'Read stored secrets',
  'ai:invoke': 'Invoke AI providers',
  'mcp:connect': 'Connect to MCP servers',
  'system:open': 'Open files, folders and links outside the app',
};

export type PermissionDeclaration =
  | PermissionName
  | { [K in PermissionName]?: K extends 'network'
      ? { hosts?: string[] }
      : K extends `filesystem:${'read' | 'write'}`
        ? string[]
        : never };

// ---------------------------------------------------------------------------
// Plugin runtime records (mirror of PluginInfo in plugin-runtime)
// ---------------------------------------------------------------------------

export type PluginState =
  | 'discovered'
  | 'installed'
  | 'disabled'
  | 'enabled'
  | 'activating'
  | 'active'
  | 'error'
  | 'uninstalled';

export interface Grants {
  permissions: Record<string, unknown>;
}

export interface PluginInfo {
  manifest: PluginManifest;
  source: 'bundled' | 'user' | 'dev';
  state: PluginState;
  pinned: boolean;
  trusted: boolean;
  installPath: string;
  failureCount: number;
  pendingPermissions: Grants | null;
  requested: Grants;
}

// ---------------------------------------------------------------------------
// Push messages (kernel → shell, spec §28)
// ---------------------------------------------------------------------------

export type PushTopic =
  | 'event'
  | 'plugin-state'
  | 'notification'
  | 'plugin-push'
  | 'shortcut'
  | 'mcp-status'
  | 'net'
  | 'pty';

export interface PushMessage {
  seq: number;
  topic: PushTopic;
  /** Plugin id for `plugin-push` routing; event source otherwise. */
  plugin?: string;
  data: unknown;
}

// ---------------------------------------------------------------------------
// Artifacts (spec §22–§24)
// ---------------------------------------------------------------------------

export interface ArtifactRecord {
  uri: string;
  type: string;
  title: string;
  pluginId: string;
  metadata: Record<string, unknown> | null;
  createdAt: string;
  updatedAt: string;
  lastOpenedAt: string | null;
}

// ---------------------------------------------------------------------------
// Search (spec §25–§26)
// ---------------------------------------------------------------------------

/** Document submitted to the search index (kernel `search.upsert`). */
export interface IndexDocument {
  uri: string;
  title: string;
  body?: string;
  tags?: string[];
  metadata?: Record<string, unknown>;
}

/** Hit returned by `search.query`. */
export interface SearchHit {
  uri: string;
  title: string;
  score: number;
  pluginId: string;
  snippet?: string;
}

export interface SearchResult {
  results: SearchHit[];
  tookMs: number;
}

export interface DirEntryInfo {
  name: string;
  isDir: boolean;
  isFile: boolean;
  isSymlink: boolean;
  size: number;
  modified?: number;
}

export interface ReadDirResult {
  path: string;
  entries: DirEntryInfo[];
}

export interface StatInfo {
  path: string;
  isDir: boolean;
  isFile: boolean;
  isSymlink: boolean;
  size: number;
  created?: number;
  modified?: number;
}

export interface ReadFileResult {
  content: string;
  truncated?: boolean;
  size?: number;
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export interface CommandDef {
  id: string;
  title: string;
  category?: string | null;
  keywords: string[];
  when?: string | null;
  pluginId?: string | null;
  defaultKeybinding?: string | null;
  takesArgs: boolean;
  hidden?: boolean;
  /** Declarative behavior: open the named view of the owning plugin. */
  opensView?: string | null;
}

// ---------------------------------------------------------------------------
// Workspace (spec §8–§9)
// ---------------------------------------------------------------------------

export interface WorkspaceInfo {
  id: string;
  name: string;
  root: string;
  createdAt: string;
  lastOpenedAt: string;
}

// ---------------------------------------------------------------------------
// Settings (spec §42–§43)
// ---------------------------------------------------------------------------

export interface SettingDescriptor {
  key: string;
  type: 'boolean' | 'string' | 'number' | 'enum' | 'path';
  title: string;
  description?: string;
  default?: unknown;
  enumValues?: string[];
  scope: 'global' | 'workspace';
  pluginId?: string;
}

// ---------------------------------------------------------------------------
// PTY / network / MCP / AI
// ---------------------------------------------------------------------------

export interface PtySessionInfo {
  sessionId: string;
  shell: string;
  cwd: string;
  createdAt?: string;
  alive?: boolean;
}

export interface McpServerStatus {
  name: string;
  command: string;
  args: string[];
  enabled: boolean;
  connected: boolean;
}

export interface McpToolInfo {
  server: string;
  name: string;
  description?: string;
  inputSchema?: unknown;
}

/** Registered AI tool as returned by `ai.listTools`. */
export interface AiTool {
  name: string;
  description: string;
  parameters: unknown;
  highRisk: boolean;
  pluginId: string;
}

export interface NotificationAction {
  id: string;
  title: string;
  command?: string;
  args?: unknown;
}

export interface ShellNotification {
  id: string;
  title: string;
  body: string;
  source?: string;
  actions: NotificationAction[];
  timestamp: string;
}

// ---------------------------------------------------------------------------
// Registry / packs / updates (spec §38–§40)
// ---------------------------------------------------------------------------

/** Catalog entry (`registry/index.json` is a JSON array of these). */
export interface CatalogEntry {
  id: string;
  name: string;
  version: string;
  publisher: string;
  description: string;
  repository?: string;
  license?: string;
  /** Local path (bundled resources or registry dir) to the package folder. */
  packagePath: string;
  sha256?: string;
  categories?: string[];
}

/** Extension pack (`registry/packs.json` is a JSON array of these). */
export interface ExtensionPack {
  id: string;
  name: string;
  description: string;
  plugins: string[];
}

// ---------------------------------------------------------------------------
// Bootstrap / diagnostics
// ---------------------------------------------------------------------------

/** What `app.bootstrapInfo` returns; the shell composes the full startup
 * state from `plugins.list` / `workspace.list` / `commands.list`. */
export interface BootstrapInfo {
  version: string;
  platform: string;
  safeMode: boolean;
  dirs: { config: string; data: string; cache: string; logs: string };
  secrets: unknown;
}

export interface StartupTiming {
  processStart: string;
  kernelReadyMs?: number;
  workspaceReadyMs?: number;
  shellReadyMs?: number;
}

export interface DiagnosticsReport {
  version: string;
  platform: string;
  startup: StartupTiming;
  plugins: Array<{ id: string; state: PluginState; failureCount: number }>;
  workspace: { id: string; root: string } | null;
  settings: Record<string, unknown>;
  timings: Record<string, number>;
}

// ---------------------------------------------------------------------------
// Method names — single source of truth for the shell and plugin SDK
// ---------------------------------------------------------------------------

export const Methods = {
  app: {
    issueToken: 'app.issueToken',
    bootstrapInfo: 'app.bootstrapInfo',
    diagnostics: 'app.diagnostics',
    exportDiagnostics: 'app.exportDiagnostics',
    logFile: 'app.logFile',
  },
  system: {
    reveal: 'system.reveal',
    openUrl: 'system.openUrl',
  },
  workspace: {
    list: 'workspace.list',
    current: 'workspace.current',
    create: 'workspace.create',
    open: 'workspace.open',
    close: 'workspace.close',
    register: 'workspace.register',
    remove: 'workspace.remove',
    saveLayout: 'workspace.saveLayout',
  },
  settings: {
    get: 'settings.get',
    getAll: 'settings.getAll',
    set: 'settings.set',
    reset: 'settings.reset',
    describe: 'settings.describe',
  },
  commands: {
    list: 'commands.list',
    register: 'commands.register',
    unregister: 'commands.unregister',
  },
  events: {
    emit: 'events.emit',
  },
  plugins: {
    list: 'plugins.list',
    get: 'plugins.get',
    install: 'plugins.install',
    installFromPath: 'plugins.installFromPath',
    installPack: 'plugins.installPack',
    approvePermissions: 'plugins.approvePermissions',
    enable: 'plugins.enable',
    disable: 'plugins.disable',
    uninstall: 'plugins.uninstall',
    setPinned: 'plugins.setPinned',
    markActive: 'plugins.markActive',
    reportFailure: 'plugins.reportFailure',
    resetFailures: 'plugins.resetFailures',
    logs: 'plugins.logs',
    registry: 'plugins.registry',
    packs: 'plugins.packs',
    checkUpdates: 'plugins.checkUpdates',
    update: 'plugins.update',
  },
  plugin: {
    log: 'plugin.log',
  },
  artifacts: {
    upsert: 'artifacts.upsert',
    remove: 'artifacts.remove',
    removeByPlugin: 'artifacts.removeByPlugin',
    listRecent: 'artifacts.listRecent',
    listByType: 'artifacts.listByType',
    markOpened: 'artifacts.markOpened',
    describe: 'artifacts.describe',
  },
  search: {
    upsert: 'search.upsert',
    removeByPlugin: 'search.removeByPlugin',
    query: 'search.query',
  },
  storage: {
    get: 'storage.get',
    set: 'storage.set',
    delete: 'storage.delete',
    keys: 'storage.keys',
    dataDir: 'storage.dataDir',
  },
  session: {
    get: 'session.get',
    set: 'session.set',
    delete: 'session.delete',
    keys: 'session.keys',
  },
  fs: {
    readFile: 'fs.readFile',
    writeFile: 'fs.writeFile',
    appendFile: 'fs.appendFile',
    readDir: 'fs.readDir',
    stat: 'fs.stat',
    mkdir: 'fs.mkdir',
    delete: 'fs.delete',
    copy: 'fs.copy',
    move: 'fs.move',
  },
  network: {
    fetch: 'network.fetch',
    fetchStream: 'network.fetchStream',
    abort: 'network.abort',
  },
  secrets: {
    status: 'secrets.status',
    list: 'secrets.list',
    get: 'secrets.get',
    set: 'secrets.set',
    delete: 'secrets.delete',
  },
  pty: {
    create: 'pty.create',
    write: 'pty.write',
    resize: 'pty.resize',
    kill: 'pty.kill',
    list: 'pty.list',
  },
  notify: {
    show: 'notify.show',
    list: 'notify.list',
  },
  ai: {
    registerTool: 'ai.registerTool',
    unregisterTools: 'ai.unregisterTools',
    listTools: 'ai.listTools',
    /** Route a tool invocation to the owning plugin and await its result. */
    callTool: 'ai.callTool',
    /** Owning plugin answers a routed tool invocation. */
    toolResult: 'ai.toolResult',
  },
  mcp: {
    status: 'mcp.status',
    listTools: 'mcp.listTools',
    callTool: 'mcp.callTool',
    addServer: 'mcp.addServer',
    removeServer: 'mcp.removeServer',
    setServerEnabled: 'mcp.setServerEnabled',
    connect: 'mcp.connect',
  },
} as const;

// ---------------------------------------------------------------------------
// Error codes (mirror of KernelError kinds)
// ---------------------------------------------------------------------------

export const ErrorCodes = {
  Unauthorized: 'kernel/unauthorized',
  NotFound: 'kernel/not-found',
  InvalidParams: 'kernel/invalid-params',
  Forbidden: 'kernel/forbidden',
  TooLarge: 'kernel/too-large',
  Internal: 'kernel/internal',
  Io: 'kernel/io',
} as const;

export class KernelRpcError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'KernelRpcError';
    this.code = code;
  }
}

// ---------------------------------------------------------------------------
// Bridge message formats (plugin iframe ⇄ trusted main frame)
// ---------------------------------------------------------------------------

/** Plugin iframe → main frame. */
export type PluginBridgeMessage =
  | { type: 'edp-rpc'; id: number; method: string; params: Record<string, unknown> }
  | { type: 'edp-ready' }
  | { type: 'edp-manifest-request' }
  | { type: 'edp-command-result'; requestId: number; ok: boolean; result?: unknown; error?: string };

/** Main frame → plugin iframe. */
export type HostBridgeMessage =
  | { type: 'edp-init'; pluginId: string; surface: string; apiVersion: string }
  | { type: 'edp-rpc-result'; id: number; ok: true; result: unknown }
  | { type: 'edp-rpc-result'; id: number; ok: false; error: { code: string; message: string } }
  | { type: 'edp-push'; topic: PushTopic; data: unknown }
  | { type: 'edp-manifest'; manifest: PluginManifest }
    | { type: 'edp-command'; requestId: number; id: string; args?: string };

/** Extracted command invocation message (host → plugin iframe). */
export interface HostBridgeCommand {
  type: 'edp-command';
  requestId: number;
  id: string;
  args?: string;
}

/** Kernel → plugin push payload for network streams (topic `net`). */
export interface NetPush {
  kind: 'net-start' | 'net-chunk' | 'net-end' | 'net-error' | 'net-abort';
  payload: {
    streamId: string;
    status?: number;
    event?: string;
    data?: string;
    message?: string;
  };
}

/** Kernel → plugin push payload for AI tool calls (topic `plugin-push`). */
export interface AiToolCallPush {
  kind: 'ai-tool-call';
  tool: string;
  args: Record<string, unknown>;
  requestId: number;
}
