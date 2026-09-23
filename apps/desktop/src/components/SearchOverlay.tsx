import { useEffect, useMemo, useRef, useState } from 'react';
import type { SearchHit } from '@eigendesk/protocol';
import type { CommandListItem } from '@eigendesk/ui-kit';
import { Methods } from '@eigendesk/protocol';
import { CommandList } from '@eigendesk/ui-kit';
import { useApp } from '../store';
import { rpc } from '../kernel';

/**
 * Universal Search (spec §25): kernel-federated FTS index merged with
 * matching commands. First results should appear fast; the input is
 * debounced at 120 ms to keep the kernel responsive.
 */
export function SearchOverlay() {
  const overlay = useApp((s) => s.overlay);
  const setOverlay = useApp((s) => s.setOverlay);
  const commands = useApp((s) => s.commands);
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState(0);
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [tookMs, setTookMs] = useState<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const open = overlay === 'search';

  useEffect(() => {
    if (open) {
      setQuery('');
      setHits([]);
      setTookMs(null);
      setSelected(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    if (timer.current) clearTimeout(timer.current);
    const q = query.trim();
    if (!q) {
      setHits([]);
      setTookMs(null);
      return;
    }
    timer.current = setTimeout(() => {
      void rpc<{ results: SearchHit[]; tookMs: number }>(Methods.search.query, {
        query: q,
        limit: 30,
      })
        .then((res) => {
          setHits(res.results);
          setTookMs(res.tookMs);
        })
        .catch(() => setHits([]));
    }, 120);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [query, open]);

  const items: CommandListItem[] = useMemo(() => {
    const out: CommandListItem[] = [];
    const q = query.trim().toLowerCase();
    for (const cmd of commands) {
      if (cmd.hidden) continue;
      if (q && !cmd.title.toLowerCase().includes(q) && !cmd.id.toLowerCase().includes(q)) continue;
      out.push({
        id: `cmd:${cmd.id}`,
        title: cmd.title,
        subtitle: cmd.pluginId ?? 'EigenDesk core',
        group: 'Commands',
      });
    }
    for (const hit of hits) {
      out.push({
        id: `hit:${hit.uri}`,
        title: hit.title,
        subtitle: hit.snippet ?? hit.pluginId,
        detail: hit.pluginId,
        group: 'Workspace',
      });
    }
    return out.slice(0, 40);
  }, [commands, hits, query]);

  useEffect(() => setSelected(0), [query]);

  if (!open) return null;

  const pick = (item: CommandListItem) => {
    setOverlay(null);
    if (item.id.startsWith('cmd:')) {
      void useApp.getState().executeCommand(item.id.slice(4));
      return;
    }
    const uri = item.id.slice(4);
    const hit = hits.find((h) => h.uri === uri);
    if (!hit) return;
    void rpc(Methods.artifacts.markOpened, { uri });
    const plugin = useApp.getState().plugins.find((p) => p.manifest.id === hit.pluginId);
    const view = plugin?.manifest.contributes?.views?.find(
      (v) => (v.location ?? 'main') === 'main',
    );
    if (plugin && view) {
      useApp.getState().openTab({
        kind: 'plugin-view',
        title: view.title,
        pluginId: plugin.manifest.id,
        viewId: view.id,
      });
    }
    void rpc(Methods.events.emit, {
      name: 'artifact.open',
      data: { uri, pluginId: hit.pluginId },
    });
  };

  return (
    <div className="ed-overlay">
      <div className="ed-overlay-backdrop" onMouseDown={() => setOverlay(null)} />
      <div className="ed-palette" role="dialog" aria-label="Universal search">
        <input
          ref={inputRef}
          value={query}
          placeholder="Search workspace…"
          aria-label="Search query"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault();
              setSelected((s) => Math.min(s + 1, items.length - 1));
            } else if (e.key === 'ArrowUp') {
              e.preventDefault();
              setSelected((s) => Math.max(s - 1, 0));
            } else if (e.key === 'Enter') {
              e.preventDefault();
              const item = items[selected];
              if (item) pick(item);
            } else if (e.key === 'Escape') {
              setOverlay(null);
            }
          }}
        />
        <CommandList
          items={items}
          selected={selected}
          onSelectedChange={setSelected}
          onPick={pick}
          empty={query ? 'No results' : 'Type to search the workspace'}
        />
        {tookMs !== null && (
          <div
            style={{
              padding: '4px 12px',
              borderTop: '1px solid var(--ed-border)',
              color: 'var(--ed-muted)',
              fontSize: 'var(--ed-text-xs)',
            }}
          >
            {hits.length} result{hits.length === 1 ? '' : 's'} · {tookMs.toFixed(1)} ms
          </div>
        )}
      </div>
    </div>
  );
}
