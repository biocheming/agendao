import { Component, type ErrorInfo, type ReactNode } from "react";

interface PanelErrorBoundaryProps {
  children: ReactNode;
  label: string;
}

interface PanelErrorBoundaryState {
  error: Error | null;
  resetKey: string;
}

export class PanelErrorBoundary extends Component<
  PanelErrorBoundaryProps,
  PanelErrorBoundaryState
> {
  state: PanelErrorBoundaryState = { error: null, resetKey: "" };

  static getDerivedStateFromError(error: Error): PanelErrorBoundaryState {
    return { error, resetKey: "" };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[AgenDao Web] ${this.props.label} panel crashed`, error, info);
  }

  render() {
    if (!this.state.error) {
      return <div key={this.state.resetKey || this.props.label}>{this.props.children}</div>;
    }

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
          onClick={() =>
            this.setState({
              error: null,
              resetKey: `${this.props.label}:${Date.now()}`,
            })}
        >
          Retry
        </button>
      </div>
    );
  }
}
