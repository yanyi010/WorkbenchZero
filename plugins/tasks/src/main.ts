/**
 * Tasks plugin (spec §61): title, status, due date, priority, tags,
 * project, notes. One JSON document per workspace: `Tasks/tasks.json`.
 * Intentionally not Jira.
 */
import { definePlugin, h, render } from '@workbench-zero/plugin-sdk';
import type { PluginContext } from '@workbench-zero/plugin-sdk';

type Status = 'todo' | 'doing' | 'done';
type Priority = 'low' | 'medium' | 'high';

interface Task {
  id: string;
  title: string;
  status: Status;
  due: string | null; // YYYY-MM-DD
  priority: Priority;
  tags: string[];
  project: string | null;
  notes: string;
  createdAt: string;
  completedAt: string | null;
}

let ctx: PluginContext;
let tasks: Task[] = [];
let selectedId: string | null = null;
let filter: 'today' | 'all' | 'done' = 'today';
let projectFilter = '';

function tasksPath(root: string): string {
  return `${root.replace(/\/+$/, '')}/Tasks/tasks.json`;
}

function uid(): string {
  return `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

function today(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function isToday(t: Task): boolean {
  return t.status !== 'done' && t.due !== null && t.due <= today();
}

function sortKey(t: Task): string {
  return [t.status === 'done' ? 'zz' : 'aa', t.due ?? '9999-99-99', t.title].join('|');
}

async function load(): Promise<void> {
  const ws = await ctx.workspace.current();
  if (!ws) {
    tasks = [];
    return;
  }
  const path = tasksPath(ws.root);
  // A *corrupt* tasks file (unparseable, or valid-but-wrong JSON) must
  // never be silently reset → overwritten by the next persist. Quarantine
  // it (copy aside, remove the unreadable original) and tell the user
  // where their data went.
  let corrupt = false;
  try {
    const file = await ctx.fs.readFile(path);
    const parsed = JSON.parse(file.content) as Task[];
    if (Array.isArray(parsed)) {
      tasks = parsed;
      return;
    }
    corrupt = true;
  } catch {
    corrupt = true;
  }
  tasks = [];
  const exists = await ctx.fs.stat(path).catch(() => null);
  if (corrupt && exists?.isFile) {
    const quarantine = `${path}.corrupt-${Date.now()}`;
    try {
      await ctx.fs.copy(path, quarantine);
      await ctx.fs.delete(path);
      await ctx.notify.show({
        title: 'Tasks file was unreadable',
        body: `A safety copy was saved to ${quarantine}. Starting with an empty list.`,
      });
    } catch {
      /* best effort — the original is still on disk */
    }
  }
}

async function persist(): Promise<void> {
  const ws = await ctx.workspace.current();
  if (!ws) throw new Error('no workspace open');
  const path = tasksPath(ws.root);
  try {
    await ctx.fs.mkdir(`${ws.root.replace(/\/+$/, '')}/Tasks`);
  } catch {
    /* exists */
  }
  await ctx.fs.writeFile(path, JSON.stringify(tasks, null, 2) + '\n');
  await reindex();
  draw();
}

async function reindex(): Promise<void> {
  await ctx.search.upsert(
    tasks.map((t) => ({
      uri: `task://${t.id}`,
      title: t.title,
      body: `${t.notes}\n${t.tags.join(' ')}${t.project ?? ''}`.trim(),
      tags: [...t.tags, ...(t.project ? [t.project] : [])],
      metadata: { kind: 'task' },
    })),
  );
  await ctx.artifacts.upsert(
    tasks
      .filter((t) => t.status !== 'done')
      .map((t) => ({
        uri: `task://${t.id}`,
        type: 'task',
        title: t.title,
        metadata: { due: t.due, priority: t.priority, project: t.project },
      })),
  );
}

async function quickAdd(text: string): Promise<Task> {
  // Parse `!high @project #tag ~2026-01-31` decorations from raw text.
  let title = text.trim();
  let priority: Priority = 'medium';
  let project: string | null = null;
  const tags: string[] = [];
  let due: string | null = null;

  title = title.replace(/(^|\s)!(high|medium|low)\b/gi, (_all, _sp, p: string) => {
    priority = p.toLowerCase() as Priority;
    return ' ';
  });
  title = title.replace(/(^|\s)@([\w-]+)/g, (_all, _sp, proj: string) => {
    project = proj;
    return ' ';
  });
  title = title.replace(/(^|\s)#([\w-]+)/g, (_all, _sp, tag: string) => {
    tags.push(tag);
    return ' ';
  });
  title = title.replace(/(^|\s)~(\d{4}-\d{2}-\d{2})\b/g, (_all, _sp, date: string) => {
    due = date;
    return ' ';
  });
  title = title.replace(/\s+/g, ' ').trim();
  if (!title) title = 'Untitled task';

  const task: Task = {
    id: uid(),
    title,
    status: 'todo',
    due,
    priority,
    tags,
    project,
    notes: '',
    createdAt: new Date().toISOString(),
    completedAt: null,
  };
  tasks.push(task);
  await persist();
  return task;
}

