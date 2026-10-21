import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { HumanTime } from '../../time/HumanTime'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { DetailsReadFailure, DetailsSpendRead } from './model'

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
  return <Button ref={buttonRef} isPending={pending} onPress={onPress} aria-describedby={describedBy} data-details-read-action={action} data-details-read-pending={pending || undefined} {...stylex.props(styles.action, narrow && styles.actionTouch, pending && styles.actionPending)}>{children}</Button>
}

/** The failure sentence; a read that kept earlier figures dates them, so no separate `Updated …` line is needed. */
function FailureText({ failure }: { readonly failure: DetailsReadFailure }) {
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
 * One failed read: its plain sentence in a polite status region and a Try again that re-runs only that read.
 * While the retry is `pending` the line and its action stay mounted as **Trying again…**; a repeated failure
 * replaces the sentence in place, so focus stays on the action. A success first moves focus from the action to
 * its section's heading control (only when the action holds focus), then removes the line.
 */
export function DetailsReadFailureLine({ failure, pending = false }: { readonly failure: DetailsReadFailure | undefined; readonly pending?: boolean }) {
  const id = React.useId()
  const button = React.useRef<HTMLButtonElement>(null)
  const [held, release] = useHeldFailure(failure)
  const recovered = failure === undefined && !pending && held !== undefined
  React.useLayoutEffect(() => {
    if (!recovered) return
    const action = button.current
    if (action !== null && action === action.ownerDocument.activeElement) action.closest('section')?.querySelector<HTMLElement>('[data-details-section-control]')?.focus({ preventScroll: true })
    release(undefined)
  }, [recovered, release])
  const shown = failure ?? held
  if (shown === undefined) return null
  const busy = failure === undefined && pending
  return <div data-details-read-failure {...stylex.props(styles.line)}>
    <div id={id} role="status" {...stylex.props(styles.text)}><FailureText failure={shown} /></div>
    <ReadAction action="retry" buttonRef={button} pending={busy} describedBy={id} onPress={shown.onRetry}>{busy ? 'Trying again…' : 'Try again'}</ReadAction>
  </div>
}

/**
 * Spend is read on open and on request, so its age is always visible: `Updated …` and a Refresh action. A failed
 * read replaces the date with its own sentence (dating any kept figures) in a polite status region, and the same
 * button becomes Try again; in flight it reads **Refreshing…** or **Trying again…**. One button node persists
 * across every state, so focus stays on it.
 */
export function DetailsSpendReadLine({ read }: { readonly read: DetailsSpendRead }) {
  const narrow = React.useContext(NarrowLayoutContext)
  const statusId = React.useId()
  const updatedId = React.useId()
  const button = React.useRef<HTMLButtonElement>(null)
  const [held, release] = useHeldFailure(read.failure)
  const settled = read.failure === undefined && !read.refreshing && held !== undefined
  React.useLayoutEffect(() => { if (settled) release(undefined) }, [settled, release])
  const failure = read.failure ?? (read.refreshing ? held : undefined)
  const pending = read.refreshing
  const label = failure === undefined ? pending ? 'Refreshing…' : 'Refresh' : pending ? 'Trying again…' : 'Try again'
  return <div data-details-spend-read data-details-read-failure={failure !== undefined || undefined} {...stylex.props(styles.line)}>
    {/* Mounted before any failure arrives, so the sentence is announced when it appears. */}
    <div id={statusId} role="status" {...stylex.props(failure === undefined ? styles.statusIdle : styles.text)}>{failure !== undefined && <FailureText failure={failure} />}</div>
    {failure === undefined && <div id={updatedId} data-details-spend-updated {...stylex.props(styles.text)}>{read.updatedAt === undefined ? null : <HumanTime at={read.updatedAt} prefix="Updated " tooltip={!narrow} />}</div>}
    <ReadAction action={failure === undefined ? 'refresh' : 'retry'} buttonRef={button} pending={pending} describedBy={failure === undefined ? updatedId : statusId} onPress={failure?.onRetry ?? read.onRefresh}>{label}</ReadAction>
  </div>
}

const styles = stylex.create({
  line: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.xs, margin: s.zero, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  text: { minWidth: 0, overflowWrap: 'normal' },
  // An empty status region stays in the accessibility tree but out of the line's flow and gaps.
  statusIdle: { position: 'absolute', width: 1, height: 1, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
  action: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outline: `2px solid ${accent.primary}` } },
  actionTouch: { height: g.touchTarget, minWidth: g.touchTarget, paddingInline: s.md },
  actionPending: { color: ink.fgMuted, cursor: 'default', ':hover': { backgroundColor: surface.transparent } },
})
