import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { Button, VisuallyHidden } from 'react-aria-components'
import { Tooltip } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { detectPlatform } from '../workbench-kit/keys.tsx'
import { ThreadRefusalAlert } from './ThreadRefusalAlert.tsx'
import type { ThreadRefusal } from './threadSource.ts'
import type { ThreadUnreadReminder, UnreadActionPort, UnreadActionResult } from './unread-action-port.ts'
import { threadAnchorAttribute, threadAnchorSelector } from './threadAnchor.ts'

export interface UnreadReminderControlsProps {
  readonly threadId: string
  readonly port: UnreadActionPort
  /** Reply IDs visible at press time, in timeline order; the first anchors Mark unread. */
  readonly visibleReplies: () => readonly string[]
  /**
   * The timeline holding message anchor rows. When supplied, Option/Alt-click on a message marks unread from
   * that reply, and Jump scrolls the reminded reply into view; without it neither is offered.
   */
  readonly timeline?: HTMLElement | null
  /** Runs just before a deliberate Mark unread is sent, so the caller can hold the visit and viewing does not clear it. */
  readonly onBeforeMark?: () => void
  /** Touch layouts give each control a full touch target. */
  readonly touch?: boolean
}

const unconfirmed: ThreadRefusal = { detail: 'The unread change could not be confirmed. Refresh to check its current state.' }
const noVisibleReply: ThreadRefusal = { detail: 'Scroll to a visible message before marking it unread.' }

/**
 * The person's own unread reminder, kept apart from the natural unread count: an indicator with Jump when Fractal
 * reports a reminder, and Mark unread / Mark read only on connections holding the reminder grant.
 * An unavailable reminder read shows nothing, because it is not evidence either way.
 */
export function UnreadReminderControls({ threadId, port, visibleReplies, timeline = null, onBeforeMark, touch = false }: UnreadReminderControlsProps): React.ReactNode {
  const read = useAtomValue(port.reminder(threadId))
  const reminder = read._tag === 'Known' ? read.reminder : undefined
  // Marks must name the reminder IDs this view observed; without a known read there is nothing safe to send.
  const canMutate = port.canMark && read._tag === 'Known'
  const [pending, setPending] = React.useState(false)
  const [refusal, setRefusal] = React.useState<ThreadRefusal>()
  const [notice, setNotice] = React.useState<string>()
  const busy = React.useRef(false)

  const run = async (action: () => Promise<UnreadActionResult>, onSent?: () => void) => {
    if (busy.current) return
    busy.current = true
    setPending(true)
    setRefusal(undefined)
    setNotice(undefined)
    try {
      const result = await action()
      if (result._tag === 'Sent') onSent?.()
      else setRefusal({ detail: result.detail })
    } catch {
      setRefusal(unconfirmed)
    } finally {
      busy.current = false
      setPending(false)
    }
  }
  const markFrom = (anchor: string | undefined, current: ThreadUnreadReminder | undefined) => {
    if (anchor === undefined) {
      setNotice(undefined)
      setRefusal(noVisibleReply)
      return
    }
    void run(() => {
      onBeforeMark?.()
      return port.mark({ threadId, anchor_message_id: anchor, observed_marks: current?.marks.map(mark => mark.mark_id) ?? [] })
    })
  }

  // Option/Alt-click on a visible reply marks unread from that reply. Listeners read the latest render through a ref.
  const [awaiting, setAwaiting] = React.useState<string>()
  const latest = React.useRef({ canMutate, reminder, markFrom, visibleReplies, awaiting })
  latest.current = { canMutate, reminder, markFrom, visibleReplies, awaiting }
  const optionClick = React.useMemo(() => timeline === null ? noListener : Atom.make(get => {
    const click = (event: MouseEvent) => {
      if (!event.altKey || event.button !== 0 || !latest.current.canMutate || !(event.target instanceof Element)) return
      if (event.target.closest('a, button, input, textarea, select, [contenteditable]:not([contenteditable="false"])') !== null) return
      const row = event.target.closest(threadAnchorSelector)
      const anchor = row === null || !timeline.contains(row) ? null : row.getAttribute(threadAnchorAttribute)
      // A hidden, covered or offscreen row is not one the person deliberately chose.
      if (anchor === null || !latest.current.visibleReplies().includes(anchor)) return
      event.preventDefault()
      latest.current.markFrom(anchor, latest.current.reminder)
    }
    timeline.addEventListener('click', click)
    get.addFinalizer(() => timeline.removeEventListener('click', click))
    return undefined
  }), [timeline])
  useAtomValue(optionClick)

  // A found reply can still be committing to the timeline after seek hydrated it: watch once for that row, scoped to
  // this jump, and reveal it on arrival. A new jump, a new timeline or unmounting disposes the watch.
  const arrival = React.useMemo(() => timeline === null || awaiting === undefined ? noListener : Atom.make(get => {
    const observer = new MutationObserver(() => {
      // A superseded watch never moves the view, even before the registry finalizes it.
      if (latest.current.awaiting !== awaiting) { observer.disconnect(); return }
      const row = findAnchor(timeline, awaiting)
      if (row === undefined) return
      observer.disconnect()
      reveal(row)
      setAwaiting(undefined)
      setNotice(undefined)
    })
    observer.observe(timeline, { childList: true, subtree: true, attributes: true, attributeFilter: [threadAnchorAttribute] })
    get.addFinalizer(() => observer.disconnect())
    return undefined
  }), [timeline, awaiting])
  useAtomValue(arrival)

  const jump = async (current: ThreadUnreadReminder) => {
    if (timeline === null || busy.current) return
    busy.current = true
    setPending(true)
    setRefusal(undefined)
    setNotice(undefined)
    setAwaiting(undefined)
    try {
      const found = await port.seek({ threadId, anchor_message_id: current.anchor_message_id })
      // Neither outcome supplies a message body or changes the reminder; the reader sees only why Jump stopped.
      if (found._tag !== 'Found') { setNotice(found.detail); return }
      const row = findAnchor(timeline, found.messageId)
      if (row !== undefined) { reveal(row); return }
      setNotice('The reply marked unread is not loaded in this view.')
      setAwaiting(found.messageId)
    } catch {
      setNotice('Fractal could not locate the reply marked unread.')
    } finally {
      busy.current = false
      setPending(false)
    }
  }

  if (reminder === undefined && !canMutate) return null
  const option = detectPlatform() === 'mac' ? '⌥' : 'Alt'
  const button = [styles.button, touch && styles.touch]
  return <span data-thread-unread-reminder-controls="" {...stylex.props(styles.group)}>
    <span {...stylex.props(styles.row)}>
      {reminder === undefined ? null : <span data-thread-unread-reminder="" {...stylex.props(styles.indicator)}>
        <span aria-hidden="true" {...stylex.props(styles.bell)}><Icon name="bell" size={12} /></span>
        <span {...stylex.props(styles.wideLabel)}>Marked unread</span>
        <VisuallyHidden elementType="span" {...stylex.props(styles.compactOnly)}>Marked unread</VisuallyHidden>
      </span>}
      {reminder === undefined || timeline === null ? null : <Button aria-label="Jump to the reply marked unread" isDisabled={pending}
        onPress={() => { void jump(reminder) }} {...stylex.props(...button)}>Jump</Button>}
      {!canMutate || reminder === undefined ? null : <Button aria-label="Mark read, clearing the unread reminder" isDisabled={pending}
        onPress={() => { void run(() => port.clear({ threadId, observed_marks: reminder.marks.map(mark => mark.mark_id) })) }} {...stylex.props(...button)}>
        Mark read
      </Button>}
      {!canMutate ? null : <Tooltip label="Mark unread from the first visible reply" {...(timeline === null ? {} : { shortcut: `${option}-click a reply` })}>
        <Button aria-label="Mark unread from the first visible reply" isDisabled={pending}
          onPress={() => markFrom(visibleReplies()[0], reminder)} {...stylex.props(...button)}>
          Mark unread
        </Button>
      </Tooltip>}
    </span>
    {refusal === undefined ? null : <ThreadRefusalAlert refusal={refusal} />}
    {notice === undefined ? null : <p role="status" data-thread-unread-notice="" {...stylex.props(styles.notice)}>{notice}</p>}
  </span>
}

