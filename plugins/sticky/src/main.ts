/**
 * Sticky plugin — small persistent cards (spec §60).
 *
 * Cards are Markdown files under `<workspace>/Stickies/` so they survive
 * plugin reinstalls and stay inspectable. Checklists are plain `- [ ]`
 * task lists rendered by the preview pane. Conversion commands hand the
 * text to the Memo / Tasks plugins via events — Sticky stays decoupled.
 */
import { definePlugin, h, render } from '@eigendesk/plugin-sdk';
import { renderMarkdown } from '@eigendesk/plugin-sdk/markdown';
import type { PluginContext } from '@eigendesk/plugin-sdk';
import type { ReadDirResult } from '@eigendesk/protocol';

interface StickyCard {
  uri: string;
  title: string;
  body: string;
  color: string;
  updatedAt: string;
}

const COLORS = ['#f6c344', '#7cc77c', '#6db3f2', '#ef8f6d', '#c792ea', '#e05555'];
const FRONTMATTER_RE = /^---\n([\s\S]*?)\n---\n?/;

let ctx: PluginContext;
let cards: StickyCard[] = [];
let scanSeq = 0;

function dir(root: string): string {
  return `${root.replace(/\/+$/, '')}/Stickies`;
}

function uid(): string {
  return Math.random().toString(36).slice(2, 8);
}

function serialize(c: StickyCard): string {
  return [
    '---',
    `title: ${JSON.stringify(c.title)}`,
    `color: ${JSON.stringify(c.color)}`,
    `updatedAt: ${c.updatedAt}`,
    '---',
    '',
    c.body.endsWith('\n') || c.body === '' ? c.body : `${c.body}\n`,
  ].join('\n');
}

