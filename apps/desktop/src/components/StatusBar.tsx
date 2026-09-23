import { useApp } from '../store';

export function StatusBar() {
  const plugins = useApp((s) => s.plugins);
  const workspace = useApp((s) => s.currentWorkspace);
  const notifications = useApp((s) => s.notifications);

  const enabled = plugins.filter((p) => p.state === 'enabled' || p.state === 'active').length;
  const failing = plugins.filter((p) => p.state === 'error').length;

  return (
    <footer className="ed-status" role="contentinfo">
      <span>{workspace ? workspace.name : 'no workspace'}</span>
      <span aria-label={`${enabled} plugins enabled`}>⬡ {enabled}</span>
      {failing > 0 && (
        <span style={{ color: 'var(--ed-danger)' }} role="alert">
          ⚠ {failing} plugin{failing > 1 ? 's' : ''} in error
        </span>
      )}
      <div style={{ flex: 1 }} />
      <button
        type="button"
        className="ed-status-notifications"
        onClick={() => useApp.getState().openTab({ kind: 'settings', title: 'Settings' })}
        style={{
          font: 'inherit',
          background: 'transparent',
          border: 'none',
          color: 'inherit',
          cursor: 'pointer',
        }}
        title={`${notifications.length} notifications this session`}
      >
        🔔 {notifications.length}
      </button>
    </footer>
  );
}
