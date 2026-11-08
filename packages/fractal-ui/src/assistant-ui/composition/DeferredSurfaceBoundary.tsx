import * as React from 'react'

/**
 * Share a successful import, but let the next user open recover from a rejected transport. Once the
 * import has landed, an open renders the component itself: no suspense, so no empty first frame.
 * `preload` fetches ahead of the first open, such as on idle.
 */
export function deferredLoader<Props extends object>(load: () => Promise<{ default: React.ComponentType<Props> }>) {
  let pending: Promise<{ default: React.ComponentType<Props> }> | undefined
  let component: React.LazyExoticComponent<React.ComponentType<Props>> | undefined
  let loaded: React.ComponentType<Props> | undefined
  const fetch = () => pending ??= load().then(
    module => { loaded = module.default; return module },
    error => { pending = undefined; component = undefined; throw error },
  )
  const resolve = (): React.ComponentType<Props> => loaded ?? (component ??= React.lazy(fetch))
  return Object.assign(resolve, { preload: (): void => { fetch().catch(() => undefined) } })
}

/** Each open can recover a failed lazy identity without replacing the eager trigger or workspace. */
export class DeferredSurfaceBoundary<Props extends object> extends React.Component<{
  readonly load: () => React.ComponentType<Props>
  readonly surfaceProps: Props
  readonly fallback: React.ReactNode
  readonly pending?: React.ReactNode
}, { readonly failed: boolean }> {
  state = { failed: false }
  private readonly Contents = this.props.load()
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return this.props.fallback
    return <React.Suspense fallback={this.props.pending ?? null}><this.Contents {...this.props.surfaceProps} /></React.Suspense>
  }
}
