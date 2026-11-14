import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, VisuallyHidden } from 'react-aria-components'
import { Button as ActionButton } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'

import { colorVars as c, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

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
  /** An embedded row that opens elsewhere (a dedicated tab): pressing the header calls this instead of expanding in place. */
  readonly onActivate?: () => void
  /** The row sits in a shared bordered list: no frame of its own, a hairline between rows. */
  readonly inList?: boolean
  /** Last-activity age, the header's final column. */
  readonly age?: React.ReactNode
  /** Where the thread came from; shown under the heading of the dedicated view only. */
  readonly origin?: React.ReactNode
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
  thread, messageCount, unread = false, embedded = true, initiallyExpanded = false, onExpansionChange, onActivate, inList = false, age, origin, onHideDraft, onOpenInTab, onRename, onChangeState, pendingChange, retryPending = false, onRetryChange, onMarkRead, canMarkRead = false, children,
}: ThreadPanelProps): React.ReactNode {
  const narrow = React.useContext(NarrowLayoutContext)
  const [expanded, setExpanded] = React.useState(initiallyExpanded)
  const contentId = React.useId()
  const opensElsewhere = embedded && onActivate !== undefined
  const open = !embedded || (expanded && !opensElsewhere)
  const [rename, setRename] = React.useState<{ readonly title: string; readonly revision: string }>()
  const actions = React.useRef<HTMLDivElement>(null)
  const restoreRename = React.useRef(false)
  // The title field owns focus while open; closing it returns focus to Rename, not the document body.
  const closeRename = () => { restoreRename.current = true; setRename(undefined) }
  React.useLayoutEffect(() => {
    if (rename !== undefined || !restoreRename.current) return
    restoreRename.current = false
    actions.current?.querySelector<HTMLElement>('[data-thread-rename]')?.focus()
  }, [rename])
  const [error, setError] = React.useState<string>()
  const [pending, setPending] = React.useState(false)
  // Changes act at once; this names the confirmed result, so the reader sees what happened without a dialog.
  const [flash, setFlash] = React.useState<string>()
  // A keyboard user's focus has just moved to the button that names the next step, and a pointer may rest on
  // the panel: the confirmation stays while either is on the panel and runs its full time once both leave.
  const [hovered, setHovered] = React.useState(false)
  const [focused, setFocused] = React.useState(false)
  const held = hovered || focused
  React.useEffect(() => {
    if (flash === undefined || held) return
    const timer = setTimeout(() => setFlash(undefined), FLASH_MS)
    return () => clearTimeout(timer)
  }, [flash, held])
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
  // A pressed Close or Reopen is disabled while the change is pending, which drops focus to the
  // document; once the change settles, focus returns to the lifecycle button that now names the next step.
  const restoreState = React.useRef(false)
  React.useLayoutEffect(() => {
    if (!restoreState.current || pending || pendingChange !== undefined) return
    restoreState.current = false
    if (document.activeElement === null || document.activeElement === document.body)
      actions.current?.querySelector<HTMLElement>('[data-thread-state]')?.focus()
  }, [pending, pendingChange, thread.state])
  const changeState = (onChange: NonNullable<typeof onChangeState>, next: ThreadPanelSummary['state']) => {
    const name = thread.title
    restoreState.current = true
    void run(() => onChange(next), () => setFlash(next === 'closed' ? `Closed “${name}”.` : `Reopened “${name}”.`))
  }
  const titleRevision = thread.title_revision
  const messages = messageCount === 1 ? 'message' : 'messages'
  // Narrow rows never show a bare number: the right-aligned number is reserved for an unread count.
  const count = messageCount === undefined ? null : <>
    <span {...stylex.props(styles.wideLabel)}>{messageCount} {messages}</span>
    <span {...stylex.props(styles.compactLabel)}>
      <span aria-hidden="true">{messageCount} {messageCount === 1 ? 'msg' : 'msgs'}</span>
      <VisuallyHidden elementType="span">{messageCount} {messages}</VisuallyHidden>
    </span>
  </>
  const status = thread.state === 'closed' ? 'Closed' : onHideDraft === undefined ? null : 'Draft'
  const metadata = count === null && status === null ? null
    : <span {...stylex.props(styles.metadata)}>{count}{count !== null && status !== null ? ' · ' : null}{status}</span>
  const ageCell = age === undefined ? null : <span data-thread-age="" {...stylex.props(styles.age)}>{age}</span>
  const feedback = error === undefined ? null : <p role="alert" {...stylex.props(styles.error)}>{error}</p>
  const title = <span {...stylex.props(styles.title, !embedded && styles.paneTitle)} title={thread.title}>{thread.title}</span>
  // Rows keep the dot's column when read, so titles stay aligned down a list; a dedicated view has no rows to align.
  const unreadDot = embedded || unread ? <span {...stylex.props(styles.dot)}>{unread ? <>
    <span aria-hidden="true">●</span><VisuallyHidden elementType="span">Unread, </VisuallyHidden>
  </> : null}</span> : null
  const chevron = embedded ? <span aria-hidden="true" {...stylex.props(styles.chevron)}><Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} /></span> : null
  const columns = embedded ? styles.rowColumns : unread ? styles.dotColumns : styles.titleColumns
  // Rename edits the title where it stands; the field and its Save/Cancel pair take the title's place in the header.
  const heading = rename !== undefined && onRename !== undefined ? <form data-thread-rename-form="" onSubmit={event => {
      event.preventDefault()
      const title = rename.title.trim()
      if (title !== '' && title !== thread.title && !pending && pendingChange === undefined) void run(() => onRename(title, rename.revision), () => { closeRename(); setFlash(`Renamed to “${title}”.`) })
    }} {...stylex.props(styles.header, columns, narrow && styles.headerTouch)}>
      {chevron}{unreadDot}
      <input aria-label="Thread title" autoFocus value={rename.title} onChange={event => setRename({ ...rename, title: event.target.value })}
        onKeyDown={event => { if (event.key === 'Escape' && !pending) { event.preventDefault(); closeRename() } }}
        {...stylex.props(styles.input, !embedded && styles.paneInput, narrow && styles.inputTouch)} />
      <span {...stylex.props(styles.renameActions)}>
        <ActionButton variant="outline" type="submit" isDisabled={pending || pendingChange !== undefined || rename.title.trim() === '' || rename.title.trim() === thread.title}>Save</ActionButton>
        <ActionButton variant="outline" isDisabled={pending} onPress={closeRename}>Cancel</ActionButton>
      </span>
    </form>
    : opensElsewhere ? <Button onPress={onActivate} {...stylex.props(styles.header, columns, narrow && styles.headerTouch, styles.toggle)}>{chevron}{unreadDot}{title}{metadata}{ageCell}</Button>
    : embedded ? <Button aria-expanded={open} aria-controls={open ? contentId : undefined} onPress={() => { setExpanded(!open); onExpansionChange?.(!open) }} {...stylex.props(styles.header, columns, narrow && styles.headerTouch, styles.toggle)}>
      {chevron}{unreadDot}{title}{metadata}{ageCell}
    </Button>
    : <h2 {...stylex.props(styles.header, columns)}>{unreadDot}{title}{metadata}{ageCell}</h2>
  const markRead = open && onMarkRead !== undefined ? <Button aria-label="Mark visible replies read"
    isDisabled={pending || !canMarkRead} onPress={() => { void run(onMarkRead) }} {...stylex.props(styles.readButton, narrow && styles.readTouch)}>
    <span {...stylex.props(styles.wideLabel)}>Mark visible replies read</span>
    <span {...stylex.props(styles.compactLabel)}>Mark read</span>
  </Button> : null
  const subline = embedded || origin === undefined ? null : <p data-thread-origin="" {...stylex.props(styles.origin)}>{origin}</p>
  // A thread in its own tab keeps the Inbox detail's reading measure instead of spanning the pane.
  return <section data-thread-panel="" role="group" aria-label={thread.title}
    onMouseEnter={() => setHovered(true)} onMouseLeave={() => setHovered(false)} onFocus={() => setFocused(true)}
    onBlur={event => { if (!(event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget))) setFocused(false) }}
    {...stylex.props(styles.panel, inList && styles.inList, !embedded && styles.dedicated)}>
    {markRead === null ? <>{heading}{subline}</> : <div data-thread-read-heading="" {...stylex.props(styles.readHeader)}>
      <div {...stylex.props(styles.readHeadingRow)}>{heading}{markRead}</div>
      {subline}
      {feedback}
    </div>}
    {open ? <div id={contentId} {...stylex.props(styles.body)}>
      {children}
      <div ref={actions} {...stylex.props(styles.actions)}>
        {onRename === undefined || titleRevision === undefined ? null : <ActionButton data-thread-rename="" variant="outline" isDisabled={pending || pendingChange !== undefined || rename !== undefined} onPress={() => { setFlash(undefined); setRename({ title: thread.title, revision: titleRevision }) }}>Rename</ActionButton>}
        {onChangeState === undefined ? null : <ActionButton data-thread-state="" variant="outline" isDisabled={pending || pendingChange !== undefined}
          onPress={() => changeState(onChangeState, thread.state === 'open' ? 'closed' : 'open')}>{thread.state === 'open' ? 'Close thread' : 'Reopen thread'}</ActionButton>}
        {onHideDraft === undefined ? null : <ActionButton variant="outline" onPress={onHideDraft}>Hide draft</ActionButton>}
        {embedded && onOpenInTab !== undefined ? <ActionButton variant="outline" onPress={onOpenInTab}>Open in tab</ActionButton> : null}
      </div>
      {flash === undefined ? null : <div role="status" data-thread-confirmed="" {...stylex.props(styles.flash)}><Icon name="check" size={12} />{flash}</div>}
      {pendingChange === undefined || onRetryChange === undefined ? null : <div role="status" {...stylex.props(styles.actions, styles.pending)}>
        {retryPending ? 'Sending…' : `Fractal has not confirmed the previous ${pendingChange}.`}
        <ActionButton variant="outline" isDisabled={pending || retryPending} onPress={() => { void run(onRetryChange, closeRename) }}>Retry {pendingChange}</ActionButton>
      </div>}
      {markRead === null ? feedback : null}
    </div> : null}
  </section>
}

