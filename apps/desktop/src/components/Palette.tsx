import React, { useEffect, useMemo, useRef, useState } from 'react';
import type { ArtifactRecord } from '@workbench-zero/protocol';
import type { CommandListItem } from '@workbench-zero/ui-kit';
import { Methods } from '@workbench-zero/protocol';
import { CommandList } from '@workbench-zero/ui-kit';
import { useApp } from '../store';
import { rpc } from '../kernel';

/**
 * Command Palette (spec §16): fuzzy search over commands, views, recent
 * artifacts and plugin actions. Local search must feel instant (<50 ms
 * warm) — scoring is a lightweight subsequence match.
 */

/** Fuzzy subsequence score; higher is better, 0 = no match. */
export function fuzzyScore(query: string, text: string): number {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!q) return 1;
  if (t === q) return 1000;
  if (t.startsWith(q)) return 500 - (t.length - q.length);
  let qi = 0;
  let score = 0;
  let streak = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      qi++;
      streak++;
      score += 10 + streak * 2;
      if (ti === 0 || t[ti - 1] === ' ' || t[ti - 1] === '.' || t[ti - 1] === '/') {
        score += 8; // word-boundary bonus
      }
    } else {
      streak = 0;
    }
  }
  return qi === q.length ? score : 0;
}

export function Palette() {
  const overlay = useApp((s) => s.overlay);
  const commands = useApp((s) => s.commands);
  const plugins = useApp((s) => s.plugins);
  const setOverlay = useApp((s) => s.setOverlay);
  const executeCommand = useApp((s) => s.executeCommand);
  const openTab = useApp((s) => s.openTab);

  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState(0);
  const [recent, setRecent] = useState<ArtifactRecord[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);

  const open = overlay === 'palette';

  useEffect(() => {
    if (open) {
      setQuery('');
      setSelected(0);
      requestAnimationFrame(() => inputRef.current?.focus());
      rpc<ArtifactRecord[]>(Methods.artifacts.listRecent, { limit: 5 })
        .then(setRecent)
        .catch(() => setRecent([]));
    }
  }, [open]);

  const items: CommandListItem[] = useMemo(() => {
    const scored: { item: CommandListItem; score: number }[] = [];

    for (const cmd of commands) {
      if (cmd.hidden) continue;
      const score = Math.max(
        fuzzyScore(query, cmd.title),
        fuzzyScore(query, cmd.id),
        ...(cmd.keywords ?? []).map((k) => fuzzyScore(query, k) * 0.6),
      );
      if (query && score <= 0) continue;
      scored.push({
        score: score || 1,
        item: {
          id: `cmd:${cmd.id}`,
          title: cmd.title,
          subtitle: cmd.pluginId ?? 'Workbench Zero core',
          detail: cmd.category ?? undefined,
          group: 'Commands',
          keyhint: cmd.defaultKeybinding ?? undefined,
        },
      });
    }

    for (const p of plugins) {
      if (p.state !== 'enabled' && p.state !== 'active') continue;
      for (const v of p.manifest.contributes?.views ?? []) {
        const score = Math.max(fuzzyScore(query, v.title), fuzzyScore(query, v.id));
        if (query && score <= 0) continue;
        scored.push({
          score: score || 1,
          item: {
            id: `view:${p.manifest.id}/${v.id}`,
            title: v.title,
            subtitle: p.manifest.name,
            group: 'Views',
          },
        });
      }
    }

    for (const a of recent) {
      const score = Math.max(fuzzyScore(query, a.title ?? a.uri), fuzzyScore(query, a.uri));
      if (query && score <= 0) continue;
      scored.push({
        score: score || 1,
        item: {
          id: `artifact:${a.uri}`,
          title: a.title ?? a.uri,
          subtitle: a.type,
          group: 'Recent',
        },
      });
    }

    scored.sort((a, b) => b.score - a.score);
    return scored.slice(0, 50).map((s) => s.item);
  }, [commands, plugins, recent, query]);

  useEffect(() => {
    setSelected(0);
  }, [query]);

  if (!open) return null;

  const pick = (item: CommandListItem) => {
    setOverlay(null);
    if (item.id.startsWith('cmd:')) {
      const cmd = commands.find((c) => `cmd:${c.id}` === item.id);
      if (cmd?.takesArgs) {
        // Argument-taking commands continue in the capture input.
        setOverlay('capture');
        return;
      }
      void executeCommand(item.id.slice(4));
    } else if (item.id.startsWith('view:')) {
      const [, rest] = item.id.split(':');
      const [pluginId, viewId] = rest.split('/');
      openTab({
        kind: 'plugin-view',
        title: typeof item.title === 'string' ? item.title : '',
        pluginId,
        viewId,
      });
    } else if (item.id.startsWith('artifact:')) {
      const uri = item.id.slice('artifact:'.length);
      const hit = recent.find((a) => a.uri === uri);
      if (hit) {
        void rpc(Methods.artifacts.markOpened, { uri });
        void rpc(Methods.events.emit, {
          name: 'artifact.open',
          data: { uri, pluginId: hit.pluginId },
        });
      }
    }
  };

  const onKey = (e: React.KeyboardEvent) => {
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
  };

  return (
    <div className="ed-overlay">
      <div className="ed-overlay-backdrop" onMouseDown={() => setOverlay(null)} />
      <div className="ed-palette" role="dialog" aria-label="Command palette">
        <input
          ref={inputRef}
          value={query}
          placeholder="Type a command…"
          aria-label="Search commands"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
        />
        <CommandList
          items={items}
          selected={selected}
          onSelectedChange={setSelected}
          onPick={pick}
          empty="No matching commands"
        />
      </div>
    </div>
  );
}
