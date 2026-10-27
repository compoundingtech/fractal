import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, VisuallyHidden } from 'react-aria-components'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { HumanTime, useClockLabel } from '../../time/HumanTime'
import { formatRelative } from '../../time/human-time'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { DetailsReadFailure, DetailsSpendRead } from './model'

type FailureSentence = { readonly message: string; readonly retainedAt?: number | undefined }

/**
 * Before an outcome removes a focused read action, focus moves to its section's heading control. Every section
 * keeps that control mounted, as the same element, in every read state, so it is always a stable target.
 */
function handOffFocus(action: HTMLElement | null): void {
  if (action === null || action !== action.ownerDocument.activeElement) return
  action.closest('section')?.querySelector<HTMLElement>('[data-details-section-control]')?.focus({ preventScroll: true })
}

/**
 * The plain-text failure sentence a read last settled with, for its polite announcer. Each settled failure clears
 * the text and sets it again on the next animation frame, so a repeated identical failure is announced again.
 * Only a semantic change speaks: a read settling (`pending` → not pending) or a new sentence. Parent re-renders that
 * recreate an equal failure object stay silent.
 */
function useFailureAnnouncement(failure: FailureSentence | undefined, pending: boolean): string | undefined {
  const [announced, setAnnounced] = React.useState<string>()
  const read = React.useRef<{ wasPending: boolean; settles: number; key: string | undefined }>({ wasPending: false, settles: 0, key: undefined })
  const message = failure?.message
  const retainedAt = failure?.retainedAt
  const label = useClockLabel((now, zone) => retainedAt === undefined ? '' : formatRelative(retainedAt, now, zone))
  const latestLabel = React.useRef(label)
  latestLabel.current = label
  React.useLayoutEffect(() => {
    const state = read.current
    if (pending) { state.wasPending = true; return }
    if (state.wasPending) { state.wasPending = false; state.settles += 1 }
    if (message === undefined) { state.key = undefined; setAnnounced(undefined); return }
    const key = `${state.settles}\u0000${retainedAt ?? ''}\u0000${message}`
    if (key === state.key) return
    state.key = key
    setAnnounced(undefined)
    let frame = requestAnimationFrame(() => {
      frame = 0
      setAnnounced(retainedAt === undefined ? message : `${message} Showing figures from ${latestLabel.current}.`)
    })
    // A superseded or unmounted frame never speaks; its key is forgotten so the next settled run announces.
    return () => { if (frame !== 0) { cancelAnimationFrame(frame); state.key = undefined } }
  }, [pending, message, retainedAt])
  return announced
}

/** A read's polite status region: mounted empty from the first render, it only ever holds a settled failure sentence. */
function ReadAnnouncer({ failure, pending }: { readonly failure: FailureSentence | undefined; readonly pending: boolean }) {
  const text = useFailureAnnouncement(failure, pending)
  return <VisuallyHidden><span role="status" aria-live="polite" aria-atomic="true" data-details-read-announcer>{text}</span></VisuallyHidden>
}

/**
 * A quiet text action; the phone sheet raises it to the 44px touch floor. While its read is in flight the same
 * node stays mounted and focusable, `aria-disabled` and `aria-busy` rather than natively disabled, so keyboard
 * focus never falls out of the pane.
 */
function ReadAction({ children, onPress, describedBy, pending, action, buttonRef }: {
  readonly children: string
  readonly onPress: () => void
  readonly describedBy: string
  readonly pending: boolean
  readonly action: 'retry' | 'refresh'
  readonly buttonRef: React.RefObject<HTMLButtonElement | null>
}) {
  const narrow = React.useContext(NarrowLayoutContext)
  // React Aria forwards no `aria-busy`; the attribute is owned here and never rendered by React.
  React.useLayoutEffect(() => {
    const button = buttonRef.current
    if (button === null) return
    if (pending) button.setAttribute('aria-busy', 'true')
    else button.removeAttribute('aria-busy')
  }, [pending, buttonRef])
  return <Button ref={buttonRef} aria-disabled={pending || undefined} onPress={() => { if (!pending) onPress() }} aria-describedby={describedBy} data-details-read-action={action} data-details-read-pending={pending || undefined} {...stylex.props(styles.action, narrow && styles.actionTouch, pending && styles.actionPending)}>{children}</Button>
}

/** The visible failure sentence; a read that kept earlier figures dates them, so no separate `Updated …` line is needed. */
function FailureText({ failure }: { readonly failure: FailureSentence }) {
  const narrow = React.useContext(NarrowLayoutContext)
  return <>{failure.message}{failure.retainedAt !== undefined && <> Showing figures from <HumanTime at={failure.retainedAt} tooltip={!narrow} />.</>}</>
}

/**
 * The last failure a read reported, kept while its retry is in flight: hosts clear `failure` during a pending read,
 * but the sentence and its action stay mounted until the read settles and the caller releases it.
 */
