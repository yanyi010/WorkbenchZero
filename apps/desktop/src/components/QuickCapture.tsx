import { useEffect, useMemo, useRef, useState } from 'react';
import { Badge } from '@eigendesk/ui-kit';
import { useApp } from '../store';
import { pluginHost } from '../pluginHost';
import { resolveCaptureProvider } from '../captureRouting';
import type { CaptureProviderContribution } from '@eigendesk/protocol';

/**
 * Quick Capture (spec §17–18). Core provides the input and the router;
 * plugins provide the providers (see captureRouting.ts for the rules).
 */

export function QuickCapture() {
  const overlay = useApp((s) => s.overlay);
  const setOverlay = useApp((s) => s.setOverlay);
  const plugins = useApp((s) => s.plugins);
  const executeCommand = useApp((s) => s.executeCommand);
  const pushToast = useApp((s) => s.pushToast);
  const [value, setValue] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const open = overlay === 'capture';

  const providers = useMemo(() => {
    const out: { pluginId: string; c: CaptureProviderContribution }[] = [];
    for (const p of plugins) {
      if (p.state !== 'enabled' && p.state !== 'active') continue;
      for (const c of p.manifest.contributes?.captureProviders ?? []) {
        out.push({ pluginId: p.manifest.id, c });
      }
    }
    return out;
  }, [plugins]);

  useEffect(() => {
    if (open) {
      setValue('');
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const route = (input: string) =>
    resolveCaptureProvider(
      input,
      providers.map(({ pluginId, c }) => ({ pluginId, contribution: c })),
    );

  const preview = route(value);

  const submit = async () => {
    const resolved = route(value);
    if (!resolved) {
      pushToast({
        title: 'No capture provider available',
        body: 'Install Memo or another capture plugin.',
        tone: 'warning',
      });
      return;
    }
    const { pluginId, contribution, text } = resolved;
    if (!text) {
      pushToast({ title: 'Nothing to capture', tone: 'warning' });
      return;
    }
    setOverlay(null);
    const title = contribution.title || pluginId;
    try {
      if (contribution.command) {
        await executeCommand(contribution.command, text);
      } else {
        // Provider without a declarative command: the plugin listens for
        // the capture event on its logic surface.
        await pluginHost.ensureLogicFrame(pluginId);
        await pluginHost.sendCommand(pluginId, `__capture:${contribution.id}`, text);
      }
      pushToast({ title: `Captured via ${title}`, tone: 'success' });
    } catch (err) {
      pushToast({ title: `Capture failed (${title})`, body: String(err), tone: 'danger' });
    }
  };

  if (!open) return null;

  return (
    <div className="ed-overlay">
      <div className="ed-overlay-backdrop" onMouseDown={() => setOverlay(null)} />
      <div className="ed-capture" role="dialog" aria-label="Quick capture">
        <input
          ref={inputRef}
          value={value}
          placeholder="Capture a thought… (/t task, ? ask)"
          aria-label="Capture text"
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              void submit();
            } else if (e.key === 'Escape') {
              setOverlay(null);
            }
          }}
        />
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 'var(--ed-space-2)',
            minHeight: 20,
            color: 'var(--ed-muted)',
            fontSize: 'var(--ed-text-sm)',
          }}
        >
          {preview ? (
            <>
              <Badge tone="accent">{preview.contribution.title || preview.pluginId}</Badge>
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {preview.text}
              </span>
            </>
          ) : value.trim() ? (
            <span>no provider matches — install a capture plugin</span>
          ) : (
            <span>Enter to capture · Esc to close</span>
          )}
        </div>
      </div>
    </div>
  );
}
