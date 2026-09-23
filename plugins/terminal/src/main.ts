/**
 * Terminal plugin (spec §62–63): xterm frontend over the kernel PTY
 * service. Processes belong to session scope — the session id never
 * leaves this plugin, and killing the view kills the session.
 */
import { definePlugin } from '@eigendesk/plugin-sdk';
import { h, render } from '@eigendesk/plugin-sdk';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import type { PluginContext } from '@eigendesk/plugin-sdk';
import type { PtySessionInfo } from '@eigendesk/protocol';

interface TermSession {
  info: PtySessionInfo;
  term: Terminal;
  fit: FitAddon;
  container: HTMLElement;
  disposers: (() => void)[];
}

let ctx: PluginContext;
let sessions: TermSession[] = [];
let activeIndex = -1;

function host(): HTMLElement | null {
  return document.getElementById('term-host');
}

function tabsBar(): HTMLElement | null {
  return document.getElementById('term-tabs');
}

function drawTabs(): void {
  const bar = tabsBar();
  if (!bar) return;
  render(
    bar,
    [
      ...sessions.map((s, i) =>
        h(
        'span',
        {
          class: 'term-tab',
          'data-active': String(i === activeIndex),
          role: 'tab',
          tabindex: '0',
          onclick: () => activate(i),
          onkeydown: (ev: KeyboardEvent) => {
            if (ev.key === 'Enter') void activate(i);
          },
        },
          `${s.info.alive === false ? '⨯ ' : ''}${s.info.shell.split('/').pop() ?? 'sh'}`,
          h(
            'span',
            {
              class: 'close',
              role: 'button',
              'aria-label': 'Close session',
              onclick: (ev: Event) => {
                ev.stopPropagation();
                void close(i);
              },
            },
            '✕',
          ),
        ),
      ),
      h(
        'span',
        {
          class: 'term-new',
          role: 'button',
          tabindex: '0',
          onclick: () => void createSession(),
          onkeydown: (ev: KeyboardEvent) => {
            if (ev.key === 'Enter') void createSession();
          },
        },
        '+ new',
      ),
    ],
  );
}

function activate(index: number): void {
  if (index < 0 || index >= sessions.length) return;
  activeIndex = index;
  const h = host();
  if (!h) return;
  for (const s of sessions) s.container.style.display = 'none';
  const s = sessions[index];
  s.container.style.display = 'block';
  s.term.focus();
  requestAnimationFrame(() => {
    try {
      s.fit.fit();
    } catch {
      /* not measurable yet */
    }
  });
  drawTabs();
}

async function createSession(): Promise<void> {
  const ws = await ctx.workspace.current();
  const shellSetting = (await ctx.settings.get<string>('eigendesk.terminal.shell')) ?? '';
  const fontSize = (await ctx.settings.get<number>('eigendesk.terminal.fontSize')) ?? 13;

  const term = new Terminal({
    fontFamily: 'var(--ed-font-mono), monospace',
    fontSize,
    cursorBlink: true,
    allowProposedApi: true,
    theme: {
      background: '#1e1e2a',
      foreground: '#e6e6ef',
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  const container = document.createElement('div');
  container.style.height = '100%';
  container.style.display = 'none';
  const h = host();
  if (!h) return;
  h.appendChild(container);
  term.open(container);

  const info = await ctx.pty.create({
    cwd: ws?.root,
    shell: shellSetting || undefined,
    cols: Math.max(20, Math.floor(container.clientWidth / 9)),
    rows: Math.max(5, Math.floor(container.clientHeight / 18)),
  });

  const disposers: (() => void)[] = [];
  disposers.push(
    ctx.pty.onData(info.sessionId, (data) => term.write(data)),
    ctx.pty.onExit(info.sessionId, () => {
      term.write('\r\n\x1b[90m[process exited]\x1b[0m\r\n');
      drawTabs();
    }),
  );
  term.onData((data) => void ctx.pty.write(info.sessionId, data));
  const resize = () => {
    try {
      fit.fit();
      void ctx.pty.resize(info.sessionId, term.cols, term.rows);
    } catch {
      /* fit needs dimensions */
    }
  };
  const observer = new ResizeObserver(resize);
  observer.observe(container);
  disposers.push(() => observer.disconnect());

  sessions.push({ info, term, fit, container, disposers });
  activate(sessions.length - 1);
}

async function close(index: number): Promise<void> {
  const s = sessions[index];
  if (!s) return;
  if (s.info.alive !== false && !confirm('Kill this terminal session?')) return;
  sessions = sessions.filter((x) => x !== s);
  try {
    await ctx.pty.kill(s.info.sessionId);
  } catch {
    /* already dead */
  }
  for (const off of s.disposers) off();
  s.term.dispose();
  s.container.remove();
  if (activeIndex >= sessions.length) activeIndex = sessions.length - 1;
  if (activeIndex >= 0) activate(activeIndex);
  else drawTabs();
}

definePlugin({
  async activate(context) {
    ctx = context;
    if (ctx.surface !== 'view:eigendesk.terminal.main') {
      // Logic frame: nothing to do — sessions are view-scoped (spec §63).
      return;
    }
    render(
      document.getElementById('app')!,
      h(
        'div',
        { class: 'term-app' },
        h('div', { class: 'term-tabs', id: 'term-tabs', role: 'tablist' }),
        h('div', { class: 'term-host', id: 'term-host' }),
      ),
    );
    drawTabs();
    await createSession();

    // Housekeeping: reap sessions killed outside this view.
    window.setInterval(() => {
      void ctx.pty.list().then((list) => {
        let changed = false;
        for (const s of sessions) {
          const current = list.find((x) => x.sessionId === s.info.sessionId);
          if (current?.alive === false && s.info.alive !== false) {
            s.info.alive = false;
            changed = true;
          }
        }
        if (changed) drawTabs();
      });
    }, 5000);
  },
  deactivate() {
    for (const s of sessions) {
      void ctx.pty.kill(s.info.sessionId).catch(() => {});
      for (const off of s.disposers) off();
      s.term.dispose();
    }
    sessions = [];
  },
});
