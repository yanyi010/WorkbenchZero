import { useApp } from '../store';

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismissToast);

  return (
    <div className="ed-toasts" role="region" aria-label="Notifications">
      {toasts.map((t) => (
        <div key={t.id} className="ed-toast" data-tone={t.tone} role="status">
          <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
            <strong>{t.title}</strong>
            <button
              type="button"
              aria-label="Dismiss"
              onClick={() => dismiss(t.id)}
              style={{
                background: 'transparent',
                border: 'none',
                color: 'var(--ed-muted)',
                cursor: 'pointer',
              }}
            >
              ✕
            </button>
          </div>
          {t.body && (
            <div style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)' }}>{t.body}</div>
          )}
        </div>
      ))}
    </div>
  );
}