const flashIn = stylex.keyframes({ from: { opacity: 0 }, to: { opacity: 1 } })

const styles = stylex.create({
  panel: { containerName: 'thread-panel', containerType: 'inline-size', minWidth: 0, borderColor: c.border, borderWidth: g.hairline, borderStyle: 'solid', borderRadius: r.sm },
  // The list draws the frame; each row after the first draws the hairline above it.
  inList: { borderWidth: 0, borderRadius: 0, borderBlockStartWidth: { default: g.hairline, ':first-child': 0 }, borderBlockStartStyle: 'solid', borderBlockStartColor: c.border },
  dedicated: { boxSizing: 'border-box', width: '100%', maxInlineSize: g.lane, marginInline: 'auto' },
  // Disclosure | unread dot | title | metadata | age. Fixed leading columns align rows down a list; metadata and age stay at the end.
  header: { display: 'grid', gridAutoFlow: 'column', gridAutoColumns: 'auto', alignItems: 'baseline', columnGap: s.sm, boxSizing: 'border-box', width: '100%', minWidth: 0, textAlign: 'start', borderWidth: 0, backgroundColor: 'transparent', color: c.fg, padding: s.sm, margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightRegular },
  rowColumns: { gridTemplateColumns: `${g.statusSm} ${g.statusSm} minmax(0, 1fr)` },
  dotColumns: { gridTemplateColumns: `${g.statusSm} minmax(0, 1fr)` },
  titleColumns: { gridTemplateColumns: 'minmax(0, 1fr)' },
  headerTouch: { minHeight: g.touchTarget, alignContent: 'center' },
  toggle: { borderRadius: r.sm, cursor: 'pointer', outline: { default: 'none', ':focus-visible': `${g.focusRing} solid ${c.primary}` }, outlineOffset: `calc(-1 * ${g.focusRing})` },
  chevron: { display: 'inline-flex', alignSelf: 'center', justifyContent: 'center', color: c.fgMuted },
  dot: { color: c.primary },
  title: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontWeight: t.weightMedium },
  paneTitle: { fontSize: t.headingSize, lineHeight: t.headingLeading },
  metadata: { color: c.fgMuted, fontVariantNumeric: 'tabular-nums', textAlign: 'end', whiteSpace: 'nowrap' },
  age: { minWidth: g.sidebarMetric, color: c.fgMuted, fontVariantNumeric: 'tabular-nums', textAlign: 'end', whiteSpace: 'nowrap' },
  origin: { margin: 0, paddingInline: s.sm, paddingBlockEnd: s.sm, color: c.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  readHeader: { position: 'sticky', top: 0, zIndex: 1, backgroundColor: c.canvas },
  readHeadingRow: { display: 'flex', alignItems: 'center', paddingInlineEnd: s.sm },
  readButton: { flexShrink: 0, whiteSpace: 'nowrap', color: c.fgMuted, fontSize: t.metaSize,
    borderRadius: r.sm, paddingInline: s.sm, paddingBlock: s.xs, cursor: 'pointer',
    ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outline: `${g.focusRing} solid ${c.primary}` },
    ':disabled': { opacity: 0.5, cursor: 'default' } },
  readTouch: { minHeight: g.touchTarget },
  wideLabel: { display: { default: 'inline', '@container thread-panel (max-width: 360px)': 'none' } },
  compactLabel: { display: { default: 'none', '@container thread-panel (max-width: 360px)': 'inline' } },
  body: { display: 'flex', minWidth: 0, flexDirection: 'column', gap: s.sm, padding: s.sm },
  actions: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.sm },
  pending: { color: c.fgMuted, fontSize: t.metaSize },
  flash: { display: 'flex', alignItems: 'center', gap: s.xs, color: c.fgSoft, fontSize: t.metaSize,
    animationName: { default: flashIn, '@media (prefers-reduced-motion: reduce)': 'none' }, animationDuration: m.fast, animationTimingFunction: 'ease-out' },
  error: { margin: 0, color: c.dangerFg, fontSize: t.metaSize },
  input: { minWidth: 0, height: g.controlSm, boxSizing: 'border-box', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid',
    borderColor: { default: c.borderStrong, ':focus-visible': c.primary }, borderRadius: r.sm, outline: 'none', backgroundColor: c.controlFill,
    color: c.fg, fontFamily: 'inherit', fontSize: t.metaSize, fontWeight: t.weightMedium },
  paneInput: { height: g.controlMd, fontSize: t.headingSize },
  inputTouch: { height: g.touchTarget, fontSize: t.touchInputSize },
  renameActions: { display: 'flex', flexWrap: 'nowrap', alignSelf: 'center', gap: s.sm },
})

const FLASH_MS = 8000