function parse(raw: string, uri: string): StickyCard {
  const card: StickyCard = { uri, title: 'Sticky', body: raw, color: COLORS[0], updatedAt: '' };
  const m = FRONTMATTER_RE.exec(raw);
  if (m) {
    card.body = raw.slice(m[0].length);
    for (const line of m[1].split('\n')) {
      const idx = line.indexOf(':');
      if (idx < 0) continue;
      const key = line.slice(0, idx).trim();
      const value = line.slice(idx + 1).trim();
      if (key === 'title') card.title = value.replace(/^["']|["']$/g, '');
      if (key === 'color' && /^#[0-9a-f]{6}$/i.test(value)) card.color = value;
      if (key === 'updatedAt') card.updatedAt = value;
    }
  }
  return card;
}

async function scan(): Promise<void> {
  const seq = ++scanSeq;
  const ws = await ctx.workspace.current();
  if (!ws) {
    cards = [];
    return;
  }
  let listing: ReadDirResult;
  try {
    listing = await ctx.fs.readDir(dir(ws.root));
  } catch {
    cards = [];
    draw();
    return;
  }
  const found: StickyCard[] = [];
  for (const entry of listing.entries) {
    if (!entry.isFile || !entry.name.endsWith('.md')) continue;
    const uri = `${dir(ws.root)}/${entry.name}`;
    try {
      const file = await ctx.fs.readFile(uri);
      found.push(parse(file.content, uri));
    } catch {
      /* skip unreadable */
    }
  }
  if (seq !== scanSeq) return;
  cards = found.sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : -1));
  void ctx.search.upsert(
    cards.map((c) => ({ uri: c.uri, title: c.title, body: c.body.slice(0, 2000), metadata: { kind: 'sticky' } })),
  );
  draw();
}

async function create(text: string): Promise<StickyCard> {
  const ws = await ctx.workspace.current();
  if (!ws) throw new Error('no workspace open');
  try {
    await ctx.fs.mkdir(dir(ws.root));
  } catch {
    /* exists */
  }
  const card: StickyCard = {
    uri: `${dir(ws.root)}/sticky-${uid()}.md`,
    title: text.trim().split('\n')[0].slice(0, 60) || 'Sticky',
    body: text.trim() ? `${text.trim()}\n` : '',
    color: COLORS[cards.length % COLORS.length],
    updatedAt: new Date().toISOString(),
  };
  await ctx.fs.writeFile(card.uri, serialize(card));
  await scan();
  return card;
}

async function save(card: StickyCard): Promise<void> {
  card.updatedAt = new Date().toISOString();
  await ctx.fs.writeFile(card.uri, serialize(card));
}

function cardNode(c: StickyCard): HTMLElement {
  const titleInput = h('input', {
    value: c.title,
    'aria-label': 'Sticky title',
    oninput: (ev: Event) => {
      c.title = (ev.target as HTMLInputElement).value;
    },
  });
  const body = h('textarea', {
    'aria-label': 'Sticky body (Markdown, - [ ] for checklist)',
    oninput: (ev: Event) => {
      c.body = (ev.target as HTMLTextAreaElement).value;
    },
    onblur: () => void save(c),
  }) as HTMLTextAreaElement;
  body.value = c.body;

  const preview = h('div', { class: 'preview' });
  let showingPreview = false;
  const toggleBtn = h('button', {
    onclick: () => {
      showingPreview = !showingPreview;
      toggleBtn.textContent = showingPreview ? 'Edit' : 'Preview';
      body.style.display = showingPreview ? 'none' : 'block';
      preview.style.display = showingPreview ? 'block' : 'none';
      if (showingPreview) preview.replaceChildren(renderMarkdown(c.body));
    },
  });
  toggleBtn.textContent = 'Preview';

  const colorDot = h('button', {
    class: 'color-dot',
    style: `background:${c.color}`,
    title: 'Change color',
    'aria-label': 'Change color',
    onclick: () => {
      c.color = COLORS[(COLORS.indexOf(c.color) + 1) % COLORS.length];
      colorDot.style.background = c.color;
      void save(c);
    },
  });

  return h(
    'div',
    { class: 'sticky-card', 'data-uri': c.uri },
    h('header', {}, titleInput, colorDot),
    body,
    preview,
    h(
      'footer',
      {},
      toggleBtn,
      h(
        'button',
        {
          title: 'Convert to memo',
          onclick: () => void ctx.events.emit('memo.createFromText', { text: `# ${c.title}\n\n${c.body}` }),
        },
        '→ Memo',
      ),
      h(
        'button',
        {
          title: 'Convert to task',
          onclick: () => void ctx.events.emit('task.createFromText', { text: `${c.title}\n${c.body}` }),
        },
        '→ Task',
      ),
      h(
        'button',
        {
          'data-variant': 'danger',
          title: 'Delete sticky',
          onclick: () => {
            if (!confirm(`Delete sticky “${c.title}”?`)) return;
            void (async () => {
              await ctx.fs.delete(c.uri);
              await scan();
            })();
          },
        },
        'Delete',
      ),
    ),
  );
}

function draw(): void {
  const app = document.getElementById('app');
  if (!app || !ctx.surface.startsWith('view:')) return;
  render(
    app,
    h(
      'div',
      { class: 'sticky-board' },
      cards.map(cardNode),
      h(
        'div',
        {
          class: 'sticky-new',
          role: 'button',
          tabindex: '0',
          onclick: () => void create(''),
          onkeydown: (ev: KeyboardEvent) => {
            if (ev.key === 'Enter') (ev.currentTarget as HTMLElement).click();
          },
        },
        '+ New sticky',
      ),
    ),
  );
}

definePlugin({
  async activate(context) {
    ctx = context;

    ctx.commands.onCommand((id, args) => {
      if (id === 'eigendesk.sticky.new') return create(args ?? '');
      if (id === 'eigendesk.sticky.convertToMemo') {
        const card = cards.find((c) => c.uri === args);
        return ctx.events.emit('memo.createFromText', {
          text: card ? `# ${card.title}\n\n${card.body}` : (args ?? ''),
        });
      }
      if (id === 'eigendesk.sticky.convertToTask') {
        const card = cards.find((c) => c.uri === args);
        return ctx.events.emit('task.createFromText', {
          text: card ? `${card.title}\n${card.body}` : (args ?? ''),
        });
      }
      return undefined;
    });

    ctx.events.on('sticky.createFromText', (data) => {
      const text = typeof (data as { text?: string })?.text === 'string' ? (data as { text: string }).text : '';
      if (text.trim()) void create(text);
    });

    ctx.events.on('workspace.opened', () => void scan());

    await scan();
  },
});
