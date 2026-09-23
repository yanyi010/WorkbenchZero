/**
 * Memo frontmatter: a deliberately tiny `key: value` block — no YAML
 * library, no surprise parses. Round-trips what serialize() writes.
 */

export interface MemoMeta {
  title: string;
  tags: string[];
  createdAt: string;
  updatedAt: string;
  body: string;
}

const FRONTMATTER_RE = /^---\n([\s\S]*?)\n---\n?/;

export function parseMemo(raw: string): MemoMeta {
  const meta: MemoMeta = { title: '', tags: [], createdAt: '', updatedAt: '', body: raw };
  const match = FRONTMATTER_RE.exec(raw);
  if (!match) {
    const first = raw.split('\n').find((l) => l.trim());
    meta.title = first ? first.replace(/^#+\s*/, '').slice(0, 80) : 'Untitled';
    return meta;
  }
  meta.body = raw.slice(match[0].length);
  for (const line of match[1].split('\n')) {
    const idx = line.indexOf(':');
    if (idx < 0) continue;
    const key = line.slice(0, idx).trim();
    const value = line.slice(idx + 1).trim();
    if (key === 'title') meta.title = unquote(value);
    else if (key === 'tags')
      meta.tags = value
        .replace(/^\[|\]$/g, '')
        .split(/[,;]/)
        .map((t) => t.trim())
        .filter(Boolean);
    else if (key === 'createdAt') meta.createdAt = value;
    else if (key === 'updatedAt') meta.updatedAt = value;
  }
  if (!meta.title) {
    const first = meta.body.split('\n').find((l) => l.trim());
    meta.title = first ? first.replace(/^#+\s*/, '').slice(0, 80) : 'Untitled';
  }
  return meta;
}

function unquote(value: string): string {
  if (value.startsWith('"') && value.endsWith('"') && value.length >= 2) {
    try {
      const parsed = JSON.parse(value) as unknown;
      if (typeof parsed === 'string') return parsed;
    } catch {
      /* fall through to raw */
    }
  }
  return value.replace(/^['"]|['"]$/g, '');
}

export function serializeMemo(m: MemoMeta): string {
  const fm = [
    '---',
    `title: ${JSON.stringify(m.title)}`,
    `createdAt: ${m.createdAt}`,
    `updatedAt: ${m.updatedAt}`,
    m.tags.length ? `tags: [${m.tags.join(', ')}]` : null,
    '---',
    '',
  ]
    .filter((x): x is string => x !== null)
    .join('\n');
  const body = m.body.endsWith('\n') || m.body === '' ? m.body : `${m.body}\n`;
  return `${fm}${body}`;
}
