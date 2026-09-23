import { useApp } from '../store';

interface NavEntry {
  id: string;
  icon: string;
  label: string;
  tab: { kind: 'dashboard' | 'store' | 'settings'; title: string };
}

const CORE_NAV: NavEntry[] = [
  { id: 'dashboard', icon: '⌂', label: 'Dashboard', tab: { kind: 'dashboard', title: 'Dashboard' } },
  { id: 'store', icon: '⬡', label: 'Plugin Store', tab: { kind: 'store', title: 'Plugin Store' } },
  { id: 'settings', icon: '⚙', label: 'Settings', tab: { kind: 'settings', title: 'Settings' } },
];

export function ActivityBar() {
  const plugins = useApp((s) => s.plugins);
  const activeTab = useApp((s) => s.activeTab);
  const openTab = useApp((s) => s.openTab);

  // One activity-bar entry per enabled plugin main view (spec §19).
  const pluginViews = plugins
    .filter((p) => p.state === 'enabled' || p.state === 'active')
    .flatMap((p) =>
      (p.manifest.contributes?.views ?? [])
        .filter((v) => (v.location ?? 'main') === 'main')
        .map((v) => ({
          id: `${p.manifest.id}/${v.id}`,
          icon: v.icon ?? '▣',
          label: v.title,
          tab: { kind: 'plugin-view' as const, title: v.title, pluginId: p.manifest.id, viewId: v.id },
        })),
    );

  const entries = [...CORE_NAV, ...pluginViews];

  return (
    <nav className="ed-activity" role="navigation" aria-label="Primary">
      {entries.map((e) => {
        const tabId = e.tab.kind;
        const pressed =
          e.tab.kind === 'plugin-view'
            ? activeTab === `view:${e.tab.pluginId}:${e.tab.viewId}`
            : activeTab === tabId;
        return (
          <button
            key={e.id}
            title={e.label}
            aria-label={e.label}
            aria-pressed={pressed}
            onClick={() => openTab(e.tab)}
          >
            {e.icon}
          </button>
        );
      })}
    </nav>
  );
}