async function toggle(id: string): Promise<void> {
  const t = tasks.find((x) => x.id === id);
  if (!t) return;
  if (t.status === 'done') {
    t.status = 'todo';
    t.completedAt = null;
  } else {
    t.status = 'done';
    t.completedAt = new Date().toISOString();
    await ctx.notify.show({ title: 'Task completed', body: t.title });
  }
  await persist();
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

function dueLabel(t: Task): { text: string; overdue: boolean } | null {
  if (!t.due) return null;
  const overdue = t.status !== 'done' && t.due < today();
  const text = t.due === today() ? 'today' : t.due;
  return { text, overdue };
}

function taskRow(t: Task): HTMLElement {
  const due = dueLabel(t);
  return h(
    'div',
    { class: 'task-row', 'data-status': t.status, role: 'listitem' },
    h('input', {
      type: 'checkbox',
      checked: t.status === 'done',
      'aria-label': `Complete ${t.title}`,
      onchange: () => void toggle(t.id),
    }),
    h(
      'span',
      {
        class: 'title',
        role: 'button',
        tabindex: '0',
        onclick: () => {
          selectedId = selectedId === t.id ? null : t.id;
          draw();
        },
      },
      t.title,
    ),
    h(
      'span',
      { class: 'meta' },
      t.priority !== 'medium' ? h('span', { class: 'prio', 'data-p': t.priority }, t.priority) : null,
      t.project ? h('span', {}, `@${t.project}`) : null,
      t.tags.map((tag) => h('span', {}, `#${tag}`)),
      due ? h('span', { class: 'due', 'data-overdue': String(due.overdue) }, due.text) : null,
      h(
        'button',
        {
          title: 'Delete task',
          'aria-label': `Delete ${t.title}`,
          onclick: () => {
            if (!confirm(`Delete task “${t.title}”?`)) return;
            tasks = tasks.filter((x) => x.id !== t.id);
            if (selectedId === t.id) selectedId = null;
            void persist();
          },
        },
        '✕',
      ),
    ),
  );
}

function detailPane(t: Task): HTMLElement {
  const save = () => void persist();
  const input = (attrs: Record<string, unknown>) => h('input', attrs);

  const notes = h('textarea', {
    'aria-label': 'Task notes',
    oninput: (ev: Event) => {
      t.notes = (ev.target as HTMLTextAreaElement).value;
    },
    onblur: save,
  }) as HTMLTextAreaElement;
  notes.value = t.notes;

  return h(
    'div',
    { class: 'task-detail' },
    h('label', {}, 'Status'),
    h(
      'select',
      {
        onchange: (ev: Event) => {
          t.status = (ev.target as HTMLSelectElement).value as Status;
          t.completedAt = t.status === 'done' ? new Date().toISOString() : null;
          save();
        },
      },
      (['todo', 'doing', 'done'] as Status[]).map((s) =>
        h('option', { value: s, selected: s === t.status }, s),
      ),
    ),
    h('label', {}, 'Due'),
    input({
      type: 'date',
      value: t.due ?? '',
      'aria-label': 'Due date',
      onchange: (ev: Event) => {
        t.due = (ev.target as HTMLInputElement).value || null;
        save();
      },
    }),
    h('label', {}, 'Priority'),
    h(
      'select',
      {
        onchange: (ev: Event) => {
          t.priority = (ev.target as HTMLSelectElement).value as Priority;
          save();
        },
      },
      (['low', 'medium', 'high'] as Priority[]).map((p) =>
        h('option', { value: p, selected: p === t.priority }, p),
      ),
    ),
    h('label', {}, 'Project'),
    input({
      type: 'text',
      value: t.project ?? '',
      placeholder: '—',
      'aria-label': 'Project',
      oninput: (ev: Event) => {
        t.project = (ev.target as HTMLInputElement).value || null;
      },
      onblur: save,
    }),
    h('label', {}, 'Tags'),
    input({
      type: 'text',
      value: t.tags.join(', '),
      placeholder: 'comma, separated',
      'aria-label': 'Tags',
      oninput: (ev: Event) => {
        t.tags = (ev.target as HTMLInputElement).value.split(',').map((s) => s.trim()).filter(Boolean);
      },
      onblur: save,
    }),
    h('label', {}, 'Notes'),
    notes,
  );
}

function visible(): Task[] {
  let list = tasks;
  if (projectFilter) list = list.filter((t) => t.project === projectFilter);
  if (filter === 'today') list = list.filter(isToday);
  if (filter === 'all') list = list.filter((t) => t.status !== 'done');
  if (filter === 'done') list = list.filter((t) => t.status === 'done');
  return [...list].sort((a, b) => sortKey(a).localeCompare(sortKey(b)));
}

function draw(): void {
  const app = document.getElementById('app');
  if (!app) return;
  if (ctx.surface.startsWith('widget:')) {
    const todayTasks = tasks.filter(isToday).sort((a, b) => sortKey(a).localeCompare(sortKey(b))).slice(0, 8);
    render(
      app,
      h(
        'div',
        { class: 'tasks-widget' },
        h('h4', {}, 'Today'),
        todayTasks.length === 0
          ? h('div', { style: 'color: var(--ed-muted)' }, 'Nothing due today')
          : h(
              'ul',
              {},
              todayTasks.map((t) =>
                h(
                  'li',
                  { class: t.status === 'done' ? 'done' : '' },
                  h('input', {
                    type: 'checkbox',
                    checked: t.status === 'done',
                    'aria-label': `Complete ${t.title}`,
                    onchange: () => void toggle(t.id),
                  }),
                  h('span', {}, t.title),
                ),
              ),
            ),
      ),
    );
    return;
  }
  if (ctx.surface !== 'view:zero.tasks.main') return;

  const projects = [...new Set(tasks.map((t) => t.project).filter((p): p is string => !!p))].sort();
  const selected = tasks.find((t) => t.id === selectedId) ?? null;
  const quick = h('input', {
    type: 'text',
    placeholder: 'Add a task… (!high @project #tag ~2026-01-31)',
    'aria-label': 'Quick add task',
    onkeydown: (ev: KeyboardEvent) => {
      if (ev.key !== 'Enter') return;
      const value = (ev.target as HTMLInputElement).value;
      if (!value.trim()) return;
      (ev.target as HTMLInputElement).value = '';
      void quickAdd(value);
    },
  });

  render(
    app,
    h(
      'div',
      { class: 'tasks-app' },
      h(
        'div',
        { class: 'tasks-toolbar' },
        quick,
        h(
          'select',
          {
            'aria-label': 'Filter view',
            onchange: (ev: Event) => {
              filter = (ev.target as HTMLSelectElement).value as typeof filter;
              draw();
            },
          },
          h('option', { value: 'today', selected: filter === 'today' }, 'Today'),
          h('option', { value: 'all', selected: filter === 'all' }, 'Open'),
          h('option', { value: 'done', selected: filter === 'done' }, 'Done'),
        ),
        projects.length > 0
          ? h(
              'select',
              {
                'aria-label': 'Filter project',
                onchange: (ev: Event) => {
                  projectFilter = (ev.target as HTMLSelectElement).value;
                  draw();
                },
              },
              h('option', { value: '', selected: projectFilter === '' }, 'All projects'),
              projects.map((p) => h('option', { value: p, selected: p === projectFilter }, p)),
            )
          : null,
      ),
      h(
        'div',
        { class: 'tasks-list', role: 'list' },
        visible().length === 0
          ? h('div', { class: 'tasks-empty' }, 'No tasks here. Add one above ↑')
          : visible().map(taskRow),
      ),
      selected ? detailPane(selected) : null,
    ),
  );
}

definePlugin({
  async activate(context) {
    ctx = context;

    ctx.commands.onCommand((id, args) => {
      if (id === 'zero.tasks.quickAdd') return quickAdd(args ?? '');
      if (id === 'zero.tasks.open') {
        return load().then(draw);
      }
      return undefined;
    });

    ctx.events.on('task.createFromText', (data) => {
      const text = typeof (data as { text?: string })?.text === 'string' ? (data as { text: string }).text : '';
      if (text.trim()) void quickAdd(text);
    });

    ctx.events.on('workspace.opened', () => {
      selectedId = null;
      void load().then(draw);
    });

    ctx.events.on('artifact.open', (data) => {
      const uri = (data as { uri?: string })?.uri ?? '';
      if (!uri.startsWith('task://')) return;
      const id = uri.slice('task://'.length);
      if (tasks.some((t) => t.id === id)) {
        selectedId = id;
        filter = 'all';
        draw();
      }
    });

    await load();
    if (ctx.surface === 'view:zero.tasks.main' || ctx.surface.startsWith('widget:')) draw();
  },
});