const noListener = Atom.make<undefined>(undefined)

const findAnchor = (timeline: HTMLElement, messageId: string): Element | undefined =>
  [...timeline.querySelectorAll(threadAnchorSelector)].find(row => row.getAttribute(threadAnchorAttribute) === messageId)
const reveal = (row: Element) => {
  const reduced = typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
  row.scrollIntoView({ block: 'center', behavior: reduced ? 'auto' : 'smooth' })
}

const styles = stylex.create({
  group: { display: 'flex', flexDirection: 'column', alignItems: 'flex-end', flexShrink: 0, minWidth: 0, maxWidth: '100%', gap: s.xs },
  row: { display: 'flex', flexWrap: 'wrap', justifyContent: 'flex-end', alignItems: 'center', maxWidth: '100%', gap: s.xs },
  indicator: { display: 'inline-flex', alignItems: 'center', gap: s.xs, whiteSpace: 'nowrap', color: c.fg, fontSize: t.metaSize, lineHeight: t.metaLeading, paddingInline: s.xs },
  bell: { display: 'inline-flex', color: c.primary },
  button: { flexShrink: 0, whiteSpace: 'nowrap', color: c.fgMuted, fontSize: t.metaSize,
    backgroundColor: 'transparent', borderWidth: 0, borderRadius: r.sm, paddingInline: s.sm, paddingBlock: s.xs, cursor: 'pointer',
    ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outline: `${g.focusRing} solid ${c.primary}` },
    ':disabled': { opacity: 0.5, cursor: 'default' } },
  touch: { minHeight: g.touchTarget },
  wideLabel: { display: { default: 'inline', '@container thread-panel (max-width: 360px)': 'none' } },
  compactOnly: { display: { default: 'none', '@container thread-panel (max-width: 360px)': 'inline' } },
  notice: { margin: 0, color: c.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere', textAlign: 'end' },
})
