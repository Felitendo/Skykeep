import { Component, type ReactNode } from "react";

/**
 * Keeps a failure in one screen inside that screen.
 *
 * Without a boundary React unmounts everything on a render error, so one bad row anywhere
 * took the sidebar, every screen and the way back with it: the window went blank and only a
 * restart helped. Here the screen says what happened and can be opened again; the rest of
 * the app, and everything stored, carry on.
 */
export class ScreenBoundary extends Component<{ children: ReactNode; name: string }, { failed: Error | null }> {
  state = { failed: null as Error | null };

  static getDerivedStateFromError(failed: Error) {
    return { failed };
  }

  componentDidCatch(error: Error) {
    console.error(`${this.props.name} failed to draw:`, error);
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="screen-failed" role="alert">
        <p>{this.props.name} couldn’t be shown. Nothing stored was affected.</p>
        <button className="quiet" onClick={() => this.setState({ failed: null })}>
          Open {this.props.name} Again
        </button>
      </div>
    );
  }
}
