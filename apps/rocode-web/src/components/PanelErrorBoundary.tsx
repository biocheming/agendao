import { Component, type ErrorInfo, type ReactNode } from "react";

interface PanelErrorBoundaryProps {
  children: ReactNode;
  label: string;
}

interface PanelErrorBoundaryState {
  error: Error | null;
}

export class PanelErrorBoundary extends Component<
  PanelErrorBoundaryProps,
  PanelErrorBoundaryState
> {
  state: PanelErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): PanelErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[ROCode Web] ${this.props.label} panel crashed`, error, info);
  }

  componentDidUpdate(previousProps: PanelErrorBoundaryProps) {
    if (previousProps.label !== this.props.label && this.state.error) {
      this.setState({ error: null });
    }
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <div className="roc-state-card m-2" data-tone="danger">
        <p className="text-sm font-semibold text-rose-700 dark:text-rose-300">
          {this.props.label} failed to render.
        </p>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">
          {this.state.error.message || "Unknown render error"}
        </p>
        <button
          className="roc-action roc-action-compact mt-3"
          type="button"
          onClick={() => this.setState({ error: null })}
        >
          Retry
        </button>
      </div>
    );
  }
}
