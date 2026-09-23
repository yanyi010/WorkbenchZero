/**
 * @eigendesk/ui-kit — shared React components for the shell and any plugin
 * that opts into React (spec §77). Visual consistency across surfaces.
 *
 * Every component is keyboard accessible, uses design tokens only, and
 * carries semantic roles/labels (spec §78).
 */
import React, {
  forwardRef,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';

// ---------------------------------------------------------------------------
// Button
// ---------------------------------------------------------------------------

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  small?: boolean;
}

const buttonBase: React.CSSProperties = {
  font: 'inherit',
  fontSize: 'var(--ed-text-md)',
  borderRadius: 'var(--ed-radius-md)',
  border: '1px solid transparent',
  cursor: 'pointer',
  display: 'inline-flex',
  alignItems: 'center',
  gap: 'var(--ed-space-1)',
  padding: '5px 12px',
  transition: 'background var(--ed-fast), border-color var(--ed-fast)',
  whiteSpace: 'nowrap',
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'secondary', small, style, ...rest },
  ref,
) {
  const variants: Record<ButtonVariant, React.CSSProperties> = {
    primary: {
      background: 'var(--ed-accent)',
      color: 'var(--ed-accent-foreground)',
      border: '1px solid var(--ed-accent)',
    },
    secondary: {
      background: 'var(--ed-background-alt)',
      color: 'var(--ed-foreground)',
      borderColor: 'var(--ed-border)',
    },
    ghost: {
      background: 'transparent',
      color: 'var(--ed-foreground)',
    },
    danger: {
      background: 'var(--ed-danger)',
      color: '#fff',
      border: '1px solid var(--ed-danger)',
    },
  };
  return (
    <button
      ref={ref}
      type="button"
      style={{
        ...buttonBase,
        ...variants[variant],
        ...(small ? { padding: '2px 8px', fontSize: 'var(--ed-text-sm)' } : null),
        ...style,
      }}
      {...rest}
    />
  );
});

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { style, ...rest },
  ref,
) {
  return (
    <input
      ref={ref}
      style={{
        font: 'inherit',
        fontSize: 'var(--ed-text-md)',
        color: 'var(--ed-foreground)',
        background: 'var(--ed-background-alt)',
        border: '1px solid var(--ed-border)',
        borderRadius: 'var(--ed-radius-md)',
        padding: '5px 10px',
        width: '100%',
        ...style,
      }}
      {...rest}
    />
  );
});

// ---------------------------------------------------------------------------
// Card / EmptyState / Toolbar / Tabs
// ---------------------------------------------------------------------------

export function Card({
  title,
  actions,
  children,
  style,
}: {
  title?: React.ReactNode;
  actions?: React.ReactNode;
  children: React.ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <section
      style={{
        background: 'var(--ed-background-alt)',
        border: '1px solid var(--ed-border)',
        borderRadius: 'var(--ed-radius-lg)',
        boxShadow: 'var(--ed-shadow-1)',
        padding: 'var(--ed-space-4)',
        ...style,
      }}
    >
      {(title || actions) && (
        <header
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 'var(--ed-space-3)',
          }}
        >
          <h3 style={{ margin: 0, fontSize: 'var(--ed-text-lg)', fontWeight: 600 }}>{title}</h3>
          <div style={{ display: 'flex', gap: 'var(--ed-space-2)' }}>{actions}</div>
        </header>
      )}
      {children}
    </section>
  );
}

export function EmptyState({
  icon = '◇',
  title,
  hint,
  action,
  style,
}: {
  icon?: React.ReactNode;
  title: string;
  hint?: string;
  action?: React.ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <div
      role="status"
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 'var(--ed-space-2)',
        padding: 'var(--ed-space-5)',
        color: 'var(--ed-muted)',
        textAlign: 'center',
        minHeight: 120,
        ...style,
      }}
    >
      <div style={{ fontSize: 28, opacity: 0.7 }} aria-hidden>
        {icon}
      </div>
      <div style={{ color: 'var(--ed-foreground)', fontSize: 'var(--ed-text-lg)' }}>{title}</div>
      {hint && <div style={{ fontSize: 'var(--ed-text-md)' }}>{hint}</div>}
      {action}
    </div>
  );
}

export function Toolbar({ children }: { children: React.ReactNode }) {
  return (
    <div
      role="toolbar"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--ed-space-2)',
        padding: 'var(--ed-space-2) 0',
        borderBottom: '1px solid var(--ed-border)',
        minHeight: 36,
      }}
    >
      {children}
    </div>
  );
}

