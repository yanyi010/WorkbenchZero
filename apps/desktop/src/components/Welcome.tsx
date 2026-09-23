import { useEffect, useState } from 'react';
import type { ExtensionPack } from '@workbench-zero/protocol';
import { Methods } from '@workbench-zero/protocol';
import { Badge, Button, Card, Dialog, Input } from '@workbench-zero/ui-kit';
import { useApp } from '../store';
import { rpc } from '../kernel';

/**
 * First-run welcome (user directive): desktop-only distribution, zero
 * command line. Steps: create/register a workspace → pick a starter pack →
 * done. Skippable; re-runnable from Settings.
 */
export function Welcome() {
  const overlay = useApp((s) => s.overlay);
  const setOverlay = useApp((s) => s.setOverlay);
  const refreshWorkspaces = useApp((s) => s.refreshWorkspaces);
  const refreshPlugins = useApp((s) => s.refreshPlugins);
  const pushToast = useApp((s) => s.pushToast);

  const [step, setStep] = useState<'workspace' | 'pack'>('workspace');
  const [name, setName] = useState('');
  const [root, setRoot] = useState('');
  const [packs, setPacks] = useState<ExtensionPack[]>([]);
  const [busy, setBusy] = useState(false);

  const open = overlay === 'welcome';

  useEffect(() => {
    if (open && step === 'pack') {
      void rpc<ExtensionPack[]>(Methods.plugins.packs).then(setPacks).catch(() => {});
    }
  }, [open, step]);

  if (!open) return null;

  const createWorkspace = async () => {
    setBusy(true);
    try {
      await rpc(Methods.workspace.create, { name, root, createRoot: true });
      await refreshWorkspaces();
      setStep('pack');
    } catch (err) {
      pushToast({ title: 'Could not create workspace', body: String(err), tone: 'danger' });
    } finally {
      setBusy(false);
    }
  };

  const installPack = async (pack: ExtensionPack | null) => {
    setBusy(true);
    try {
      if (pack) {
        const results = (await rpc<{ id: string; ok: boolean; error?: string }[]>(
          Methods.plugins.installPack,
          { packId: pack.id },
        )) ?? [];
        const failed = results.filter((r) => !r.ok);
        if (failed.length > 0) {
          pushToast({
            title: 'Some plugins failed to install',
            body: failed.map((f) => `${f.id}: ${f.error}`).join('; '),
            tone: 'warning',
          });
        } else {
          pushToast({ title: `${pack.name} installed`, tone: 'success' });
        }
      }
      await refreshPlugins();
      await useApp.getState().refreshCommands();
    } finally {
      setBusy(false);
      setOverlay(null);
    }
  };

  return (
    <Dialog
      open
      onClose={() => setOverlay(null)}
      title={step === 'workspace' ? 'Welcome to Workbench Zero' : 'Choose your starter pack'}
      width={620}
    >
      {step === 'workspace' ? (
        <>
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 12 }}>
            <svg width="52" height="52" viewBox="0 0 1024 1024" aria-hidden>
              <defs>
                <linearGradient id="wz-welcome-ring" x1="0" y1="0" x2="1" y2="1">
                  <stop offset="0" stopColor="#38BDF8" />
                  <stop offset="1" stopColor="#818CF8" />
                </linearGradient>
              </defs>
              <rect width="1024" height="1024" rx="230" fill="#0B1120" />
              <line x1="168" y1="712" x2="856" y2="712" stroke="#E2E8F0" strokeWidth="42" strokeLinecap="round" />
              <circle cx="512" cy="452" r="234" fill="none" stroke="url(#wz-welcome-ring)" strokeWidth="98" />
              <rect x="300" y="764" width="74" height="74" rx="18" fill="#38BDF8" />
              <rect x="475" y="764" width="74" height="74" rx="18" fill="#818CF8" />
              <rect x="650" y="764" width="74" height="74" rx="18" fill="#E2E8F0" />
            </svg>
            <div>
              <div style={{ fontWeight: 700, fontSize: 'var(--ed-text-lg)' }}>Your workbench. From zero.</div>
              <div style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)' }}>
                A local-first personal workbench built entirely around plugins.
              </div>
            </div>
          </div>
          <p style={{ marginTop: 0, color: 'var(--ed-muted)' }}>
            Everything lives in a folder you own: memos, tasks, notes and indexes. Pick where Workbench
            Zero should keep this workspace.
          </p>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ed-space-3)' }}>
            <label>
              <div style={{ marginBottom: 4 }}>Workspace name</div>
              <Input
                autoFocus
                placeholder="e.g. Research"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <label>
              <div style={{ marginBottom: 4 }}>Folder</div>
              <Input
                placeholder="e.g. ~/Workbench"
                value={root}
                onChange={(e) => setRoot(e.target.value)}
              />
            </label>
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <Button onClick={() => setOverlay(null)}>Skip for now</Button>
              <Button
                variant="primary"
                disabled={busy || !name.trim() || !root.trim()}
                onClick={() => void createWorkspace()}
              >
                Continue
              </Button>
            </div>
          </div>
        </>
      ) : (
        <>
          <p style={{ marginTop: 0, color: 'var(--ed-muted)' }}>
            Packs install a curated set of plugins. You can change everything later in the Plugin
            Store.
          </p>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ed-space-2)' }}>
            {packs.map((pack) => (
              <Card key={pack.id} title={pack.name} style={{ padding: 'var(--ed-space-3)' }}>
                <div style={{ color: 'var(--ed-muted)', marginBottom: 8 }}>{pack.description}</div>
                <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 8 }}>
                  {pack.plugins.map((pid) => (
                    <Badge key={pid}>{pid}</Badge>
                  ))}
                </div>
                <Button
                  variant="primary"
                  disabled={busy}
                  onClick={() => void installPack(pack)}
                >
                  Install {pack.name}
                </Button>
              </Card>
            ))}
            {packs.length === 0 && (
              <div style={{ color: 'var(--ed-muted)' }}>No packs available.</div>
            )}
            <Button disabled={busy} onClick={() => void installPack(null)}>
              Skip — I'll choose plugins myself
            </Button>
          </div>
        </>
      )}
    </Dialog>
  );
}
