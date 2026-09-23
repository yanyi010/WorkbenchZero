/**
 * Pomodoro — a widget-first plugin: a focus timer that survives
 * tab switches because the countdown lives in session storage.
 */
import { definePlugin, h, render } from '@workbench-zero/plugin-sdk';
import type { PluginContext } from '@workbench-zero/plugin-sdk';

let ctx: PluginContext;
let endsAt = 0; // epoch ms; 0 = idle
let running = false;

function fmt(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  return `${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`;
}

async function restore(): Promise<void> {
  const saved = (await ctx.session.get('timer')) as { endsAt?: number; running?: boolean } | null;
  if (saved && typeof saved.endsAt === 'number') {
    endsAt = saved.endsAt;
    running = !!saved.running;
    if (running && endsAt <= Date.now()) {
      await finish();
    }
  }
}

async function persist(): Promise<void> {
  await ctx.session.set('timer', { endsAt, running });
}

async function finish(): Promise<void> {
  running = false;
  endsAt = 0;
  await persist();
  await ctx.notify.show({ title: 'Pomodoro', body: 'Focus session complete — take a break 🍅' });
  draw();
}

function start(): void {
  void (async () => {
    const minutes = (await ctx.settings.get<number>('community.pomodoro.focusMinutes')) ?? 25;
    endsAt = Date.now() + minutes * 60_000;
    running = true;
    await persist();
    draw();
  })();
}

async function reset(): Promise<void> {
  running = false;
  endsAt = 0;
  await persist();
  draw();
}

function draw(): void {
  const app = document.getElementById('app');
  if (!app) return;
  const remaining = endsAt > 0 ? endsAt - Date.now() : 0;
  const label = running ? fmt(remaining) : fmt(0);
  render(
    app,
    h(
      'div',
      {
        style:
          'height:100%;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;font-family:var(--ed-font-ui)',
      },
      h('div', { style: 'font-size:2.4em;font-variant-numeric:tabular-nums' }, label),
      h(
        'div',
        { style: 'display:flex;gap:8px' },
        running
          ? h('button', { onclick: () => void reset() }, 'Reset')
          : h('button', { onclick: start }, 'Start focus'),
      ),
    ),
  );
}

definePlugin({
  async activate(context) {
    ctx = context;

    ctx.commands.onCommand((id) => {
      if (id === 'community.pomodoro.start') {
        start();
        return 'started';
      }
      if (id === 'community.pomodoro.reset') {
        void reset();
        return 'reset';
      }
      return undefined;
    });

    await restore();
    if (ctx.surface.startsWith('widget:') || ctx.surface === 'view:community.pomodoro.main') {
      draw();
      window.setInterval(() => {
        if (!running) return;
        if (endsAt <= Date.now()) void finish();
        else draw();
      }, 1000);
    }
  },
});
