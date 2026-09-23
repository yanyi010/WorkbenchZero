import { useState } from 'react';
import type { SettingDescriptor } from '@eigendesk/protocol';
import { Methods } from '@eigendesk/protocol';
import { Button, Card, Input, Table, Tabs } from '@eigendesk/ui-kit';
import { useApp } from '../store';
import { rpc } from '../kernel';

/**
 * Settings (spec §42–§43): global + workspace scopes, core settings and
 * plugin-contributed settings, plus workspace management.
 */
export function SettingsView() {
  const [tab, setTab] = useState('general');
  const descriptors = useApp((s) => s.descriptors);

  const core = descriptors.filter((d) => !d.pluginId);
  const byPlugin = new Map<string, SettingDescriptor[]>();
  for (const d of descriptors) {
    if (!d.pluginId) continue;
    const list = byPlugin.get(d.pluginId) ?? [];
    list.push(d);
    byPlugin.set(d.pluginId, list);
  }

  return (
    <div className="ed-scroll">
      <div style={{ maxWidth: 860, margin: '0 auto' }}>
        <Tabs
          tabs={[
            { id: 'general', title: 'General' },
            { id: 'workspace', title: 'Workspace' },
            ...[...byPlugin.keys()].map((p) => ({ id: `plugin:${p}`, title: p })),
          ]}
          active={tab}
          onChange={setTab}
        />
        <div style={{ paddingTop: 'var(--ed-space-3)' }}>
          {tab === 'general' && <GeneralSettings descriptors={core} />}
          {tab === 'workspace' && <WorkspaceSettings />}
          {tab.startsWith('plugin:') && (
            <PluginSettings
              pluginId={tab.slice('plugin:'.length)}
              descriptors={byPlugin.get(tab.slice('plugin:'.length)) ?? []}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function SettingRow({ d }: { d: SettingDescriptor }) {
  const settings = useApp((s) => s.settings);
  const refresh = useApp((s) => s.refreshSettings);
  const pushToast = useApp((s) => s.pushToast);
  const value = settings[d.key] ?? d.default;

  const set = async (v: unknown) => {
    const scope = d.scope === 'workspace' ? 'workspace' : 'global';
    try {
      await rpc(Methods.settings.set, { scope, key: d.key, value: v });
      await refresh();
      if (d.key === 'core.appearance.theme') {
        useApp.setState({ theme: v === 'dark' ? 'dark' : 'light' });
      }
    } catch (err) {
      pushToast({ title: `Cannot set ${d.key}`, body: String(err), tone: 'danger' });
    }
  };

  return (
    <div className="ed-setting-row">
      <div>
        <div>{d.title}</div>
        {d.description && (
          <div style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)' }}>
            {d.description}
          </div>
        )}
        <code style={{ fontSize: 'var(--ed-text-xs)', color: 'var(--ed-muted)' }}>{d.key}</code>
      </div>
      <div>
        {d.type === 'boolean' && (
          <Button variant={value ? 'primary' : 'secondary'} onClick={() => void set(!value)}>
            {value ? 'On' : 'Off'}
          </Button>
        )}
        {d.type === 'enum' && (
          <select
            value={typeof value === 'string' ? value : ''}
            onChange={(e) => void set(e.target.value)}
            aria-label={d.title}
            style={{
              font: 'inherit',
              padding: '4px 8px',
              borderRadius: 'var(--ed-radius-md)',
              border: '1px solid var(--ed-border)',
              background: 'var(--ed-background-alt)',
              color: 'var(--ed-foreground)',
            }}
          >
            {(d.enumValues ?? []).map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
        )}
        {(d.type === 'string' || d.type === 'path' || d.type === 'number') && (
          <Input
            style={{ width: 260 }}
            defaultValue={typeof value === 'string' ? value : ''}
            aria-label={d.title}
            onBlur={(e) => void set(d.type === 'number' ? Number(e.target.value) : e.target.value)}
          />
        )}
      </div>
    </div>
  );
}

function GeneralSettings({ descriptors }: { descriptors: SettingDescriptor[] }) {
  const groups = new Map<string, SettingDescriptor[]>();
  for (const d of descriptors) {
    const section = d.key.split('.').slice(0, 2).join('.');
    const list = groups.get(section) ?? [];
    list.push(d);
    groups.set(section, list);
  }
  return (
    <>
      {[...groups.entries()].map(([section, list]) => (
        <Card key={section} title={section} style={{ marginBottom: 'var(--ed-space-3)' }}>
          {list.map((d) => (
            <SettingRow key={d.key} d={d} />
          ))}
        </Card>
      ))}
      {descriptors.length === 0 && <Card>No core settings registered.</Card>}
    </>
  );
}

function PluginSettings({
  pluginId,
  descriptors,
}: {
  pluginId: string;
  descriptors: SettingDescriptor[];
}) {
  return (
    <>
      {descriptors.map((d) => (
        <Card key={d.key} title={d.title} style={{ marginBottom: 'var(--ed-space-3)' }}>
          <SettingRow d={d} />
        </Card>
      ))}
      {descriptors.length === 0 && <Card>No settings contributed by {pluginId}.</Card>}
    </>
  );
}

function WorkspaceSettings() {
  const workspaces = useApp((s) => s.workspaces);
  const current = useApp((s) => s.currentWorkspace);
  const refresh = useApp((s) => s.refreshWorkspaces);
  const pushToast = useApp((s) => s.pushToast);
  const [name, setName] = useState('');
  const [root, setRoot] = useState('');

  const create = async (registerExisting: boolean) => {
    try {
      if (registerExisting) {
        await rpc(Methods.workspace.register, { root });
      } else {
        await rpc(Methods.workspace.create, { name, root, createRoot: true });
      }
      await refresh();
      pushToast({ title: 'Workspace ready', tone: 'success' });
    } catch (err) {
      pushToast({ title: 'Workspace operation failed', body: String(err), tone: 'danger' });
    }
  };

  return (
    <>
      <Card title="Create workspace" style={{ marginBottom: 'var(--ed-space-3)' }}>
        <div style={{ display: 'flex', gap: 'var(--ed-space-2)', flexWrap: 'wrap' }}>
          <Input
            placeholder="Name (e.g. Research)"
            aria-label="Workspace name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            style={{ width: 200 }}
          />
          <Input
            placeholder="Path (e.g. ~/workbench)"
            aria-label="Workspace root path"
            value={root}
            onChange={(e) => setRoot(e.target.value)}
            style={{ flex: 1, minWidth: 220 }}
          />
          <Button variant="primary" disabled={!name || !root} onClick={() => void create(false)}>
            Create
          </Button>
          <Button disabled={!root} onClick={() => void create(true)}>
            Register existing
          </Button>
        </div>
        <p style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)', marginBottom: 0 }}>
          A `.workbench` directory is created inside the root for indexes and plugin state.
        </p>
      </Card>

      <Card title="Known workspaces">
        <Table
          columns={[
            { key: 'name', title: 'Name' },
            { key: 'root', title: 'Root' },
            { key: 'opened', title: 'Last opened', width: 180 },
            { key: 'actions', title: '', width: 150 },
          ]}
          rows={workspaces.map((w) => ({
            name: (
              <strong>
                {w.name} {current?.id === w.id ? '· current' : ''}
              </strong>
            ),
            root: <code style={{ fontSize: 'var(--ed-text-sm)' }}>{w.root}</code>,
            opened: new Date(w.lastOpenedAt).toLocaleString(),
            actions: (
              <span style={{ display: 'flex', gap: 6 }}>
                <Button
                  small
                  disabled={current?.id === w.id}
                  onClick={() => {
                    void rpc(Methods.workspace.open, { id: w.id }).then(() => refresh());
                  }}
                >
                  Open
                </Button>
                <Button
                  small
                  variant="danger"
                  disabled={current?.id === w.id}
                  onClick={() => {
                    void rpc(Methods.workspace.remove, { id: w.id }).then(() => refresh());
                  }}
                >
                  Remove
                </Button>
              </span>
            ),
          }))}
        />
      </Card>
    </>
  );
}
