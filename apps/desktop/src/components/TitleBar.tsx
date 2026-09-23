import { Badge, Button, Dropdown, DropdownItem } from '@eigendesk/ui-kit';
import { Methods } from '@eigendesk/protocol';
import { useApp } from '../store';
import { rpc } from '../kernel';

export function TitleBar() {
  const current = useApp((s) => s.currentWorkspace);
  const workspaces = useApp((s) => s.workspaces);
  const openTab = useApp((s) => s.openTab);
  const refreshWorkspaces = useApp((s) => s.refreshWorkspaces);

  const switchTo = async (id: string) => {
    await rpc(Methods.workspace.open, { id });
    await refreshWorkspaces();
    useApp.getState().pushToast({ title: `Workspace: ${id}`, tone: 'info' });
  };

  return (
    <header className="ed-titlebar" role="banner">
      <strong style={{ fontSize: 'var(--ed-text-md)', letterSpacing: 0.3 }}>EigenDesk</strong>
      <Dropdown label={current ? current.name : 'No workspace'}>
        {(close) => (
          <>
            {workspaces.map((w) => (
              <DropdownItem
                key={w.id}
                onPick={() => {
                  void switchTo(w.id);
                  close();
                }}
              >
                {w.name}
                {current?.id === w.id ? ' ✓' : ''}
              </DropdownItem>
            ))}
            <DropdownItem
              onPick={() => {
                openTab({ kind: 'settings', title: 'Settings' });
                close();
              }}
            >
              Manage workspaces…
            </DropdownItem>
          </>
        )}
      </Dropdown>
      <div style={{ flex: 1 }} />
      <Badge tone="accent">v0.1</Badge>
      <Button
        variant="ghost"
        small
        aria-label="Toggle theme"
        onClick={() => useApp.getState().toggleTheme()}
      >
        ◐
      </Button>
      <Button
        variant="ghost"
        small
        aria-label="Open Plugin Store"
        onClick={() => openTab({ kind: 'store', title: 'Plugin Store' })}
      >
        ⬡ Plugins
      </Button>
    </header>
  );
}
