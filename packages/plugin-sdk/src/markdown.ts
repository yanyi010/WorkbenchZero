/**
 * Safe Markdown rendering for plugin surfaces: marked → DOMPurify → DOM
 * nodes. Never uses innerHTML with unsanitized input, and link targets are
 * restricted to http(s) so plugin content cannot open exotic schemes.
 */
import { marked } from 'marked';
import DOMPurify from 'dompurify';

marked.setOptions({ gfm: true, breaks: true });

export function renderMarkdown(source: string): HTMLElement {
  const raw = marked.parse(source, { async: false });
  const html = DOMPurify.sanitize(raw, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: ['style', 'form', 'input', 'iframe', 'script'],
    FORBID_ATTR: ['onerror', 'onclick', 'onload'],
  });
  const host = document.createElement('div');
  host.className = 'ed-markdown';
  host.innerHTML = html;

  // Harden links: only http(s), open via the host (system browser), and
  // never let plugin markdown navigate its own iframe.
  for (const a of host.querySelectorAll('a')) {
    const href = a.getAttribute('href') ?? '';
    if (/^https?:\/\//i.test(href)) {
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
    } else {
      a.removeAttribute('href');
      a.style.textDecoration = 'line-through';
      a.title = 'blocked link scheme';
    }
  }
  return host;
}
