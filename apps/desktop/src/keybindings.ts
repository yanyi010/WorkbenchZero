/**
 * Keybinding resolution (spec §74): defaults come from the kernel command
 * registry; user overrides in the `core.keybindings` setting win. Shortcut
 * conflicts are surfaced, and plugins must not assume their default is
 * active.
 */
import type { CommandDef } from '@eigendesk/protocol';

export interface ResolvedBinding {
  accel: string;
  commandId: string;
}

/** User overrides: command id → accelerator. */
export type UserBindings = Record<string, string>;

/** Normalize an accelerator for matching: ctrl+shift+p → "Ctrl+Shift+P". */
function normalizeAccel(accel: string): string {
  return accel
    .split('+')
    .map((part) => {
      const p = part.trim().toLowerCase();
      if (p === 'cmd' || p === 'meta' || p === 'super') return 'Cmd';
      if (p === 'ctrl') return 'Ctrl';
      if (p === 'alt' || p === 'option' || p === 'opt') return 'Alt';
      if (p === 'shift') return 'Shift';
      if (p === 'space') return 'Space';
      if (p === 'esc' || p === 'escape') return 'Escape';
      if (p.length === 1) return p.toUpperCase();
      return p.charAt(0).toUpperCase() + p.slice(1);
    })
    .join('+');
}

/** Build the active binding table. User overrides replace the command's
 * default binding (the released default becomes available again). */
export function resolveBindings(
  commands: CommandDef[],
  user: UserBindings,
): ResolvedBinding[] {
  const byCommand = new Map<string, string>(); // commandId → normalized accel
  for (const cmd of commands) {
    if (cmd.defaultKeybinding) {
      byCommand.set(cmd.id, normalizeAccel(cmd.defaultKeybinding));
    }
  }
  for (const [commandId, accel] of Object.entries(user)) {
    if (!accel) continue;
    byCommand.set(commandId, normalizeAccel(accel));
  }
  // Two commands claiming one accel: later registration wins and the
  // earlier binding is dropped with a console warning (spec §74: shortcut
  // conflicts require visible resolution).
  const byAccel = new Map<string, ResolvedBinding>();
  const owner = new Map<string, string>(); // accel → commandId
  for (const [commandId, accel] of byCommand) {
    const existing = owner.get(accel);
    if (existing && existing !== commandId) {
      console.warn(`keybinding conflict: ${commandId} and ${existing} both want ${accel}`);
      continue;
    }
    owner.set(accel, commandId);
    byAccel.set(accel, { accel, commandId });
  }
  return [...byAccel.values()];
}

/** Translate a KeyboardEvent into a normalized accelerator. */
export function eventToAccel(e: KeyboardEvent): string | null {
  if (e.key === 'Control' || e.key === 'Shift' || e.key === 'Alt' || e.key === 'Meta') {
    return null; // modifier-only press
  }
  const parts: string[] = [];
  const isMac = navigator.platform.toUpperCase().includes('MAC');
  const ctrl = isMac ? e.metaKey : e.ctrlKey;
  if (ctrl) parts.push('Ctrl');
  if (e.shiftKey) parts.push('Shift');
  if (e.altKey) parts.push('Alt');
  let key = e.key;
  if (key === ' ') key = 'Space';
  if (key === 'Tab') key = 'Tab';
  if (key.length === 1) key = key.toUpperCase();
  else key = key.charAt(0).toUpperCase() + key.slice(1);
  parts.push(key);
  return parts.join('+');
}
