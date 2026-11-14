import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { Button as ActionButton } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'

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
  readonly pendingChange?: string
  readonly retryPending?: boolean
  readonly onRetryChange?: () => Promise<string | undefined>
  readonly onMarkRead?: () => Promise<string | undefined>
  readonly canMarkRead?: boolean
  /** Mount the timeline demand and composer only while this presentation is expanded. */
  readonly children: React.ReactNode
}

/** The same conversation surface works inline in any context or as a standalone pane. */
export function ThreadPanel({
  thread, messageCount, unread = false, embedded = true, initiallyExpanded = false, onExpansionChange, onHideDraft, onOpenInTab, onRename, onChangeState, pendingChange, retryPending = false, onRetryChange, onMarkRead, canMarkRead = false, children,
}: ThreadPanelProps): React.ReactNode {
  const [expanded, setExpanded] = React.useState(initiallyExpanded)
  const contentId = React.useId()
  const open = !embedded || expanded
  const [rename, setRename] = React.useState<{ readonly title: string; readonly revision: string }>()
  const actions = React.useRef<HTMLDivElement>(null)
  const restoreRename = React.useRef(false)
  // The rename form owns focus while open; closing it returns focus to Rename, not the document body.
  const closeRename = () => { restoreRename.current = true; setRename(undefined) }
  React.useLayoutEffect(() => {
    if (rename !== undefined || !restoreRename.current) return
    restoreRename.current = false
    actions.current?.querySelector<HTMLElement>('[data-thread-rename]')?.focus()
  }, [rename])
  const [error, setError] = React.useState<string>()
  const [pending, setPending] = React.useState(false)
  // Changes act at once; this names the confirmed result, so the reader sees what happened without a dialog.
  const [flash, setFlash] = React.useState<{ readonly text: string; readonly reopen: boolean }>()
  React.useEffect(() => {
    if (flash === undefined) return
    const timer = setTimeout(() => setFlash(undefined), FLASH_MS)
    return () => clearTimeout(timer)
  }, [flash])
  const run = async (action: () => Promise<string | undefined>, onSuccess?: () => void) => {
    setPending(true)
    setError(undefined)
    setFlash(undefined)
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
  const changeState = (onChange: NonNullable<typeof onChangeState>, next: ThreadPanelSummary['state']) => {
    const name = thread.title
    void run(() => onChange(next), () => setFlash(next === 'closed'
      ? { text: `Closed “${name}”.`, reopen: true }
      : { text: `Reopened “${name}”.`, reopen: false }))
  }
  const titleRevision = thread.title_revision
  const metadata = <span {...stylex.props(styles.metadata)}>
    {messageCount === undefined ? null : <>
      <span {...stylex.props(styles.wideLabel)}>{messageCount} {messageCount === 1 ? 'message' : 'messages'}</span>
      <span aria-label={`${messageCount} ${messageCount === 1 ? 'message' : 'messages'}`} {...stylex.props(styles.compactLabel)}>{messageCount}</span>
    </>}
    {messageCount !== undefined && thread.state === 'closed' ? ' · ' : null}
    {thread.state === 'closed' ? 'Closed' : onHideDraft === undefined ? null : 'Draft'}
  </span>
  const feedback = error === undefined ? null : <p role="alert" {...stylex.props(styles.error)}>{error}</p>
  const title = <span {...stylex.props(styles.title)} title={thread.title}>{thread.title}</span>
  // The dot keeps its column when read, so titles stay aligned down a list.
  const unreadDot = <span {...stylex.props(styles.unread)}>{unread ? <span aria-label="Unread">●</span> : null}</span>
  const heading = embedded ? <Button aria-expanded={open} aria-controls={open ? contentId : undefined} onPress={() => { setExpanded(!open); onExpansionChange?.(!open) }} {...stylex.props(styles.header)}>
      <span aria-hidden="true">{open ? '▾' : '▸'}</span>
      {unreadDot}{title}{metadata}
    </Button> : <h2 {...stylex.props(styles.header)}>{unreadDot}{title}{metadata}</h2>
  return <section data-thread-panel="" role="group" aria-label={thread.title} {...stylex.props(styles.panel)}>
    {open && onMarkRead !== undefined ? <div data-thread-read-heading="" {...stylex.props(styles.readHeader)}>
      <div {...stylex.props(styles.readHeadingRow)}>
        {heading}
        <Button aria-label="Mark visible replies read"
          isDisabled={pending || !canMarkRead} onPress={() => { void run(onMarkRead) }} {...stylex.props(styles.readButton)}>
          <span {...stylex.props(styles.wideLabel)}>Mark visible replies read</span>
          <span {...stylex.props(styles.compactLabel)}>Mark read</span>
        </Button>
      </div>
      {feedback}
    </div> : heading}
    {open ? <div id={contentId} {...stylex.props(styles.body)}>
      {children}
      <div ref={actions} {...stylex.props(styles.actions)}>
        {onRename === undefined || titleRevision === undefined ? null : <ActionButton data-thread-rename="" variant="outline" isDisabled={pending || pendingChange !== undefined} onPress={() => setRename({ title: thread.title, revision: titleRevision })}>Rename</ActionButton>}
        {onChangeState === undefined ? null : <ActionButton variant="outline" isDisabled={pending || pendingChange !== undefined}
          onPress={() => changeState(onChangeState, thread.state === 'open' ? 'closed' : 'open')}>{thread.state === 'open' ? 'Close thread' : 'Reopen thread'}</ActionButton>}
        {onHideDraft === undefined ? null : <ActionButton onPress={onHideDraft}>Hide draft</ActionButton>}
        {embedded && onOpenInTab !== undefined ? <ActionButton onPress={onOpenInTab}>Open in tab</ActionButton> : null}
      </div>
      {flash === undefined ? null : <div role="status" data-thread-confirmed="" {...stylex.props(styles.actions, styles.flash)}>
        <span>{flash.text}</span>
        {flash.reopen && onChangeState !== undefined && thread.state === 'closed'
          ? <ActionButton variant="ghost" isDisabled={pending || pendingChange !== undefined} onPress={() => changeState(onChangeState, 'open')}>Reopen</ActionButton>
          : null}
      </div>}
      {pendingChange === undefined || onRetryChange === undefined ? null : <div role="status" {...stylex.props(styles.actions, styles.pending)}>
        {retryPending ? 'Sending…' : `Fractal has not confirmed the previous ${pendingChange}.`}
        <ActionButton variant="outline" isDisabled={pending || retryPending} onPress={() => { void run(onRetryChange, closeRename) }}>Retry {pendingChange}</ActionButton>
      </div>}
      {rename === undefined || onRename === undefined ? null : <form onSubmit={event => {
        event.preventDefault()
        const title = rename.title.trim()
        if (title !== '' && title !== thread.title && !pending && pendingChange === undefined) void run(() => onRename(title, rename.revision), () => { closeRename(); setFlash({ text: `Renamed to “${title}”.`, reopen: false }) })
      }} {...stylex.props(styles.actions)}>
        <label {...stylex.props(styles.field)}>Thread title <input autoFocus value={rename.title} onChange={event => setRename({ ...rename, title: event.target.value })}
          onKeyDown={event => { if (event.key === 'Escape' && !pending) { event.preventDefault(); closeRename() } }} {...stylex.props(styles.input)} /></label>
        <ActionButton variant="outline" type="submit" isDisabled={pending || pendingChange !== undefined || rename.title.trim() === '' || rename.title.trim() === thread.title}>Save</ActionButton>
        <ActionButton isDisabled={pending} onPress={closeRename}>Cancel</ActionButton>
      </form>}
      {onMarkRead === undefined ? feedback : null}
    </div> : null}
  </section>
}

const styles = stylex.create({
  panel: { containerName: 'thread-panel', containerType: 'inline-size', minWidth: 0, borderColor: c.border, borderWidth: 1, borderStyle: 'solid', borderRadius: 6 },
  header: { display: 'flex', alignItems: 'baseline', gap: s.sm, width: '100%', minWidth: 0, textAlign: 'start', borderWidth: 0, backgroundColor: 'transparent', color: c.fg, padding: s.sm, margin: 0, fontSize: t.metaSize },
  readHeader: { position: 'sticky', top: 0, zIndex: 1, backgroundColor: c.canvas },
  readHeadingRow: { display: 'flex', alignItems: 'center', paddingInlineEnd: s.sm },
  readButton: { flexShrink: 0, whiteSpace: 'nowrap', color: c.fgMuted, fontSize: t.metaSize,
    borderRadius: 6, paddingInline: s.sm, paddingBlock: s.xs, cursor: 'pointer',
    ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outline: `2px solid ${c.primary}` },
    ':disabled': { opacity: 0.5, cursor: 'default' } },
  wideLabel: { display: { default: 'inline', '@container thread-panel (max-width: 360px)': 'none' } },
  compactLabel: { display: { default: 'none', '@container thread-panel (max-width: 360px)': 'inline' } },
  title: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  metadata: { flexShrink: 0, marginInlineStart: 'auto', color: c.fgMuted, fontVariantNumeric: 'tabular-nums', textAlign: 'end' },
  unread: { flexShrink: 0, width: '0.75em', color: c.primary },
  body: { display: 'flex', minWidth: 0, flexDirection: 'column', gap: s.sm, padding: s.sm },
  actions: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.sm },
  pending: { color: c.fgMuted, fontSize: t.metaSize },
  flash: { color: c.fgMuted, fontSize: t.metaSize },
  error: { margin: 0, color: c.dangerFg, fontSize: t.metaSize },
  field: { display: 'flex', alignItems: 'center', gap: s.sm, flex: '1 1 16rem', minWidth: 0, color: c.fgMuted, fontSize: t.metaSize },
  input: { flex: '1 1 0', minWidth: 0, height: 24, boxSizing: 'border-box', paddingInline: s.sm, borderWidth: 1, borderStyle: 'solid',
    borderColor: { default: c.borderStrong, ':focus-visible': c.primary }, borderRadius: 6, outline: 'none', backgroundColor: c.controlFill,
    color: c.fg, fontFamily: 'inherit', fontSize: t.metaSize },
})

const FLASH_MS = 8000
