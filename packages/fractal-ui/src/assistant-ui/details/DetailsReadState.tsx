import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { HumanTime } from '../../time/HumanTime'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { DetailsReadFailure, DetailsSpendRead } from './model'

/** A quiet text action; the phone sheet raises it to the 44px touch floor. */
function ReadAction({ children, onPress, describedBy, ...data }: { readonly children: string; readonly onPress: () => void; readonly describedBy?: string; readonly 'data-details-read-action': 'retry' | 'refresh' }) {
  const narrow = React.useContext(NarrowLayoutContext)
  return <Button onPress={onPress} aria-describedby={describedBy} {...data} {...stylex.props(styles.action, narrow && styles.actionTouch)}>{children}</Button>
}

/** One failed read: its plain sentence and a Try again that re-runs only that read. */
export function DetailsReadFailureLine({ failure }: { readonly failure: DetailsReadFailure }) {
  const id = React.useId()
  return <div data-details-read-failure {...stylex.props(styles.line)}>
    <span id={id} role="status" {...stylex.props(styles.text)}>{failure.message}</span>
    <ReadAction data-details-read-action="retry" describedBy={id} onPress={failure.onRetry}>Try again</ReadAction>
  </div>
}

/**
 * Spend is read on open and on request, so its age is always visible: `Updated …` and a Refresh
 * action. A failed read keeps the last figures (still dated) beside its own Try again.
 */
export function DetailsSpendReadLine({ read }: { readonly read: DetailsSpendRead }) {
  const narrow = React.useContext(NarrowLayoutContext)
  const id = React.useId()
  const updated = read.updatedAt === undefined ? null : <HumanTime at={read.updatedAt} prefix="Updated " tooltip={!narrow} />
  if (read.failure !== undefined) return <>
    <DetailsReadFailureLine failure={read.failure} />
    {updated !== null && <div data-details-spend-updated {...stylex.props(styles.line, styles.text)}>{updated}</div>}
  </>
  return <div data-details-spend-read {...stylex.props(styles.line)}>
    <div id={id} data-details-spend-updated {...stylex.props(styles.text)}>{read.refreshing ? 'Refreshing…' : updated}</div>
    {/* Pressing during an in-flight read is a no-op at the source; the button stays focusable. */}
    <ReadAction data-details-read-action="refresh" describedBy={id} onPress={read.onRefresh}>Refresh</ReadAction>
  </div>
}

const styles = stylex.create({
  line: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.xs, margin: s.zero, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  text: { minWidth: 0, overflowWrap: 'normal' },
  action: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outline: `2px solid ${accent.primary}` } },
  actionTouch: { height: g.touchTarget, minWidth: g.touchTarget, paddingInline: s.md },
})
