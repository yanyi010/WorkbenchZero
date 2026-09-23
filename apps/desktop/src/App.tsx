/** Root layout: title bar, activity bar, sidebar, tabs, overlays. */
import { useEffect } from 'react';
import { useApp } from './store';
import { eventToAccel, resolveBindings, type UserBindings } from './keybindings';
import { TitleBar } from './components/TitleBar';
import { ActivityBar } from './components/ActivityBar';
import { Sidebar } from './components/Sidebar';
import { StatusBar } from './components/StatusBar';
import { TabArea } from './components/TabArea';
import { Palette } from './components/Palette';
import { QuickCapture } from './components/QuickCapture';
import { SearchOverlay } from './components/SearchOverlay';
import { Toasts } from './components/Toasts';
import { Welcome } from './components/Welcome';
import { ShortcutsOverlay } from './components/ShortcutsOverlay';

export function App() {
  const booted = useApp((s) => s.booted);
  const bootError = useApp((s) => s.bootError);
  const theme = useApp((s) => s.theme);
  const sidebarVisible = useApp((s) => s.sidebarVisible);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const accel = eventToAccel(e);
      if (!accel) return;
      const target = e.target as HTMLElement;
      const typing =
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable;
      const user = (useApp.getState().settings['core.keybindings'] ?? {}) as UserBindings;
      const bindings = resolveBindings(useApp.getState().commands, user);
      const hit = bindings.find((b) => b.accel === accel);
      if (!hit) return;
      // Typing contexts swallow plain keys but still allow modifiers.
      if (typing && !accel.includes('Ctrl') && !accel.includes('Alt') && !accel.includes('Cmd')) {
        return;
      }
      e.preventDefault();
      void useApp.getState().executeCommand(hit.commandId);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  if (!booted) {
    return (
      <div className="ed-app" aria-busy="true">
        <div style={{ gridArea: 'titlebar', display: 'grid', placeItems: 'center' }}>
          Workbench Zero
        </div>
        <div style={{ gridArea: 'main', display: 'grid', placeItems: 'center', color: 'var(--ed-muted)' }}>
          ◌ starting kernel…
        </div>
      </div>
    );
  }

  if (bootError) {
    return (
      <div className="ed-boot-error" role="alert">
        <h1>Kernel bootstrap failed</h1>
        <p>{bootError}</p>
      </div>
    );
  }

  return (
    <>
      <div className="ed-app" data-sidebar={sidebarVisible}>
        <TitleBar />
        <ActivityBar />
        <Sidebar />
        <TabArea />
        <StatusBar />
      </div>
      <Palette />
      <QuickCapture />
      <SearchOverlay />
      <ShortcutsOverlay />
      <Welcome />
      <Toasts />
      <div id="wz-logic-frames" className="ed-logic-frames" />
    </>
  );
}
