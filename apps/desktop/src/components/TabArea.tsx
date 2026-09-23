import { useApp } from '../store';
import { Dashboard } from './Dashboard';
import { SettingsView } from './SettingsView';
import { PluginStore } from './PluginStore';
import { PluginViewHost } from './PluginViewHost';
import { ErrorBoundary } from './ErrorBoundary';

export function TabArea() {
  const tabs = useApp((s) => s.tabs);
  const activeTab = useApp((s) => s.activeTab);
  const setActiveTab = useApp((s) => s.setActiveTab);
  const closeTab = useApp((s) => s.closeTab);

  const active = tabs.find((t) => t.id === activeTab) ?? tabs[0];

  return (
    <main className="ed-main" role="main">
      <div className="ed-tabs" role="tablist" aria-label="Open views">
        {tabs.map((t) => (
          <button
            key={t.id}
            role="tab"
            className="ed-tab"
            aria-selected={t.id === active?.id}
            onClick={() => setActiveTab(t.id)}
          >
            {t.title}
            {tabs.length > 1 && (
              <span
                role="button"
                aria-label={`Close ${t.title}`}
                className="ed-tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.id);
                }}
              >
                ✕
              </span>
            )}
          </button>
        ))}
      </div>
      <div className="ed-view" role="tabpanel">
        {active?.kind === 'dashboard' && (
          <ErrorBoundary label="Dashboard">
            <Dashboard />
          </ErrorBoundary>
        )}
        {active?.kind === 'settings' && (
          <ErrorBoundary label="Settings">
            <SettingsView />
          </ErrorBoundary>
        )}
        {active?.kind === 'store' && (
          <ErrorBoundary label="Plugin Store">
            <PluginStore />
          </ErrorBoundary>
        )}
        {active?.kind === 'plugin-view' && active.pluginId && active.viewId && (
          <ErrorBoundary label={`${active.pluginId} view`} key={active.id}>
            <PluginViewHost pluginId={active.pluginId} viewId={active.viewId} />
          </ErrorBoundary>
        )}
      </div>
    </main>
  );
}
