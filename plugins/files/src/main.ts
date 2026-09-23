/**
 * Files plugin (spec §58): workspace tree, open, rename, move, delete,
 * preview. Core exposes only permission-gated fs APIs — everything here
 * is built on them, like any third-party plugin would.
 */
import { definePlugin, h, render } from '@workbench-zero/plugin-sdk';
import { renderMarkdown } from '@workbench-zero/plugin-sdk/markdown';
import type { PluginContext } from '@workbench-zero/plugin-sdk';
import type { ReadDirResult } from '@workbench-zero/protocol';

interface TreeNode {
  name: string;
  path: string;
  isDir: boolean;
  expanded: boolean;
  children: TreeNode[] | null;
}

const HIDDEN = new Set(['.git', 'node_modules', '.workbench-zero', '__pycache__', 'target', 'dist']);
const TEXT_MAX = 2 * 1024 * 1024;

const ICONS: Record<string, string> = {
  md: '📝',
  txt: '📄',
  json: '{}',
  rs: '🦀',
  ts: 'TS',
  tsx: 'TS',
  js: 'JS',
  py: '🐍',
  sh: '$',
  toml: '⚙',
  yaml: '⚙',
  yml: '⚙',
  csv: '▦',
  png: '🖼',
  jpg: '🖼',
  jpeg: '🖼',
  gif: '🖼',
  svg: '🖼',
  pdf: '📕',
};

let ctx: PluginContext;
let rootPath = '';
let tree: TreeNode | null = null;
let selectedPath: string | null = null;
let preview: { path: string; kind: 'text' | 'markdown' | 'binary'; content: string } | null = null;
const loading = false;

function iconFor(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  return ICONS[ext] ?? (ext ? ext.slice(0, 3) : '📄');
}

function nodeFor(path: string, entries: ReadDirResult['entries']): TreeNode[] {
  return entries
    .filter((e) => !e.name.startsWith('.') && !HIDDEN.has(e.name))
    .sort((a, b) => (a.isDir === b.isDir ? a.name.localeCompare(b.name) : a.isDir ? -1 : 1))
    .map((e) => ({
      name: e.name,
      path: `${path}/${e.name}`,
      isDir: e.isDir,
      expanded: false,
      children: null,
    }));
}

async function expand(node: TreeNode): Promise<void> {
  if (!node.isDir || node.children) return;
  const dir = await ctx.fs.readDir(node.path);
  node.children = nodeFor(node.path, dir.entries);
  node.expanded = true;
  draw();
}

async function buildRoot(): Promise<void> {
  const ws = await ctx.workspace.current();
  if (!ws) {
    tree = null;
    draw();
    return;
  }
  rootPath = ws.root.replace(/\/+$/, '');
  const root: TreeNode = { name: ws.name, path: rootPath, isDir: true, expanded: true, children: null };
  tree = root;
  try {
    const dir = await ctx.fs.readDir(rootPath);
    root.children = nodeFor(rootPath, dir.entries);
  } catch (err) {
    ctx.log('warn', `workspace read failed: ${String(err)}`);
  }
  draw();
}

