import * as React from 'react'

export type CopyAction = () => void | Promise<void>
type CopyState = { readonly tag: 'ready' } | { readonly tag: 'copying' } | { readonly tag: 'copied'; readonly source: string } | { readonly tag: 'failed' }
const feedbackDuration = 1500

/** Both code and answer actions acknowledge only a fulfilled clipboard/host operation. */
export function useCopyFeedback(source: string, action: CopyAction) {
  const [state, setState] = React.useState<CopyState>({ tag: 'ready' })
  const timer = React.useRef<number | undefined>(undefined)
  const mounted = React.useRef(true)
  const pending = React.useRef(false)
  React.useEffect(() => {
    mounted.current = true
    return () => { mounted.current = false; clearTimeout(timer.current) }
  }, [])
  const copy = async () => {
    if (pending.current) return
    pending.current = true
    clearTimeout(timer.current)
    setState({ tag: 'copying' })
    try {
      await action()
      if (!mounted.current) return
      setState({ tag: 'copied', source })
      timer.current = window.setTimeout(() => setState({ tag: 'ready' }), feedbackDuration)
    } catch {
      if (mounted.current) setState({ tag: 'failed' })
    } finally {
      pending.current = false
    }
  }
  const copied = state.tag === 'copied' && state.source === source
  return { state, copied, copy, announcement: copied ? 'Copied' : '' }
}
