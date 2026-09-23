import { useApp } from '../store';
import { Dashboard } from './Dashboard';
import { SettingsView } from './SettingsView';
import { PluginStore } from './PluginStore';
import { PluginViewHost } from './PluginViewHost';

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
        {active?.kind === 'dashboard' && <Dashboard />}
        {active?.kind === 'settings' && <SettingsView />}
        {active?.kind === 'store' && <PluginStore />}
        {active?.kind === 'plugin-view' && active.pluginId && active.viewId && (
          <PluginViewHost pluginId={active.pluginId} viewId={active.viewId} />
        )}
      </div>
    </main>
  );
}
