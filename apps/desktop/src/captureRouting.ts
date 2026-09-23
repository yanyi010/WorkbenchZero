/**
 * Quick Capture routing (spec §18) — pure logic, extracted for testing.
 *
 * Rules:
 *  - a leading token matching a provider prefix routes to that provider
 *    (longest prefix wins, then priority),
 *  - plain text routes to the highest-priority provider without prefixes,
 *  - core never depends on a specific plugin.
 */
import type { CaptureProviderContribution } from '@eigendesk/protocol';

export interface CaptureRouteInput {
  pluginId: string;
  contribution: CaptureProviderContribution;
}

export interface ResolvedCapture {
  pluginId: string;
  contribution: CaptureProviderContribution;
  /** Text with the routing prefix stripped. */
  text: string;
}

export function resolveCaptureProvider(
  input: string,
  providers: CaptureRouteInput[],
): ResolvedCapture | null {
  const trimmed = input.trim();
  if (!trimmed) return null;

  const withPrefix: ResolvedCapture[] = [];
  for (const { pluginId, contribution } of providers) {
    const matched = contribution.prefixes
      .filter((pfx) => trimmed.startsWith(pfx + ' ') || trimmed === pfx)
      .sort((a, b) => b.length - a.length)[0];
    if (matched) {
      withPrefix.push({
        pluginId,
        contribution,
        text: trimmed.slice(matched.length).trim(),
      });
    }
  }
  if (withPrefix.length > 0) {
    withPrefix.sort(
      (a, b) => (b.contribution.priority ?? 100) - (a.contribution.priority ?? 100),
    );
    return withPrefix[0];
  }

  const plain = providers
    .filter(({ contribution }) => contribution.prefixes.length === 0)
    .sort((a, b) => (b.contribution.priority ?? 100) - (a.contribution.priority ?? 100));
  if (plain.length > 0) {
    return { pluginId: plain[0].pluginId, contribution: plain[0].contribution, text: trimmed };
  }
  return null;
}