async function openFile(path: string): Promise<void> {
  selectedPath = path;
  try {
    const stat = await ctx.fs.stat(path);
    if (stat.size > TEXT_MAX) {
      preview = { path, kind: 'binary', content: `(file too large: ${stat.size} bytes)` };
      draw();
      return;
    }
    const file = await ctx.fs.readFile(path);
    if (path.endsWith('.md')) preview = { path, kind: 'markdown', content: file.content };
    else preview = { path, kind: 'text', content: file.content };
  } catch (err) {
    preview = { path, kind: 'text', content: `cannot read file: ${String(err)}` };
  }
  draw();
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

function treeNodeRow(node: TreeNode, depth: number): HTMLElement {
  const row = h(
    'div',
    {
      class: 'tree-row',
      'data-selected': String(node.path === selectedPath),
      style: `padding-left:${depth * 14 + 6}px`,
      role: node.isDir ? 'button' : 'button',
      tabindex: '0',
      onclick: () => {
        if (node.isDir) {
          node.expanded = !node.expanded;
          void expand(node);
          if (!node.expanded) draw();
        } else {
          void openFile(node.path);
        }
      },
      onkeydown: (ev: KeyboardEvent) => {
        if (ev.key === 'Enter') (ev.currentTarget as HTMLElement).click();
      },
    },
    node.isDir
      ? h('span', { class: 'caret' }, node.expanded ? '▾' : '▸')
      : h('span', { class: 'caret' }),
    h('span', { class: 'icon' }, node.isDir ? (node.expanded ? '📂' : '📁') : iconFor(node.name)),
    h('span', {}, node.name),
  );
  const out = [row];
  if (node.isDir && node.expanded && node.children) {
    for (const child of node.children) out.push(treeNodeRow(child, depth + 1));
  } else if (node.isDir && node.expanded && !node.children) {
    out.push(h('div', { class: 'tree-row', style: `padding-left:${(depth + 1) * 14 + 18}px;color:var(--ed-muted)` }, '…'));
  }
  return h('div', {}, out);
}

function confirmDialog(message: string, defaultValue = ''): string | null {
  // Native prompt/confirm keep v0.1 honest (no dialog framework here).
  return prompt(message, defaultValue);
}

function previewHeader(path: string): HTMLElement {
  const name = path.split('/').pop() ?? path;
  return h(
    'header',
    {},
    h('span', { class: 'path', title: path }, path),
    h(
      'button',
      {
        title: 'Rename',
        onclick: () => {
          const next = confirmDialog(`Rename “${name}” to:`, name);
          if (!next || next === name) return;
          const parent = path.split('/').slice(0, -1).join('/');
          void (async () => {
            await ctx.fs.move(path, `${parent}/${next}`);
            selectedPath = `${parent}/${next}`;
            await buildRoot();
            await openFile(selectedPath);
          })();
        },
      },
      'Rename',
    ),
    h(
      'button',
      {
        title: 'Move to folder',
        onclick: () => {
          const dest = confirmDialog(`Move “${name}” to (workspace-relative folder):`);
          if (!dest) return;
          void (async () => {
            const target = `${rootPath}/${dest.replace(/^\/+/, '')}/${name}`;
            await ctx.fs.move(path, target);
            selectedPath = target;
            await buildRoot();
            await openFile(target);
          })();
        },
      },
      'Move',
    ),
    h(
      'button',
      {
        title: 'Reveal in system file manager',
        onclick: () => void ctx.workspace.reveal(path),
      },
      'Reveal',
    ),
    h(
      'button',
      {
        'data-variant': 'danger',
        title: 'Delete',
        onclick: () => {
          if (!confirm(`Delete “${name}”?`)) return;
          void (async () => {
            await ctx.fs.delete(path, false);
            selectedPath = null;
            preview = null;
            await buildRoot();
          })();
        },
      },
      'Delete',
    ),
  );
}

function draw(): void {
  const app = document.getElementById('app');
  if (!app || ctx.surface !== 'view:zero.files.main') return;

  const previewPane = preview
    ? h(
        'div',
        { class: 'files-preview' },
        previewHeader(preview.path),
        h(
          'div',
          { class: 'content' },
          preview.kind === 'markdown'
            ? renderMarkdown(preview.content)
            : h('pre', {}, preview.content),
        ),
      )
    : h('div', { class: 'files-empty' }, loading ? 'Reading workspace…' : 'Select a file to preview');

  render(
    app,
    h(
      'div',
      { class: 'files-app' },
      h('div', { class: 'files-tree' }, tree ? treeNodeRow(tree, 0) : h('div', { class: 'files-loading' }, 'No workspace')),
      previewPane,
    ),
  );
}

definePlugin({
  async activate(context) {
    ctx = context;
    ctx.commands.onCommand((id) => {
      if (id === 'zero.files.open') return buildRoot();
      return undefined;
    });
    ctx.events.on('workspace.opened', () => {
      selectedPath = null;
      preview = null;
      void buildRoot();
    });
    await buildRoot();
  },
});
