import { useEffect, useState } from 'react';
import type { ArtifactRecord } from '@eigendesk/protocol';
import { Methods } from '@eigendesk/protocol';
import { EmptyState, Spinner } from '@eigendesk/ui-kit';
import { useApp } from '../store';
import { rpc } from '../kernel';

export function Sidebar() {
  const workspace = useApp((s) => s.currentWorkspace);
  const commands = useApp((s) => s.commands);
  const [recent, setRecent] = useState<ArtifactRecord[] | null>(null);

  useEffect(() => {
    let alive = true;
    if (!workspace) {
      setRecent(null);
      return;
    }
    rpc<ArtifactRecord[]>(Methods.artifacts.listRecent, { limit: 8 })
      .then((r) => alive && setRecent(r))
      .catch(() => alive && setRecent([]));
    return () => {
      alive = false;
    };
  }, [workspace?.id]);

  const openArtifact = (uri: string, pluginId: string) => {
    void rpc(Methods.artifacts.markOpened, { uri });
    useApp.setState({ searchResults: null });
    // Route to the owning plugin's first view; plugins listen for the
    // `artifact.open` event to focus the item.
    const plugin = useApp.getState().plugins.find((p) => p.manifest.id === pluginId);
    const view = plugin?.manifest.contributes?.views?.find((v) => (v.location ?? 'main') === 'main');
    if (plugin && view) {
      useApp.getState().openTab({
        kind: 'plugin-view',
        title: view.title,
        pluginId,
        viewId: view.id,
      });
    }
    void rpc(Methods.events.emit, { name: 'artifact.open', data: { uri, pluginId } });
  };

  const quickCommands = commands
    .filter((c) => !c.hidden && c.pluginId && (c.category === 'Capture' || c.takesArgs))
    .slice(0, 6);

  return (
    <aside className="ed-sidebar" aria-label="Sidebar">
      {workspace ? (
        <>
          <div>
            <div className="ed-section-title">Workspace</div>
            <div style={{ fontWeight: 600 }}>{workspace.name}</div>
            <div
              style={{ fontSize: 'var(--ed-text-xs)', color: 'var(--ed-muted)', wordBreak: 'break-all' }}
              title={workspace.root}
            >
              {workspace.root}
            </div>
          </div>
          <div>
            <div className="ed-section-title">Recent</div>
            {recent === null ? (
              <Spinner label="Loading recents" />
            ) : recent.length === 0 ? (
              <div style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)' }}>
                Nothing captured yet
              </div>
            ) : (
              <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
                {recent.map((a) => (
                  <li key={a.uri}>
                    <button
                      type="button"
                      onClick={() => openArtifact(a.uri, a.pluginId)}
                      style={{
                        font: 'inherit',
                        fontSize: 'var(--ed-text-sm)',
                        textAlign: 'left',
                        background: 'transparent',
                        border: 'none',
                        color: 'var(--ed-foreground)',
                        cursor: 'pointer',
                        padding: '3px 6px',
                        borderRadius: 'var(--ed-radius-sm)',
                        width: '100%',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                      title={a.title ?? a.uri}
                    >
                      {a.title ?? a.uri}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {quickCommands.length > 0 && (
            <div>
              <div className="ed-section-title">Quick actions</div>
              <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
                {quickCommands.map((c) => (
                  <li key={c.id}>
                    <button
                      type="button"
                      onClick={() => void useApp.getState().executeCommand(c.id)}
                      style={{
                        font: 'inherit',
                        fontSize: 'var(--ed-text-sm)',
                        textAlign: 'left',
                        background: 'transparent',
                        border: 'none',
                        color: 'var(--ed-foreground)',
                        cursor: 'pointer',
                        padding: '3px 6px',
                        borderRadius: 'var(--ed-radius-sm)',
                      }}
                    >
                      {c.title}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </>
      ) : (
        <EmptyState
          icon="▤"
          title="No workspace"
          hint="Create or register a workspace to start capturing."
        />
      )}
    </aside>
  );
}
