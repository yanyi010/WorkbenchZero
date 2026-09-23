/**
 * Hello Plugin — the ecosystem quality metric (spec §88): a minimal
 * plugin in under 100 lines that still exercises the full stack —
 * command, view, search index, notification.
 */
import { definePlugin, h, render } from '@eigendesk/plugin-sdk';

definePlugin({
  async activate(ctx) {
    let greetings = 0;

    ctx.commands.onCommand((id, args) => {
      if (id !== 'community.hello-plugin.hello') return undefined;
      greetings += 1;
      const who = (args ?? 'world').trim() || 'world';
      void ctx.notify.show({ title: 'Hello', body: `Hello, ${who}! (${greetings})` });
      void ctx.search.upsert({
        uri: `hello://${greetings}`,
        title: `Hello, ${who}`,
        body: `Greeting #${greetings} at ${new Date().toISOString()}`,
        metadata: { kind: 'hello' },
      });
      return `Hello, ${who}!`;
    });

    if (ctx.surface === 'view:community.hello-plugin.main') {
      render(
        document.getElementById('app')!,
        h(
          'div',
          { style: 'padding:16px;font-family:var(--ed-font-ui)' },
          h('h2', {}, `Hello from ${ctx.pluginId}`),
          h('p', {}, 'This view runs sandboxed on the edp:// origin with zero host privileges beyond the manifest.'),
          h('p', {}, 'Try the command “Hello: Greet” from the palette (Ctrl+K).'),
        ),
      );
    }
  },
});