function useHeldFailure(failure: DetailsReadFailure | undefined): readonly [DetailsReadFailure | undefined, (value: undefined) => void] {
  const [held, setHeld] = React.useState(failure)
  if (failure !== undefined && failure !== held) setHeld(failure)
  return [held, setHeld]
}
/**
 * One failed read: its plain sentence (`data-details-read-message`) and a Try again that re-runs only that read, with
 * a separate polite announcer that speaks each settled failure. While the retry is `pending` the line and its action
 * stay mounted as **Trying again…**; a repeated failure replaces the sentence in place, so focus stays on the action.
 * Any other settled outcome (success, refused, unsupported) first moves focus from the action to its section's
 * heading control, then removes the line.
 */
export function DetailsReadFailureLine({ failure, pending = false }: { readonly failure: DetailsReadFailure | undefined; readonly pending?: boolean }) {
  const id = React.useId()
  const button = React.useRef<HTMLButtonElement>(null)
  const [held, release] = useHeldFailure(failure)
  const settled = failure === undefined && !pending && held !== undefined
  React.useLayoutEffect(() => {
    if (!settled) return
    handOffFocus(button.current)
    release(undefined)
  }, [settled, release])
  const shown = failure ?? held
  const busy = failure === undefined && pending
  return <>
    <ReadAnnouncer failure={failure} pending={pending} />
    {shown !== undefined && <div data-details-read-failure {...stylex.props(styles.line)}>
      <div id={id} data-details-read-message {...stylex.props(styles.text)}><FailureText failure={shown} /></div>
      <ReadAction action="retry" buttonRef={button} pending={busy} describedBy={id} onPress={shown.onRetry}>{busy ? 'Trying again…' : 'Try again'}</ReadAction>
    </div>}
  </>
}

/**
 * Spend is read on open and on request, so its age is always visible: `Updated …` and a Refresh action. A failed
 * read replaces the date with its own sentence (dating any kept figures), announced by the polite announcer, and the
 * same button becomes Try again; in flight it reads **Refreshing…** or **Trying again…**. One button node persists
 * across every state, so focus stays on it. When the host withdraws the read (`read` becomes undefined: refused or
 * unsupported) the last read stays painted for one layout pass while focus moves to the Spend heading, then goes.
 */
export function DetailsSpendReadLine({ read }: { readonly read: DetailsSpendRead | undefined }) {
  const narrow = React.useContext(NarrowLayoutContext)
  const messageId = React.useId()
  const updatedId = React.useId()
  const button = React.useRef<HTMLButtonElement>(null)
  const [kept, setKept] = React.useState(read)
  if (read !== undefined && read !== kept) setKept(read)
  const withdrawn = read === undefined && kept !== undefined
  React.useLayoutEffect(() => {
    if (!withdrawn) return
    handOffFocus(button.current)
    setKept(undefined)
  }, [withdrawn])
  const refreshing = read?.refreshing ?? false
  const [held, release] = useHeldFailure(read?.failure)
  const settled = read?.failure === undefined && !refreshing && held !== undefined
  React.useLayoutEffect(() => { if (settled) release(undefined) }, [settled, release])
  const shown = read ?? kept
  const failure = shown?.failure ?? (shown?.refreshing === true ? held : undefined)
  const pending = shown?.refreshing === true
  const label = failure === undefined ? pending ? 'Refreshing…' : 'Refresh' : pending ? 'Trying again…' : 'Try again'
  return <>
    <ReadAnnouncer failure={read?.failure} pending={refreshing} />
    {shown !== undefined && <div data-details-spend-read data-details-read-failure={failure !== undefined || undefined} {...stylex.props(styles.line)}>
      {failure !== undefined
        ? <div id={messageId} data-details-read-message {...stylex.props(styles.text)}><FailureText failure={failure} /></div>
        : <div id={updatedId} data-details-spend-updated {...stylex.props(styles.text)}>{shown.updatedAt === undefined ? null : <HumanTime at={shown.updatedAt} prefix="Updated " tooltip={!narrow} />}</div>}
      <ReadAction action={failure === undefined ? 'refresh' : 'retry'} buttonRef={button} pending={pending} describedBy={failure === undefined ? updatedId : messageId} onPress={failure?.onRetry ?? shown.onRefresh}>{label}</ReadAction>
    </div>}
  </>
}

const styles = stylex.create({
  line: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.xs, margin: s.zero, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  text: { minWidth: 0, overflowWrap: 'normal' },
  action: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outline: `2px solid ${accent.primary}` } },
  actionTouch: { height: g.touchTarget, minWidth: g.touchTarget, paddingInline: s.md },
  actionPending: { color: ink.fgMuted, cursor: 'default', ':hover': { backgroundColor: surface.transparent } },
})
