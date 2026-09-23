import { describe, expect, it } from 'vitest';
import { resolveCaptureProvider } from './captureRouting';
import { resolveBindings } from './keybindings';
import { fuzzyScore } from './components/Palette';

const memo = {
  pluginId: 'eigendesk.memo',
  contribution: { id: 'memo', prefixes: [], priority: 100, command: 'eigendesk.memo.new' },
};
const tasks = {
  pluginId: 'eigendesk.tasks',
  contribution: { id: 'task', prefixes: ['/t', 'task:'], priority: 90, command: 'eigendesk.tasks.quickAdd' },
};
const sticky = {
  pluginId: 'eigendesk.sticky',
  contribution: { id: 'sticky', prefixes: ['/s'], priority: 80, command: 'eigendesk.sticky.new' },
};
const ask = {
  pluginId: 'eigendesk.ai',
  contribution: { id: 'ask', prefixes: ['?'], priority: 95 },
};

describe('capture routing (spec §18)', () => {
  const providers = [memo, tasks, sticky, ask];

  it('routes by prefix and strips it from the text', () => {
    const r = resolveCaptureProvider('/t buy milk', providers);
    expect(r?.pluginId).toBe('eigendesk.tasks');
    expect(r?.text).toBe('buy milk');
  });

  it('supports multi-character prefixes', () => {
    const r = resolveCaptureProvider('task: call advisor', providers);
    expect(r?.pluginId).toBe('eigendesk.tasks');
    expect(r?.text).toBe('call advisor');
  });

  it('routes question marks to the ask provider', () => {
    const r = resolveCaptureProvider('? why singular', providers);
    expect(r?.pluginId).toBe('eigendesk.ai');
    expect(r?.text).toBe('why singular');
  });

  it('plain text falls back to the prefix-less provider (memo default)', () => {
    const r = resolveCaptureProvider('remember to check Berry convergence', providers);
    expect(r?.pluginId).toBe('eigendesk.memo');
    expect(r?.text).toBe('remember to check Berry convergence');
  });

  it('returns null for empty or whitespace input', () => {
    expect(resolveCaptureProvider('', providers)).toBeNull();
    expect(resolveCaptureProvider('   ', providers)).toBeNull();
  });

  it('returns null when no provider is installed (core must not assume memo)', () => {
    expect(resolveCaptureProvider('hello', [])).toBeNull();
    expect(resolveCaptureProvider('/t hello', [])).toBeNull();
  });

  it('prefix without trailing text yields empty text', () => {
    const r = resolveCaptureProvider('/t', providers);
    expect(r?.pluginId).toBe('eigendesk.tasks');
    expect(r?.text).toBe('');
  });

  it('highest priority wins among prefix-less providers', () => {
    const low = { pluginId: 'a.low', contribution: { id: 'low', prefixes: [], priority: 10 } };
    const r = resolveCaptureProvider('note', [low, memo]);
    expect(r?.pluginId).toBe('eigendesk.memo');
  });
});

describe('keybindings (spec §74)', () => {
  const commands = [
    {
      id: 'core.showPalette',
      title: 'Palette',
      keywords: [],
      takesArgs: false,
      defaultKeybinding: 'Ctrl+K',
      pluginId: null,
    },
    {
      id: 'memo.new',
      title: 'New memo',
      keywords: [],
      takesArgs: true,
      defaultKeybinding: 'Ctrl+Alt+M',
      pluginId: 'eigendesk.memo',
    },
  ];

  it('defaults come from the command registry', () => {
    const bindings = resolveBindings(commands, {});
    expect(bindings.find((b) => b.accel === 'Ctrl+K')?.commandId).toBe('core.showPalette');
  });

  it('user overrides win over defaults', () => {
    const bindings = resolveBindings(commands, { 'core.showPalette': 'Ctrl+Shift+K' });
    expect(bindings.find((b) => b.accel === 'Ctrl+Shift+K')?.commandId).toBe('core.showPalette');
    expect(bindings.find((b) => b.accel === 'Ctrl+K')).toBeUndefined();
  });

  it('normalizes accelerator spellings', () => {
    const bindings = resolveBindings([], { 'x.y': 'cmd+p' });
    expect(bindings.find((b) => b.accel === 'Cmd+P')).toBeTruthy();
  });
});

describe('fuzzy scoring', () => {
  it('ranks exact and prefix matches highest', () => {
    expect(fuzzyScore('new', 'new')).toBeGreaterThan(fuzzyScore('new', 'New memo'));
    expect(fuzzyScore('new', 'New memo')).toBeGreaterThan(0);
  });

  it('matches subsequences with word-boundary bonus', () => {
    // 'm' right after a space scores higher than the same letter mid-word.
    expect(fuzzyScore('am', 'xxa mo')).toBeGreaterThan(fuzzyScore('am', 'xxamo'));
  });

  it('returns 0 for non-matches and 1 for empty query', () => {
    expect(fuzzyScore('zzz', 'New memo')).toBe(0);
    expect(fuzzyScore('', 'anything')).toBe(1);
  });
});
