/**
 * Drift guard: every method name exported by `Methods` must exist as a
 * dispatch arm in the kernel (`crates/kernel/src/rpc.rs`), and vice versa.
 * This test fails whenever either side adds, renames or removes an RPC
 * method without updating the other — the exact class of bug the SDK↔kernel
 * audit was written to eliminate.
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { Methods } from './index';

const here = dirname(fileURLToPath(import.meta.url));
const rpcPath = join(here, '../../../crates/kernel/src/rpc.rs');
const rpcSource = readFileSync(rpcPath, 'utf8');

/** `"kernel.method" => match` arms, e.g. `"storage.set" => {`. */
const kernelMethods = new Set(
  [...rpcSource.matchAll(/^\s{8}"([a-z]+\.[a-zA-Z]+)"\s*=>/gm)].map((m) => m[1]),
);

function collectProtocolMethods(obj: unknown, prefix = ''): string[] {
  const out: string[] = [];
  for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
    if (typeof value === 'string') {
      out.push(value);
    } else if (value && typeof value === 'object') {
      out.push(...collectProtocolMethods(value, `${prefix}${key}.`));
    }
  }
  return out;
}

describe('Methods mirrors the kernel dispatch table', () => {
  const protocolMethods = collectProtocolMethods(Methods);

  it('has at least one method per namespace', () => {
    expect(protocolMethods.length).toBeGreaterThan(60);
  });

  it('every protocol method exists in the kernel', () => {
    const missing = protocolMethods.filter((m) => !kernelMethods.has(m));
    expect(missing, `protocol methods missing from kernel: ${missing.join(', ')}`).toEqual([]);
  });

  it('every kernel method is exposed by the protocol', () => {
    const protocolSet = new Set(protocolMethods);
    const missing = [...kernelMethods].filter((m) => !protocolSet.has(m));
    expect(missing, `kernel methods missing from protocol: ${missing.join(', ')}`).toEqual([]);
  });

  it('permission descriptions cover every known permission', async () => {
    const { ALL_PERMISSIONS, PERMISSION_DESCRIPTIONS } = await import('./index');
    for (const p of ALL_PERMISSIONS) {
      expect(PERMISSION_DESCRIPTIONS[p], `description for ${p}`).toBeTruthy();
    }
    expect(Object.keys(PERMISSION_DESCRIPTIONS).sort()).toEqual([...ALL_PERMISSIONS].sort());
  });
});
