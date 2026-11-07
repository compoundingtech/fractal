import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'

import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/** Thread identity and lifecycle stay with the caller's authoritative observation. */
export interface ThreadPanelSummary {
  readonly title: string
  readonly title_revision?: string
  readonly state: 'open' | 'closed'
}

export interface ThreadPanelProps {
  readonly thread: ThreadPanelSummary
  /** Supplied only when the read projection knows the full count. */
  readonly messageCount?: number
  readonly unread?: boolean
  readonly embedded?: boolean
  readonly initiallyExpanded?: boolean
  readonly onExpansionChange?: (expanded: boolean) => void
  readonly onHideDraft?: () => void
  readonly onOpenInTab?: () => void
  readonly onRename?: (title: string, titleRevision: string) => Promise<string | undefined>
  readonly onChangeState?: (state: ThreadPanelSummary['state']) => Promise<string | undefined>
  /** Mount the timeline demand and composer only while this presentation is expanded. */
  readonly children: React.ReactNode
}

/** The same conversation surface works inline in any context or as a standalone pane. */
export function ThreadPanel({
  thread, messageCount, unread = false, embedded = true, initiallyExpanded = false, onExpansionChange, onHideDraft, onOpenInTab, onRename, onChangeState, children,
}: ThreadPanelProps): React.ReactNode {
  const [expanded, setExpanded] = React.useState(initiallyExpanded)
  const contentId = React.useId()
  const open = !embedded || expanded
  const [rename, setRename] = React.useState<{ readonly title: string; readonly revision: string }>()
  const [error, setError] = React.useState<string>()
  const [pending, setPending] = React.useState(false)
  const run = async (action: () => Promise<string | undefined>, onSuccess?: () => void) => {
    setPending(true)
    setError(undefined)
    try {
      const failure = await action()
      if (failure === undefined) onSuccess?.()
      else setError(failure)
    } catch {
      setError('The thread change could not be confirmed. Refresh to check its current state.')
    } finally {
      setPending(false)
    }
  }
  const titleRevision = thread.title_revision
  const metadata = <span {...stylex.props(styles.metadata)}>
    {messageCount === undefined ? null : `${messageCount} ${messageCount === 1 ? 'message' : 'messages'}`}
    {messageCount !== undefined && thread.state === 'closed' ? ' · ' : null}
    {thread.state === 'closed' ? 'Closed' : onHideDraft === undefined ? null : 'Draft'}
  </span>
  return <section aria-label={thread.title} {...stylex.props(styles.panel)}>
    {embedded ? <Button aria-expanded={open} aria-controls={open ? contentId : undefined} onPress={() => { setExpanded(!open); onExpansionChange?.(!open) }} {...stylex.props(styles.header)}>
      <span aria-hidden="true">{open ? '▾' : '▸'}</span>
      <span {...stylex.props(styles.title)}>{thread.title}</span>
      {metadata}
      {unread ? <span aria-label="Unread" {...stylex.props(styles.unread)}>●</span> : null}
    </Button> : <h2 {...stylex.props(styles.header)}>{thread.title}{metadata}</h2>}
    {open ? <div id={contentId} {...stylex.props(styles.body)}>
      {children}
      <div {...stylex.props(styles.actions)}>
        {onRename === undefined || titleRevision === undefined ? null : <Button isDisabled={pending} onPress={() => setRename({ title: thread.title, revision: titleRevision })}>Rename</Button>}
        {onChangeState === undefined ? null : <Button isDisabled={pending} onPress={() => {
          void run(() => onChangeState(thread.state === 'open' ? 'closed' : 'open'))
        }}>{thread.state === 'open' ? 'Close thread' : 'Reopen thread'}</Button>}
        {onHideDraft === undefined ? null : <Button onPress={onHideDraft}>Hide draft</Button>}
        {embedded && onOpenInTab !== undefined ? <Button onPress={onOpenInTab}>Open in tab</Button> : null}
      </div>
      {rename === undefined || onRename === undefined ? null : <form onSubmit={event => {
        event.preventDefault()
        const title = rename.title.trim()
        if (title !== '' && title !== thread.title && !pending) void run(() => onRename(title, rename.revision), () => setRename(undefined))
      }} {...stylex.props(styles.actions)}>
        <label>Thread title <input autoFocus value={rename.title} onChange={event => setRename({ ...rename, title: event.target.value })} /></label>
        <Button type="submit" isDisabled={pending || rename.title.trim() === '' || rename.title.trim() === thread.title}>Save</Button>
        <Button isDisabled={pending} onPress={() => setRename(undefined)}>Cancel</Button>
      </form>}
      {error === undefined ? null : <p role="alert">{error}</p>}
    </div> : null}
  </section>
}

const styles = stylex.create({
  panel: { minWidth: 0, borderColor: c.border, borderWidth: 1, borderStyle: 'solid', borderRadius: 6 },
  header: { display: 'flex', alignItems: 'baseline', gap: s.sm, width: '100%', minWidth: 0, textAlign: 'start', borderWidth: 0, backgroundColor: 'transparent', color: c.fg, padding: s.sm, margin: 0, fontSize: t.metaSize },
  title: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  metadata: { flexShrink: 0, color: c.fgMuted },
  unread: { flexShrink: 0, color: c.primary },
  body: { display: 'flex', minWidth: 0, flexDirection: 'column', gap: s.sm, padding: s.sm },
  actions: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.sm },
})
