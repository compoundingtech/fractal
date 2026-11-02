import * as React from 'react'

/** Contain a failed optional surface without replacing its eager trigger or workspace. */
export class DeferredSurfaceBoundary extends React.Component<{ readonly children: React.ReactNode; readonly fallback: React.ReactNode }, { readonly failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() { return { failed: true } }
  render() { return this.state.failed ? this.props.fallback : this.props.children }
}
