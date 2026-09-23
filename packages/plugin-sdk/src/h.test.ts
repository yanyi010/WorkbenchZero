import { describe, expect, it } from 'vitest';
import { h, render } from './h';

describe('h()', () => {
  it('creates elements with tag and classes', () => {
    const el = h('div.note.pinned');
    expect(el.tagName).toBe('DIV');
    expect(el.classList.contains('note')).toBe(true);
    expect(el.classList.contains('pinned')).toBe(true);
  });

  it('sets attributes, dataset and style', () => {
    const el = h('span', {
      id: 'x',
      title: 'hello',
      hidden: true,
      dataset: { kind: 'badge' },
      style: { color: 'red' },
    });
    expect(el.id).toBe('x');
    expect(el.title).toBe('hello');
    expect(el.hidden).toBe(true);
    expect(el.dataset.kind).toBe('badge');
    expect((el as HTMLElement).style.color).toBe('red');
  });

  it('wires event listeners', () => {
    let clicked = 0;
    const el = h('button', { onclick: () => clicked++ }, 'Go');
    el.click();
    el.click();
    expect(clicked).toBe(2);
    expect(el.textContent).toBe('Go');
  });

  it('appends nested children, arrays, skips null/false and renders numbers', () => {
    const el = h(
      'ul',
      null,
      h('li', null, 'a'),
      [h('li', null, 'b'), null, false, h('li', null, 'c')],
      undefined,
      0,
    );
    expect(el.children.length).toBe(3);
    expect(el.textContent).toBe('abc0');
  });

  it('render() replaces content', () => {
    const target = h('div', null, 'old');
    render(target, h('p', null, 'new'));
    expect(target.children.length).toBe(1);
    expect(target.textContent).toBe('new');
  });

  it('escapes text content (never innerHTML)', () => {
    const el = h('div', null, '<script>alert(1)</script>');
    expect(el.innerHTML).toBe('&lt;script&gt;alert(1)&lt;/script&gt;');
    expect(el.children.length).toBe(0);
  });
});
