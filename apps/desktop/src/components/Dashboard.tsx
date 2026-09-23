import { useEffect, useRef } from 'react';
import { Card, EmptyState } from '@eigendesk/ui-kit';
import { useApp } from '../store';
import { pluginHost, startupActivationPlugins } from '../pluginHost';

/**
 * Dashboard: core welcome card + one host per enabled plugin widget
 * (spec §21). Widget layout belongs to the user — v0.1 uses a simple
 * responsive grid; the positions are not persisted yet.
 */
export function Dashboard() {
  const plugins = useApp((s) => s.plugins);
  const workspace = useApp((s) => s.currentWorkspace);
  const activatedRef = useRef(false);

  // Lazy startup activation: only plugins that explicitly opted in with
  // onStartup / onWorkspaceOpen get their logic frame at boot (spec §12).
  useEffect(() => {
    if (activatedRef.current) return;
    activatedRef.current = true;
    for (const id of startupActivationPlugins(
      plugins.map((p) => ({ manifest: p.manifest, state: p.state })),
    )) {
      void pluginHost.ensureLogicFrame(id).catch(() => {
        useApp.getState().pushToast({
          title: `Failed to activate ${id}`,
          tone: 'danger',
        });
      });
    }
  }, [plugins]);

  const widgets = plugins
    .filter((p) => p.state === 'enabled' || p.state === 'active')
    .flatMap((p) =>
      (p.manifest.contributes?.widgets ?? []).map((w) => ({
        pluginId: p.manifest.id,
        widget: w,
      })),
    );

  return (
    <div className="ed-scroll">
      <div style={{ maxWidth: 1100, margin: '0 auto' }}>
        <Card
          title={workspace ? `Good to see you — ${workspace.name}` : 'Welcome to EigenDesk'}
          actions={
            <span style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)' }}>
              Ctrl+K commands · Alt+Space capture · Ctrl+P search
            </span>
          }
        >
          <p style={{ margin: 0, color: 'var(--ed-muted)' }}>
            {workspace
              ? 'Everything you capture stays in this workspace, on this machine.'
              : 'Create a workspace from the title bar to start capturing.'}
          </p>
        </Card>

        {widgets.length === 0 ? (
          <EmptyState
            icon="▤"
            title="No dashboard widgets"
            hint="Enable plugins with widget contributions to fill this space."
            style={{ marginTop: 'var(--ed-space-4)' }}
          />
        ) : (
          <div className="ed-dashboard-grid">
            {widgets.map(({ pluginId, widget }) => (
              <DashboardWidget key={`${pluginId}/${widget.id}`} pluginId={pluginId} widgetId={widget.id} title={widget.title} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function DashboardWidget({
  pluginId,
  widgetId,
  title,
}: {
  pluginId: string;
  widgetId: string;
  title: string;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!ref.current) return;
    pluginHost.attachViewFrame(ref.current, pluginId, `widget:${widgetId}`);
    return () => {
      pluginHost.detachViewFrame(pluginId, `widget:${widgetId}`);
    };
  }, [pluginId, widgetId]);

  return (
    <section className="ed-widget" aria-label={title}>
      <div ref={ref} className="ed-widget-frame" />
    </section>
  );
}
