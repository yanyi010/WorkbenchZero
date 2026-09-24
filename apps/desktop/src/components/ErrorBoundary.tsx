import { Component, type ReactNode } from 'react';

interface Props {
  children: ReactNode;
  /** Rendered when the subtree throws; identifies the surface. */
  label: string;
}

interface State {
  error: string | null;
}

/**
 * Fault containment for shell surfaces (spec §119: a plugin view crash must
 * not take the shell down). Plugin code itself runs cross-origin in iframes,
 * but the host-side React around a surface can still throw — this boundary
 * keeps the rest of the shell interactive.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: unknown): State {
    return { error: error instanceof Error ? error.message : String(error) };
  }

  componentDidCatch(error: unknown, info: { componentStack?: string }) {
    // Best-effort diagnostics; never throw from the boundary itself.
    console.error(`[${this.props.label}]`, error, info.componentStack ?? '');
  }

  render() {
    if (this.state.error) {
      return (
        <div role="alert" className="ed-view-host-error" style={{ padding: 16 }}>
          <strong>{this.props.label} crashed.</strong>
          <p style={{ opacity: 0.8 }}>{this.state.error}</p>
          <button onClick={() => this.setState({ error: null })}>Retry</button>
        </div>
      );
    }
    return this.props.children;
  }
}
