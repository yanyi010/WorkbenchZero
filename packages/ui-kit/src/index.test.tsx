/**
 * UI Kit tests: focus on the security- and a11y-critical paths —
 * Markdown sanitization, CommandList keyboard navigation, Dialog escape.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  CommandList,
  Dialog,
  Dropdown,
  Markdown,
  type CommandListItem,
} from './index';

afterEach(cleanup);

describe('Markdown', () => {
  it('renders basic markdown', () => {
    const { container } = render(<Markdown source={'# hi\n\nsome **bold** text'} />);
    const h1 = container.querySelector('h1');
    expect(h1?.textContent).toBe('hi');
    expect(container.querySelector('strong')?.textContent).toBe('bold');
  });

  it('strips script tags and event handlers', () => {
    const evil = [
      '# ok',
      '<script>alert(1)</script>',
      '<img src=x onerror="alert(2)">',
      '<iframe src="https://evil.example"></iframe>',
      '<a href="javascript:alert(3)">click</a>',
    ].join('\n');
    const { container } = render(<Markdown source={evil} />);
    const html = container.innerHTML.toLowerCase();
    expect(html).not.toContain('<script');
    expect(html).not.toContain('onerror');
    expect(html).not.toContain('<iframe');
    expect(html).not.toContain('javascript:');
    expect(container.textContent).toContain('ok');
  });

  it('caches renders for identical sources', () => {
    const { container: a } = render(<Markdown source="same" />);
    const { container: b } = render(<Markdown source="same" />);
    expect(a.innerHTML).toBe(b.innerHTML);
  });
});

describe('CommandList', () => {
  const items: CommandListItem[] = [
    { id: 'a', title: 'Alpha', subtitle: 'first', group: 'Commands' },
    { id: 'b', title: 'Beta', group: 'Commands' },
    { id: 'c', title: 'Gamma', group: 'Recent' },
  ];

  it('renders groups and items with aria roles', () => {
    render(
      <CommandList items={items} selected={0} onSelectedChange={() => {}} onPick={() => {}} />,
    );
    const listbox = screen.getByRole('listbox');
    expect(listbox.getAttribute('aria-label')).toBe('Results');
    const options = screen.getAllByRole('option');
    expect(options).toHaveLength(3);
    expect(options[0].getAttribute('aria-selected')).toBe('true');
    expect(screen.getByText('Commands')).toBeTruthy();
    expect(screen.getByText('Recent')).toBeTruthy();
  });

  it('mouse enter moves selection and click picks', () => {
    const onPick = vi.fn();
    const onSel = vi.fn();
    render(<CommandList items={items} selected={0} onSelectedChange={onSel} onPick={onPick} />);
    fireEvent.mouseEnter(screen.getByText('Gamma'));
    expect(onSel).toHaveBeenCalledWith(2);
    fireEvent.click(screen.getByText('Beta'));
    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ id: 'b' }));
  });

  it('shows the empty state', () => {
    render(
      <CommandList items={[]} selected={0} onSelectedChange={() => {}} onPick={() => {}} empty="nothing" />,
    );
    expect(screen.getByText('nothing')).toBeTruthy();
  });
});

describe('Dialog', () => {
  it('closes on Escape', () => {
    const onClose = vi.fn();
    render(
      <Dialog open onClose={onClose} title="Test">
        body
      </Dialog>,
    );
    expect(screen.getByRole('dialog')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('renders nothing when closed', () => {
    const { container } = render(
      <Dialog open={false} onClose={() => {}} title="Test">
        body
      </Dialog>,
    );
    expect(container.innerHTML).toBe('');
  });
});

describe('Dropdown', () => {
  it('opens, renders items and closes on outside click', () => {
    const { container } = render(
      <Dropdown label="Menu">
        {(close) => (
          <button type="button" onClick={close}>
            Item 1
          </button>
        )}
      </Dropdown>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Menu' }));
    expect(screen.getByRole('menu')).toBeTruthy();
    fireEvent.click(screen.getByText('Item 1'));
    expect(screen.queryByRole('menu')).toBeNull();
    // reopen and click outside
    fireEvent.click(screen.getByRole('button', { name: 'Menu' }));
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole('menu')).toBeNull();
    expect(container).toBeTruthy();
  });
});
