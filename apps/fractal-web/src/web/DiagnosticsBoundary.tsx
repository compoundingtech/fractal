import * as React from 'react'
import * as stylex from '@stylexjs/stylex'

/** Optional diagnostics must never take down the workspace's independent React subtree. */
export class DiagnosticsBoundary extends React.Component<{
  readonly children: React.ReactNode
  readonly onReload?: () => void
}> {
  state: { readonly _tag: 'Ready' | 'Unavailable' } = { _tag: 'Ready' }

  static getDerivedStateFromError() {
    return { _tag: 'Unavailable' as const }
  }

  render() {
    if (this.state._tag === 'Ready') return this.props.children
    return <aside data-testid="wf-devbar" aria-label="Diagnostics unavailable" {...stylex.props(styles.bar)}>
      <span role="status">Diagnostics could not load. Your workspace is still available.</span>
      <button type="button" onClick={this.props.onReload ?? (() => window.location.reload())} {...stylex.props(styles.retry)}>Reload diagnostics</button>
    </aside>
  }
}

const styles = stylex.create({
  bar: { position: 'fixed', insetInline: 0, bottom: 0, minHeight: 32, boxSizing: 'border-box', paddingInline: 12, display: 'flex', alignItems: 'center', gap: 12, backgroundColor: 'Canvas', color: 'CanvasText', fontSize: 12, zIndex: 1000 },
  retry: { backgroundColor: 'transparent', color: 'inherit', borderWidth: 1, borderStyle: 'solid', borderColor: 'currentColor', borderRadius: 4, cursor: 'pointer', fontSize: 12, paddingBlock: 2, paddingInline: 6 },
})
