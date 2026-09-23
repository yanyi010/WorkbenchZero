import { useEffect, useState } from 'react';
import type {
  CatalogEntry,
  ExtensionPack,
  PermissionName,
  PluginInfo,
} from '@workbench-zero/protocol';
import { Methods, PERMISSION_DESCRIPTIONS } from '@workbench-zero/protocol';
import { Badge, Button, Dialog, EmptyState, Tabs } from '@workbench-zero/ui-kit';
import { useApp } from '../store';
import { rpc } from '../kernel';

/**
 * Plugin Store (spec §38, §33): installed plugins with lifecycle controls,
 * the bundled catalog, and extension packs. Installs that need permission
 * approval show the §33 consent dialog before enabling.
 */
export function PluginStore() {
  const [tab, setTab] = useState('installed');
  const plugins = useApp((s) => s.plugins);
  const refreshPlugins = useApp((s) => s.refreshPlugins);
  const pushToast = useApp((s) => s.pushToast);
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [packs, setPacks] = useState<ExtensionPack[]>([]);
  const [approving, setApproving] = useState<PluginInfo | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    void rpc<CatalogEntry[]>(Methods.plugins.registry).then(setCatalog).catch(() => {});
    void rpc<ExtensionPack[]>(Methods.plugins.packs).then(setPacks).catch(() => {});
  }, []);

  const run = async (label: string, fn: () => Promise<unknown>) => {
    setBusy(label);
    try {
      await fn();
      await refreshPlugins();
      await useApp.getState().refreshCommands();
    } catch (err) {
      pushToast({ title: `${label} failed`, body: String(err), tone: 'danger' });
    } finally {
      setBusy(null);
    }
  };

  const install = (id: string) =>
    run(`Install ${id}`, () => rpc(Methods.plugins.install, { id }));

  const onInstallResult = (info: PluginInfo) => {
    if (info.pendingPermissions && Object.keys(info.pendingPermissions.permissions ?? {}).length) {
      setApproving(info);
    }
  };

  const installedIds = new Set(
    plugins.filter((p) => p.state !== 'discovered' && p.state !== 'uninstalled').map((p) => p.manifest.id),
  );
  const available = catalog.filter((c) => !installedIds.has(c.id));

  return (
    <div className="ed-scroll">
      <div style={{ maxWidth: 900, margin: '0 auto' }}>
        <Tabs
          tabs={[
            { id: 'installed', title: `Installed (${plugins.length})` },
            { id: 'browse', title: `Browse (${available.length})` },
            { id: 'packs', title: `Packs (${packs.length})` },
          ]}
          active={tab}
          onChange={setTab}
        />
        <div style={{ paddingTop: 'var(--ed-space-3)' }}>
          {tab === 'installed' && (
            <div className="ed-plugin-list">
              {plugins.length === 0 && (
                <EmptyState icon="⬡" title="No plugins installed" hint="Browse the catalog to add some." />
              )}
              {plugins.map((p) => (
                <PluginRow
                  key={p.manifest.id}
                  plugin={p}
                  busy={busy === `Enable ${p.manifest.id}` || busy === `Disable ${p.manifest.id}`}
                  onApprove={() => setApproving(p)}
                  onEnable={() =>
                    void run(`Enable ${p.manifest.id}`, () =>
                      rpc(Methods.plugins.enable, { id: p.manifest.id }),
                    )
                  }
                  onDisable={() =>
                    void run(`Disable ${p.manifest.id}`, () =>
                      rpc(Methods.plugins.disable, { id: p.manifest.id }),
                    )
                  }
                  onUninstall={() =>
                    void run(`Uninstall ${p.manifest.id}`, () =>
                      rpc(Methods.plugins.uninstall, { id: p.manifest.id }),
                    )
                  }
                />
              ))}
            </div>
          )}

          {tab === 'browse' && (
            <div className="ed-plugin-list">
              {available.length === 0 && (
                <EmptyState icon="⬡" title="Catalog is empty" hint="Bundled plugins appear here." />
              )}
              {available.map((c) => (
                <div key={c.id} className="ed-plugin-row">
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div>
                      <strong>{c.name}</strong>{' '}
                      <span style={{ color: 'var(--ed-muted)' }}>
                        {c.id} · v{c.version}
                      </span>
                    </div>
                    <div style={{ color: 'var(--ed-muted)' }}>{c.description}</div>
                  </div>
                  <Button
                    variant="primary"
                    disabled={busy === `Install ${c.id}`}
                    onClick={() => void install(c.id).then(() => {
                      const info = useApp.getState().plugins.find((p) => p.manifest.id === c.id);
                      if (info) onInstallResult(info);
                    })}
                  >
                    Install
                  </Button>
                </div>
              ))}
            </div>
          )}

          {tab === 'packs' && (
            <div className="ed-plugin-list">
              {packs.length === 0 && <EmptyState icon="◫" title="No packs available" />}
              {packs.map((pack) => (
                <div key={pack.id} className="ed-plugin-row">
                  <div style={{ flex: 1 }}>
                    <strong>{pack.name}</strong>
                    <div style={{ color: 'var(--ed-muted)' }}>{pack.description}</div>
                    <div style={{ marginTop: 4, display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                      {pack.plugins.map((pid) => (
                        <Badge key={pid}>{pid}</Badge>
                      ))}
                    </div>
                  </div>
                  <Button
                    variant="primary"
                    disabled={busy === `Install pack ${pack.id}`}
                    onClick={() =>
                      void run(`Install pack ${pack.id}`, () =>
                        rpc(Methods.plugins.installPack, { packId: pack.id }),
                      )
                    }
                  >
                    Install pack
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <PermissionApprovalDialog
        plugin={approving}
        onClose={() => setApproving(null)}
        onDecision={(approve) =>
          void run('Approve permissions', async () => {
            await rpc(Methods.plugins.approvePermissions, {
              id: approving!.manifest.id,
              approve,
            });
            if (approve) {
              await rpc(Methods.plugins.enable, { id: approving!.manifest.id });
            }
            setApproving(null);
          })
        }
      />
    </div>
  );
}

function stateTone(state: PluginInfo['state']): 'success' | 'warning' | 'muted' | 'danger' {
  switch (state) {
    case 'active':
    case 'enabled':
      return 'success';
    case 'error':
      return 'danger';
    case 'installed':
    case 'discovered':
      return 'muted';
    default:
      return 'warning';
  }
}

function PluginRow({
  plugin,
  busy,
  onApprove,
  onEnable,
  onDisable,
  onUninstall,
}: {
  plugin: PluginInfo;
  busy: boolean;
  onApprove: () => void;
  onEnable: () => void;
  onDisable: () => void;
  onUninstall: () => void;
}) {
  const [logs, setLogs] = useState<{ ts: string; level: string; message: string }[] | null>(
    null,
  );
  const loadLogs = (id: string) =>
    rpc<{ ts: string; level: string; message: string }[]>(Methods.plugins.logs, { id })
      .then(setLogs)
      .catch(() => setLogs([]));
  const p = plugin;
  const needsApproval =
    (p.pendingPermissions && Object.keys(p.pendingPermissions.permissions ?? {}).length > 0) ||
    (p.state === 'installed' && !p.trusted);
  return (
    <div className="ed-plugin-row" style={{ alignItems: 'flex-start', flexDirection: 'column' }}>
      <div style={{ display: 'flex', gap: 'var(--ed-space-3)', width: '100%', alignItems: 'center' }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div>
            <strong>{p.manifest.name}</strong>{' '}
            <span style={{ color: 'var(--ed-muted)' }}>
              {p.manifest.id} · v{p.manifest.version}
            </span>{' '}
            <Badge tone={stateTone(p.state)}>{p.state}</Badge>
            {p.trusted && <Badge tone="accent">first-party</Badge>}
            {p.pinned && <Badge>pinned</Badge>}
          </div>
          <div style={{ color: 'var(--ed-muted)' }}>{p.manifest.description}</div>
          {p.state === 'error' && p.failureCount > 0 && (
            <div style={{ color: 'var(--ed-danger)', fontSize: 'var(--ed-text-sm)' }}>
              failed {p.failureCount}× — check logs
            </div>
          )}
        </div>
        <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          {needsApproval && (
            <Button variant="primary" small onClick={onApprove}>
              Review permissions
            </Button>
          )}
          {(p.state === 'enabled' || p.state === 'active') && (
            <Button small disabled={busy} onClick={onDisable}>
              Disable
            </Button>
          )}
          {(p.state === 'installed' || p.state === 'disabled' || p.state === 'discovered') &&
            !needsApproval && (
              <Button small variant="primary" disabled={busy} onClick={onEnable}>
                Enable
              </Button>
            )}
          <Button
            small
            onClick={() => {
              if (logs) return setLogs(null);
              void loadLogs(p.manifest.id);
            }}
          >
            {logs ? 'Hide logs' : 'Logs'}
          </Button>
          <Button small variant="danger" onClick={onUninstall}>
            Uninstall
          </Button>
        </div>
      </div>
      {logs && (
        <pre
          style={{
            width: '100%',
            maxHeight: 180,
            overflow: 'auto',
            background: 'var(--ed-background-sink)',
            border: '1px solid var(--ed-border)',
            borderRadius: 'var(--ed-radius-md)',
            padding: 'var(--ed-space-2)',
            fontSize: 'var(--ed-text-sm)',
            fontFamily: 'var(--ed-font-mono)',
          }}
        >
          {logs.length === 0
            ? 'no log output'
            : logs.map((l) => `[${l.level}] ${l.message}`).join('\n')}
        </pre>
      )}
    </div>
  );
}

function PermissionApprovalDialog({
  plugin,
  onClose,
  onDecision,
}: {
  plugin: PluginInfo | null;
  onClose: () => void;
  onDecision: (approve: boolean) => void;
}) {
  if (!plugin) return null;
  const requested = Object.keys(
    (plugin.pendingPermissions ?? plugin.requested).permissions ?? {},
  ) as PermissionName[];
  return (
    <Dialog
      open
      onClose={onClose}
      title={`Permissions — ${plugin.manifest.name}`}
      footer={
        <>
          <Button onClick={() => onDecision(false)}>Deny</Button>
          <Button variant="primary" onClick={() => onDecision(true)}>
            Approve & enable
          </Button>
        </>
      }
    >
      <p style={{ marginTop: 0 }}>
        <strong>{plugin.manifest.id}</strong> requests:
      </p>
      <ul className="ed-perm-list">
        {requested.map((perm) => (
          <li key={perm}>
            <span aria-hidden>✓</span>
            <span>{PERMISSION_DESCRIPTIONS[perm] ?? perm}</span>
          </li>
        ))}
        {requested.length === 0 && <li>No additional permissions requested.</li>}
      </ul>
      <p style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)', marginBottom: 0 }}>
        New permissions added by future updates require renewed approval. Denying keeps the plugin
        installed but disabled.
      </p>
    </Dialog>
  );
}
