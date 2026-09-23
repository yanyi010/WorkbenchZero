/**
 * QuickCalc — arithmetic in Quick Capture with zero permissions:
 * a permission-free plugin showcase (spec §30: plugins run with least
 * privilege).
 */
import { definePlugin } from '@eigendesk/plugin-sdk';
import { evaluate } from './calc';

definePlugin({
  activate(ctx) {
    ctx.commands.onCommand((id, args) => {
      if (id !== 'community.quickcalc.eval') return undefined;
      const expression = (args ?? '').trim();
      if (!expression) return undefined;
      const value = evaluate(expression);
      return `${expression} = ${value}`;
    });
  },
});
