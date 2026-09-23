/**
 * Memo plugin — the default universal capture destination (spec §59).
 *
 * Data model: one Markdown file per memo under `<workspace>/<Memos dir>/`,
 * with a small frontmatter block (id, title, createdAt, updatedAt, tags).
 * File names are `YYYYMMDD-HHmmss-<slug>.md` so the directory stays
 * human-sortable even outside the app.
 */
import { definePlugin, h, render } from '@eigendesk/plugin-sdk';
import { renderMarkdown } from '@eigendesk/plugin-sdk/markdown';
import type { PluginContext } from '@eigendesk/plugin-sdk';
import type { ReadDirResult } from '@eigendesk/protocol';
import { parseMemo, serializeMemo } from './frontmatter';

interface Memo {
  uri: string;
  title: string;
  body: string;
  createdAt: string;
  updatedAt: string;
  tags: string[];
}

const SLUG_RE = /[^a-z0-9]+/gi;

let ctx: PluginContext;
let memos: Memo[] = [];
let activeUri: string | null = null;
let query = '';
let scanSeq = 0;
let dirName = 'Memos';

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

function memosDir(root: string): string {
  return `${root.replace(/\/+$/, '')}/${dirName}`;
}

function slugify(title: string): string {
  const slug = title.trim().replace(SLUG_RE, '-').replace(/^-+|-+$/g, '').toLowerCase();
  return slug.slice(0, 40) || 'memo';
}

function pad(n: number): string {
  return String(n).padStart(2, '0');
}

function stamp(d = new Date()): string {
  return `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}-${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}${pad(d.getUTCSeconds())}`;
}

function parseFrontmatter(raw: string): ReturnType<typeof parseMemo> {
  return parseMemo(raw);
}

function serialize(m: Memo): string {
  return serializeMemo(m);
}

function sortMemos(list: Memo[]): Memo[] {
  return [...list].sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : -1));
}

// ---------------------------------------------------------------------------
// storage + index
// ---------------------------------------------------------------------------

async function scanWorkspace(): Promise<void> {
  const seq = ++scanSeq;
  dirName = (await ctx.settings.get<string>('eigendesk.memo.directory')) ?? 'Memos';
  const ws = await ctx.workspace.current();
  if (!ws) {
    memos = [];
    return;
  }
  let dir: ReadDirResult;
  try {
    dir = await ctx.fs.readDir(memosDir(ws.root));
  } catch {
    memos = [];
    return;
  }
  const found: Memo[] = [];
  for (const entry of dir.entries) {
    if (!entry.isFile || !entry.name.endsWith('.md')) continue;
    const uri = `${memosDir(ws.root)}/${entry.name}`;
    try {
      const file = await ctx.fs.readFile(uri);
      const meta = parseFrontmatter(file.content);
      found.push({
        uri,
        title: meta.title,
        body: meta.body,
        createdAt: meta.createdAt,
        updatedAt: meta.updatedAt,
        tags: meta.tags,
      });
    } catch (err) {
      ctx.log('warn', `unreadable memo ${entry.name}: ${String(err)}`);
    }
  }
  if (seq !== scanSeq) return; // superseded by a newer scan
  memos = sortMemos(found);
  void reindex();
}

async function reindex(): Promise<void> {
  const docs = memos.map((m) => ({
    uri: m.uri,
    title: m.title,
    body: m.body.slice(0, 4000),
    tags: m.tags,
    metadata: { kind: 'memo' },
  }));
  await ctx.search.upsert(docs);
  await ctx.artifacts.upsert(
    memos.map((m) => ({
      uri: m.uri,
      type: 'memo',
      title: m.title,
      metadata: { updatedAt: m.updatedAt, tags: m.tags },
    })),
  );
}

async function persist(m: Memo): Promise<void> {
  await ctx.fs.writeFile(m.uri, serialize(m));
  await scanWorkspace();
}

