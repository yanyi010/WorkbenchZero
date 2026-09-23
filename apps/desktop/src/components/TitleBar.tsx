import { Badge, Button, Dropdown, DropdownItem } from '@workbench-zero/ui-kit';
import { useApp } from '../store';

export function TitleBar() {
  const current = useApp((s) => s.currentWorkspace);
  const workspaces = useApp((s) => s.workspaces);
  const openTab = useApp((s) => s.openTab);

  const switchTo = async (id: string) => {
    await useApp.getState().openWorkspace(id);
    const name = useApp.getState().currentWorkspace?.name ?? id;
    useApp.getState().pushToast({ title: `Workspace: ${name}`, tone: 'info' });
  };

  return (
    <header className="ed-titlebar" role="banner">
      <strong style={{ fontSize: 'var(--ed-text-md)', letterSpacing: 0.3 }}>Workbench Zero</strong>
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
        onClick={() => void useApp.getState().toggleTheme()}
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
