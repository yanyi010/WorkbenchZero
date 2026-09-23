import { describe, expect, it } from 'vitest';
import { parseMemo, serializeMemo } from './frontmatter';

describe('memo frontmatter', () => {
  it('round-trips serialize → parse', () => {
    const m = {
      title: 'Berry convergence "notes"',
      tags: ['math', 'semantics'],
      createdAt: '2026-09-23T00:00:00.000Z',
      updatedAt: '2026-09-23T01:00:00.000Z',
      body: 'The estimator is consistent under mild conditions.\n',
    };
    const again = parseMemo(serializeMemo(m));
    expect(again.title).toBe(m.title);
    expect(again.tags).toEqual(m.tags);
    expect(again.createdAt).toBe(m.createdAt);
    expect(again.updatedAt).toBe(m.updatedAt);
    expect(again.body).toBe(m.body);
  });

  it('quotes survive in titles', () => {
    const again = parseMemo(serializeMemo({ title: 'a "quoted" title', tags: [], createdAt: '', updatedAt: '', body: '' }));
    expect(again.title).toBe('a "quoted" title');
  });

  it('files without frontmatter fall back to the first line', () => {
    const m = parseMemo('# Heading\n\nBody text\n');
    expect(m.title).toBe('Heading');
    expect(m.body).toBe('# Heading\n\nBody text\n');
  });

  it('empty documents become Untitled', () => {
    expect(parseMemo('').title).toBe('Untitled');
    expect(parseMemo('\n\n  \n').title).toBe('Untitled');
  });

  it('tag lists tolerate spaces and semicolons', () => {
    const m = parseMemo('---\ntitle: t\ntags: [a, b;c ]\n---\nbody');
    expect(m.tags).toEqual(['a', 'b', 'c']);
  });

  it('trailing newline is normalized on write', () => {
    const out = serializeMemo({ title: 't', tags: [], createdAt: '', updatedAt: '', body: 'no newline' });
    expect(out.endsWith('\n')).toBe(true);
  });
});
