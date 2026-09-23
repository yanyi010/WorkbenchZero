import { Dialog, Table } from '@eigendesk/ui-kit';
import { useApp } from '../store';

export function ShortcutsOverlay() {
  const overlay = useApp((s) => s.overlay);
  const setOverlay = useApp((s) => s.setOverlay);
  const commands = useApp((s) => s.commands);

  const rows = commands
    .filter((c) => !c.hidden && (c.defaultKeybinding || c.pluginId))
    .map((c) => ({
      key: c.id,
      command: c.title,
      binding: (
        <kbd
          key={c.id}
          style={{
            fontFamily: 'var(--ed-font-mono)',
            fontSize: 'var(--ed-text-xs)',
            border: '1px solid var(--ed-border)',
            borderRadius: 'var(--ed-radius-sm)',
            padding: '1px 6px',
          }}
        >
          {c.defaultKeybinding ?? '—'}
        </kbd>
      ),
      source: c.pluginId ?? 'core',
    }))
    .sort((a, b) => a.source.localeCompare(b.source) || a.command.localeCompare(b.command));

  return (
    <Dialog
      open={overlay === 'shortcuts'}
      onClose={() => setOverlay(null)}
      title="Keyboard shortcuts"
      width={640}
    >
      <p style={{ marginTop: 0, color: 'var(--ed-muted)' }}>
        Defaults are shown; you can override them in Settings → Keybindings.
      </p>
      <Table
        columns={[
          { key: 'binding', title: 'Shortcut', width: 140 },
          { key: 'command', title: 'Command' },
          { key: 'source', title: 'From', width: 160 },
        ]}
        rows={rows}
      />
    </Dialog>
  );
}
