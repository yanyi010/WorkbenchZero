/**
 * Minimal hyperscript DOM helper so plugin UIs can be written without any
 * framework or build step (spec §5: plugins choose their own stack; the SDK
 * must work from a plain `<script type="module">`).
 *
 *   h('div.note', { onclick: fn }, [h('strong', 'hello'), ' world'])
 */
export type Child = Node | string | number | null | undefined | false | Child[];

export type Attrs = Record<string, unknown>;

function appendChild(parent: Node, child: Child): void {
  if (child === null || child === undefined || child === false) return;
  if (Array.isArray(child)) {
    for (const c of child) appendChild(parent, c);
    return;
  }
  if (child instanceof Node) {
    parent.appendChild(child);
    return;
  }
  parent.appendChild(document.createTextNode(String(child)));
}

export function h(
  tagAndClass: string,
  attrs?: Attrs | null,
  ...children: Child[]
): HTMLElement {
  const [tag, ...classes] = tagAndClass.split('.');
  const el = document.createElement(tag || 'div');
  for (const cls of classes) if (cls) el.classList.add(cls);
  if (attrs) {
    for (const [key, value] of Object.entries(attrs)) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'class' && typeof value === 'string') {
        el.classList.add(...value.split(/\s+/).filter(Boolean));
      } else if (key === 'style' && typeof value === 'object') {
        Object.assign(el.style, value);
      } else if (key.startsWith('on') && typeof value === 'function') {
        el.addEventListener(key.slice(2), value as EventListener);
      } else if (key === 'dataset' && typeof value === 'object') {
        Object.assign(el.dataset, value as Record<string, string>);
      } else if (value === true) {
        el.setAttribute(key, '');
      } else {
        el.setAttribute(key, String(value as string | number));
      }
    }
  }
  for (const child of children) appendChild(el, child);
  return el;
}

/** Convenience: clear an element and (optionally) refill it. */
export function render(target: HTMLElement, child: Child): void {
  target.replaceChildren();
  appendChild(target, child);
}

export { appendChild };