export function Tabs({
  tabs,
  active,
  onChange,
}: {
  tabs: Array<{ id: string; title: string }>;
  active: string;
  onChange: (id: string) => void;
}) {
  return (
    <div role="tablist" style={{ display: 'flex', gap: 'var(--ed-space-1)', borderBottom: '1px solid var(--ed-border)' }}>
      {tabs.map((t) => {
        const selected = t.id === active;
        return (
          <button
            key={t.id}
            role="tab"
            aria-selected={selected}
            type="button"
            onClick={() => onChange(t.id)}
            style={{
              font: 'inherit',
              fontSize: 'var(--ed-text-md)',
              cursor: 'pointer',
              padding: '6px 12px',
              background: 'transparent',
              color: selected ? 'var(--ed-foreground)' : 'var(--ed-muted)',
              border: 'none',
              borderBottom: `2px solid ${selected ? 'var(--ed-accent)' : 'transparent'}`,
              marginBottom: -1,
            }}
          >
            {t.title}
          </button>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// CommandList — keyboard-driven list used by palette, search, quick capture
// ---------------------------------------------------------------------------

export interface CommandListItem {
  id: string;
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  detail?: React.ReactNode;
  icon?: React.ReactNode;
  keyhint?: string;
  group?: string;
}

export function CommandList({
  items,
  onPick,
  selected,
  onSelectedChange,
  empty,
  maxHeight = 420,
}: {
  items: CommandListItem[];
  onPick: (item: CommandListItem) => void;
  selected: number;
  onSelectedChange: (index: number) => void;
  empty?: React.ReactNode;
  maxHeight?: number;
}) {
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>('[data-selected="true"]');
    // jsdom has no scrollIntoView; guard for test environments.
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [selected]);

  if (items.length === 0) {
    return (
      <div style={{ padding: 'var(--ed-space-4)', color: 'var(--ed-muted)', textAlign: 'center' }}>
        {empty ?? 'No matches'}
      </div>
    );
  }

  let lastGroup: string | undefined;
  return (
    <div
      ref={listRef}
      role="listbox"
      aria-label="Results"
      style={{ overflowY: 'auto', maxHeight }}
    >
      {items.map((item, i) => {
        const groupHeader =
          item.group && item.group !== lastGroup ? (
            <div
              key={`g-${item.group}`}
              style={{
                padding: '6px 12px 2px',
                fontSize: 'var(--ed-text-xs)',
                color: 'var(--ed-muted)',
                textTransform: 'uppercase',
                letterSpacing: 0.4,
              }}
            >
              {item.group}
            </div>
          ) : null;
        lastGroup = item.group ?? lastGroup;
        const isSelected = i === selected;
        return (
          <React.Fragment key={item.id}>
            {groupHeader}
            <div
              role="option"
              aria-selected={isSelected}
              data-selected={isSelected}
              tabIndex={-1}
              onClick={() => onPick(item)}
              onMouseEnter={() => onSelectedChange(i)}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 'var(--ed-space-2)',
                padding: '6px 12px',
                cursor: 'pointer',
                background: isSelected ? 'var(--ed-accent-soft)' : 'transparent',
                color: 'var(--ed-foreground)',
              }}
            >
              {item.icon !== undefined && (
                <span aria-hidden style={{ width: 18, textAlign: 'center', opacity: 0.8 }}>
                  {item.icon}
                </span>
              )}
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {item.title}
                </div>
                {item.subtitle && (
                  <div
                    style={{
                      fontSize: 'var(--ed-text-sm)',
                      color: 'var(--ed-muted)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {item.subtitle}
                  </div>
                )}
              </div>
              {item.detail && (
                <div style={{ color: 'var(--ed-muted)', fontSize: 'var(--ed-text-sm)' }}>
                  {item.detail}
                </div>
              )}
              {item.keyhint && (
                <kbd
                  style={{
                    fontFamily: 'var(--ed-font-mono)',
                    fontSize: 'var(--ed-text-xs)',
                    color: 'var(--ed-muted)',
                    border: '1px solid var(--ed-border)',
                    borderRadius: 'var(--ed-radius-sm)',
                    padding: '1px 5px',
                  }}
                >
                  {item.keyhint}
                </kbd>
              )}
            </div>
          </React.Fragment>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------

export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  width = 520,
}: {
  open: boolean;
  onClose: () => void;
  title: React.ReactNode;
  children: React.ReactNode;
  footer?: React.ReactNode;
  width?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    const prev = ref.current?.querySelector<HTMLElement>(
      'button, input, select, textarea, [tabindex]:not([tabindex="-1"])',
    );
    prev?.focus();
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(15, 16, 20, 0.45)',
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: '12vh',
        zIndex: 100,
      }}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        style={{
          width: 'min(92vw, ' + width + 'px)',
          maxHeight: '76vh',
          overflow: 'auto',
          background: 'var(--ed-background-alt)',
          color: 'var(--ed-foreground)',
          border: '1px solid var(--ed-border)',
          borderRadius: 'var(--ed-radius-lg)',
          boxShadow: 'var(--ed-shadow-overlay)',
        }}
      >
        <header
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            padding: 'var(--ed-space-3) var(--ed-space-4)',
            borderBottom: '1px solid var(--ed-border)',
          }}
        >
          <h2 id={titleId} style={{ margin: 0, fontSize: 'var(--ed-text-lg)' }}>
            {title}
          </h2>
          <Button variant="ghost" small aria-label="Close dialog" onClick={onClose}>
            ✕
          </Button>
        </header>
        <div style={{ padding: 'var(--ed-space-4)' }}>{children}</div>
        {footer && (
          <footer
            style={{
              display: 'flex',
              justifyContent: 'flex-end',
              gap: 'var(--ed-space-2)',
              padding: 'var(--ed-space-3) var(--ed-space-4)',
              borderTop: '1px solid var(--ed-border)',
            }}
          >
            {footer}
          </footer>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Dropdown
// ---------------------------------------------------------------------------

export function Dropdown({
  label,
  children,
  align = 'left',
}: {
  label: React.ReactNode;
  children: (close: () => void) => React.ReactNode;
  align?: 'left' | 'right';
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setOpen(false), []);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDoc);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    <div ref={rootRef} style={{ position: 'relative', display: 'inline-block' }}>
      <Button variant="ghost" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        {label}
      </Button>
      {open && (
        <div
          role="menu"
          style={{
            position: 'absolute',
            top: '100%',
            [align]: 0,
            marginTop: 4,
            minWidth: 200,
            background: 'var(--ed-background-alt)',
            border: '1px solid var(--ed-border)',
            borderRadius: 'var(--ed-radius-md)',
            boxShadow: 'var(--ed-shadow-2)',
            padding: 'var(--ed-space-1)',
            zIndex: 60,
          }}
        >
          {children(close)}
        </div>
      )}
    </div>
  );
}

export function DropdownItem({
  onPick,
  children,
  danger,
}: {
  onPick: () => void;
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onPick}
      style={{
        display: 'block',
        width: '100%',
        textAlign: 'left',
        font: 'inherit',
        fontSize: 'var(--ed-text-md)',
        padding: '6px 10px',
        background: 'transparent',
        color: danger ? 'var(--ed-danger)' : 'var(--ed-foreground)',
        border: 'none',
        borderRadius: 'var(--ed-radius-sm)',
        cursor: 'pointer',
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--ed-accent-soft)')}
      onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
    >
      {children}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Tree
// ---------------------------------------------------------------------------

export interface TreeNode {
  id: string;
  label: string;
  icon?: React.ReactNode;
  children?: TreeNode[];
}

export function Tree({
  nodes,
  onSelect,
  selectedId,
  depth = 0,
}: {
  nodes: TreeNode[];
  onSelect?: (node: TreeNode) => void;
  selectedId?: string;
  depth?: number;
}) {
  return (
    <ul role="tree" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
      {nodes.map((n) => (
        <TreeItem key={n.id} node={n} onSelect={onSelect} selectedId={selectedId} depth={depth} />
      ))}
    </ul>
  );
}

function TreeItem({
  node,
  onSelect,
  selectedId,
  depth,
}: {
  node: TreeNode;
  onSelect?: (node: TreeNode) => void;
  selectedId?: string;
  depth: number;
}) {
  const [open, setOpen] = useState(depth < 1);
  const hasChildren = !!node.children?.length;
  const selected = node.id === selectedId;
  return (
    <li role="treeitem" aria-expanded={hasChildren ? open : undefined}>
      <div
        role="button"
        tabIndex={0}
        onClick={() => {
          if (hasChildren) setOpen((v) => !v);
          onSelect?.(node);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            if (hasChildren) setOpen((v) => !v);
            onSelect?.(node);
          }
        }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '3px 8px',
          paddingLeft: 8 + depth * 14,
          cursor: 'pointer',
          borderRadius: 'var(--ed-radius-sm)',
          background: selected ? 'var(--ed-accent-soft)' : 'transparent',
          color: selected ? 'var(--ed-foreground)' : 'var(--ed-foreground)',
        }}
      >
        <span aria-hidden style={{ width: 12, fontSize: 10, color: 'var(--ed-muted)' }}>
          {hasChildren ? (open ? '▾' : '▸') : ''}
        </span>
        {node.icon && (
          <span aria-hidden style={{ opacity: 0.8 }}>
            {node.icon}
          </span>
        )}
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {node.label}
        </span>
      </div>
      {hasChildren && open && (
        <Tree nodes={node.children!} onSelect={onSelect} selectedId={selectedId} depth={depth + 1} />
      )}
    </li>
  );
}

// ---------------------------------------------------------------------------
// Table
// ---------------------------------------------------------------------------

export function Table({
  columns,
  rows,
  empty,
}: {
  columns: Array<{ key: string; title: React.ReactNode; width?: number | string }>;
  rows: Array<Record<string, React.ReactNode>>;
  empty?: React.ReactNode;
}) {
  if (rows.length === 0) {
    return <>{empty ?? <EmptyState title="Nothing here yet" />}</>;
  }
  return (
    <table
      style={{
        width: '100%',
        borderCollapse: 'collapse',
        fontSize: 'var(--ed-text-md)',
      }}
    >
      <thead>
        <tr>
          {columns.map((c) => (
            <th
              key={c.key}
              scope="col"
              style={{
                textAlign: 'left',
                fontWeight: 600,
                fontSize: 'var(--ed-text-sm)',
                color: 'var(--ed-muted)',
                borderBottom: '1px solid var(--ed-border)',
                padding: '6px 10px',
                width: c.width,
              }}
            >
              {c.title}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, i) => (
          <tr key={i}>
            {columns.map((c) => (
              <td
                key={c.key}
                style={{
                  borderBottom: '1px solid var(--ed-border)',
                  padding: '6px 10px',
                  verticalAlign: 'top',
                }}
              >
                {row[c.key]}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ---------------------------------------------------------------------------
// CodeBlock
// ---------------------------------------------------------------------------

export function CodeBlock({ code, language }: { code: string; language?: string }) {
  return (
    <pre
      style={{
        margin: 0,
        padding: 'var(--ed-space-3)',
        background: 'var(--ed-background-sink)',
        border: '1px solid var(--ed-border)',
        borderRadius: 'var(--ed-radius-md)',
        overflow: 'auto',
        fontFamily: 'var(--ed-font-mono)',
        fontSize: 'var(--ed-text-sm)',
        lineHeight: 1.5,
      }}
      {...(language ? { 'data-lang': language } : {})}
    >
      <code>{code}</code>
    </pre>
  );
}

// ---------------------------------------------------------------------------
// Markdown — marked + DOMPurify, never dangerouslySetInnerHTML of raw input
// ---------------------------------------------------------------------------

const mdCache = new Map<string, string>();

export function Markdown({ source }: { source: string }) {
  let html = mdCache.get(source);
  if (html === undefined) {
    const raw = marked.parse(source, { async: false, gfm: true, breaks: true }) as string;
    html = DOMPurify.sanitize(raw, {
      USE_PROFILES: { html: true },
      FORBID_TAGS: ['style', 'form', 'input', 'iframe', 'script'],
      FORBID_ATTR: ['onerror', 'onclick', 'onload'],
    });
    if (mdCache.size > 200) mdCache.clear();
    mdCache.set(source, html);
  }
  return (
    <div
      className="ed-markdown"
      // Sanitized above; the pipeline is marked → DOMPurify (no raw HTML).
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}

// ---------------------------------------------------------------------------
// Badge / Spinner — small shared atoms
// ---------------------------------------------------------------------------

export function Badge({
  children,
  tone = 'muted',
}: {
  children: React.ReactNode;
  tone?: 'muted' | 'accent' | 'success' | 'warning' | 'danger';
}) {
  const tones: Record<string, React.CSSProperties> = {
    muted: { color: 'var(--ed-muted)', background: 'var(--ed-background-sink)' },
    accent: { color: 'var(--ed-accent)', background: 'var(--ed-accent-soft)' },
    success: { color: 'var(--ed-success)', background: 'var(--ed-success-soft)' },
    warning: { color: 'var(--ed-warning)', background: 'var(--ed-warning-soft)' },
    danger: { color: 'var(--ed-danger)', background: 'var(--ed-danger-soft)' },
  };
  return (
    <span
      style={{
        display: 'inline-block',
        fontSize: 'var(--ed-text-xs)',
        fontWeight: 600,
        borderRadius: 999,
        padding: '1px 8px',
        ...tones[tone],
      }}
    >
      {children}
    </span>
  );
}

export function Spinner({ label }: { label?: string }) {
  return (
    <span role="status" aria-label={label ?? 'Loading'} style={{ color: 'var(--ed-muted)' }}>
      ◌
    </span>
  );
}