async function createMemo(text: string): Promise<Memo> {
  const ws = await ctx.workspace.current();
  if (!ws) throw new Error('no workspace open');
  const dir = memosDir(ws.root);
  try {
    await ctx.fs.mkdir(dir);
  } catch {
    /* exists */
  }
  const now = new Date().toISOString();
  const title = text.trim().split('\n')[0].slice(0, 80) || 'Untitled';
  const m: Memo = {
    uri: `${dir}/${stamp()}-${slugify(title)}.md`,
    title,
    body: text.trim() ? `${text.trim()}\n` : '',
    createdAt: now,
    updatedAt: now,
    tags: [],
  };
  await persist(m);
  await ctx.notify.show({ title: 'Memo saved', body: title });
  return m;
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

function relTime(iso: string): string {
  if (!iso) return '';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso.slice(0, 10);
  const diff = Date.now() - t;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(t).toLocaleDateString();
}

function renderMemoRow(m: Memo): HTMLElement {
  return h(
    'div',
    {
      class: 'memo-item',
      'data-active': String(m.uri === activeUri),
      role: 'button',
      tabindex: '0',
      onclick: () => {
        activeUri = m.uri;
        void ctx.artifacts.markOpened(m.uri);
        drawMain();
      },
      onkeydown: (ev: KeyboardEvent) => {
        if (ev.key === 'Enter') (ev.currentTarget as HTMLElement).click();
      },
    },
    h('div', { class: 'title' }, m.title),
    h('div', { class: 'meta' }, [relTime(m.updatedAt), m.tags.join(' · ')].filter(Boolean).join(' · ')),
  );
}

function renderEditor(active: Memo | null): HTMLElement {
  if (!active) {
    return h('div', { class: 'memo-empty' }, 'Select or create a memo');
  }
  let preview = false;
  const titleInput = h('input', {
    value: active.title,
    'aria-label': 'Memo title',
    oninput: (ev: Event) => {
      active.title = (ev.target as HTMLInputElement).value;
    },
  });
  const bodyArea = h('textarea', {
    'aria-label': 'Memo body (Markdown)',
    oninput: (ev: Event) => {
      active.body = (ev.target as HTMLTextAreaElement).value;
    },
  }) as HTMLTextAreaElement;
  bodyArea.value = active.body;

  const previewPane = h('div', { class: 'preview' });
  const toggle = h('button', {
    onclick: () => {
      preview = !preview;
      toggle.textContent = preview ? 'Edit' : 'Preview';
      previewPane.replaceChildren(preview ? renderMarkdown(active.body) : h(''));
      previewPane.style.display = preview ? 'block' : 'none';
      bodyArea.style.display = preview ? 'none' : 'block';
    },
  });
  toggle.textContent = 'Preview';

  const save = async () => {
    active.updatedAt = new Date().toISOString();
    await persist(active);
    drawMain();
  };

  return h(
    'div',
    { class: 'memo-editor' },
    h(
      'div',
      { class: 'toolbar' },
      titleInput,
      toggle,
      h('button', { 'data-variant': 'primary', onclick: () => void save() }, 'Save'),
      h(
        'button',
        {
          'data-variant': 'danger',
          onclick: () => {
            if (!activeUri) return;
            if (!confirm(`Delete memo “${active.title}”?`)) return;
            void (async () => {
              await ctx.fs.delete(activeUri);
              await ctx.artifacts.remove(activeUri);
              activeUri = null;
              await scanWorkspace();
              drawMain();
            })();
          },
        },
        'Delete',
      ),
    ),
    h('div', { class: 'memo-body' }, bodyArea, previewPane),
  );
}

function filtered(): Memo[] {
  if (!query.trim()) return memos;
  const q = query.toLowerCase();
  return memos.filter(
    (m) => m.title.toLowerCase().includes(q) || m.body.toLowerCase().includes(q) || m.tags.some((t) => t.toLowerCase().includes(q)),
  );
}

function drawMain(): void {
  const app = document.getElementById('app');
  if (!app) return;
  const active = memos.find((m) => m.uri === activeUri) ?? null;

  const search = h('input', {
    placeholder: 'Filter memos…',
    value: query,
    'aria-label': 'Filter memos',
    oninput: (ev: Event) => {
      query = (ev.target as HTMLInputElement).value;
      const list = document.querySelector<HTMLElement>('.memo-list .items');
      if (list) render(list, filtered().map(renderMemoRow));
    },
  });
  // Re-focus + restore caret after redraw.
  const focusSearch = query && document.activeElement?.getAttribute('aria-label') === 'Filter memos';
  render(
    app,
    h(
      'div',
      { class: 'memo-app' },
      h(
        'div',
        { class: 'memo-list' },
        h('header', {}, h('strong', {}, 'Memos'), search),
        h('div', { class: 'items' }, filtered().map(renderMemoRow)),
      ),
      renderEditor(active),
    ),
  );
  if (focusSearch) {
    const el = document.querySelector<HTMLInputElement>('.memo-list input');
    if (el) {
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    }
  }
}

function drawWidget(): void {
  const app = document.getElementById('app');
  if (!app) return;
  const recent = memos.slice(0, 6);
  render(
    app,
    h(
      'div',
      { class: 'memo-widget' },
      h('h4', {}, 'Recent Memos'),
      recent.length === 0
        ? h('div', { style: 'color: var(--ed-muted)' }, 'No memos yet')
        : h(
            'ul',
            {},
            recent.map((m) =>
              h(
                'li',
                {
                  title: m.title,
                  role: 'button',
                  onclick: () => {
                    void ctx.artifacts.markOpened(m.uri);
                    void ctx.events.emit('openArtifact', { uri: m.uri, type: 'memo' });
                  },
                },
                m.title,
              ),
            ),
          ),
    ),
  );
}

// ---------------------------------------------------------------------------
// plugin definition
// ---------------------------------------------------------------------------

definePlugin({
  async activate(context) {
    ctx = context;

    // Commands from the manifest (Quick Capture lands here).
    ctx.commands.onCommand((id, args) => {
      if (id === 'eigendesk.memo.new') {
        return createMemo(args ?? '');
      }
      if (id === 'eigendesk.memo.open') {
        return scanWorkspace().then(drawMain);
      }
      return undefined;
    });

    // Cross-plugin requests (AI plugin “Save as Memo”, sticky conversion).
    ctx.events.on('memo.createFromText', (data) => {
      const text = typeof (data as { text?: string })?.text === 'string' ? (data as { text: string }).text : '';
      if (text.trim()) void createMemo(text);
    });

    ctx.events.on('workspace.opened', () => {
      activeUri = null;
      void scanWorkspace().then(() => {
        if (ctx.surface === 'view:eigendesk.memo.main') drawMain();
        else if (ctx.surface.startsWith('widget:')) drawWidget();
      });
    });

    ctx.events.on('artifact.open', (data) => {
      const uri = (data as { uri?: string })?.uri;
      if (!uri || !uri.endsWith('.md')) return;
      if (memos.some((m) => m.uri === uri)) {
        activeUri = uri;
        drawMain();
      }
    });

    if (ctx.surface === 'logic') {
      // Background frame: keep the index warm, nothing to render.
      await scanWorkspace();
      return;
    }
    await scanWorkspace();
    if (ctx.surface === 'view:eigendesk.memo.main') drawMain();
    else if (ctx.surface.startsWith('widget:')) drawWidget();
  },
});
