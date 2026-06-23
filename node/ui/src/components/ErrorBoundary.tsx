// A render-time error boundary. React has no hook equivalent for
// `getDerivedStateFromError`/`componentDidCatch`, so this is a (small) class
// component. We deliberately avoid pulling in `react-error-boundary` for ~40
// lines — the surface here is exactly what we need and nothing more.
//
// This catches errors thrown during *render* (including the app's intentional
// "useXyz must be used within a Provider" context guards). It does NOT catch
// async rejections (event handlers, query promises) — those are surfaced at the
// call site (toasts / `isError` branches), see EnvironmentGate.
//
// The routed area uses TanStack Router's built-in `defaultErrorComponent`
// instead (see routeTree.tsx), which keeps the shell mounted and gives a
// router-aware reset. This component guards the non-routed regions (the root
// safety net and the environment manager).

import { Component, type ErrorInfo, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { parseUcError } from "@/lib/uc/errors";

interface Props {
  children: ReactNode;
  /** Custom fallback. Defaults to a centered "Something went wrong" card. */
  fallback?: (error: unknown, reset: () => void) => ReactNode;
  /** Called when the boundary resets (e.g. to clear related state). */
  onReset?: () => void;
  /**
   * When any value here changes, the boundary clears its error and re-renders
   * its children. Use to auto-recover on navigation / environment switches.
   */
  resetKeys?: readonly unknown[];
}

interface State {
  error: unknown;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: unknown): State {
    return { error };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    // No telemetry sink exists today; log so it's visible in the console.
    console.error("Uncaught render error:", error, info.componentStack);
  }

  componentDidUpdate(prev: Props) {
    if (this.state.error === null) return;
    if (!keysChanged(prev.resetKeys, this.props.resetKeys)) return;
    this.reset();
  }

  reset = () => {
    this.props.onReset?.();
    this.setState({ error: null });
  };

  render() {
    const { error } = this.state;
    if (error === null) return this.props.children;
    if (this.props.fallback) return this.props.fallback(error, this.reset);
    return <ErrorFallback error={error} onReset={this.reset} />;
  }
}

function keysChanged(a: readonly unknown[] = [], b: readonly unknown[] = []) {
  return a.length !== b.length || a.some((v, i) => !Object.is(v, b[i]));
}

// The default fallback — a centered card matching the app's destructive tokens.
// Reused by the router error component too, so the two surfaces look identical.
export function ErrorFallback({
  error,
  onReset,
  resetLabel = "Try again",
}: {
  error: unknown;
  onReset: () => void;
  resetLabel?: string;
}) {
  return (
    <div className="flex flex-1 items-center justify-center p-8">
      <div className="max-w-md rounded-lg border bg-destructive/5 p-6 text-center">
        <h2 className="text-sm font-semibold text-destructive">
          Something went wrong
        </h2>
        <p className="mt-2 break-words text-sm text-muted-foreground">
          {parseUcError(error, "An unexpected error occurred.")}
        </p>
        <Button variant="outline" size="sm" className="mt-4" onClick={onReset}>
          {resetLabel}
        </Button>
      </div>
    </div>
  );
}
