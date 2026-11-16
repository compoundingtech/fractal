// Time-bearing leaves support a contextual or explicit clock.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria-components'
import { geometryVars as g, spaceVars as s, statusVars as tone, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { compactTime, statuses, type AgentStatus } from './model'
import { useClockLabel } from '../../time/clock'
import { LiveRing } from '../composition/LiveRing'

export type GlyphVariant = 'SG-1' | 'SG-2' | 'SG-3' | 'Agents' | 'Marks'
export type DiscRefinement = 'G1' | 'G2' | 'G3'
export const glyphDescriptions: Record<GlyphVariant, string> = { 'SG-1': '8px discs in a fixed16px glyph column; refinements add running ring or canonical shape cues', 'SG-2': 'Quiet colored dot with the full canonical status word', 'SG-3': '16px stroke-only canonical status shapes', Agents: 'Agent activity vocabulary: ● ○ × ! ~; unknown activity has no glyph', Marks: 'Neutral filled and hollow state marks; amber and red discs, each with its own knockout, only when the row asks you to act' }
export interface SidebarStatusProps {
  readonly status: AgentStatus
  readonly statusLabel?: string
  readonly variant?: GlyphVariant
  readonly discRefinement?: DiscRefinement
  readonly statusSince?: number
  readonly now?: number
  readonly freshness?: 'live' | 'stale' | 'unobserved'
  readonly iconOnly?: boolean
  /** Stale is an observation gap, never an alarm: `muted` (default, every variant). `danger` exists only for taste comparison. */
  readonly staleTone?: 'muted' | 'danger'
  /**
   * Part of a last-observed snapshot: every tone goes muted, nothing spins, and SG-1 draws the shape cue for every state
   * (working and idle included), so states stay apart by shape alone. Independent of `freshness` and `status`.
   */
  readonly frozen?: boolean
  /** Beside text that already says the status: hidden from assistive tech. */
  readonly decorative?: boolean
  /** `Marks` only: draw the amber Needs-you disc. Failure draws its red disc from `status`, and outranks this. */
  readonly needsYou?: boolean
}
const paths: Record<AgentStatus, React.ReactNode> = {
  working: <><circle cx="10" cy="10" r="7" opacity=".4" /><path d="M10 3a7 7 0 0 1 7 7" /></>,
  waiting: <><circle cx="10" cy="10" r="7" /><path d="M7.7 7.5a2.3 2.3 0 1 1 3.9 1.7c-1 .6-1.6 1.1-1.6 2.3M10 14h.01" /></>,
  idle: <path d="M16.5 11.5A7 7 0 0 1 8.5 3.5a7 7 0 1 0 8 8Z" />,
  pending: <path d="M6 3h8M6 17h8M7 3v3l6 8v3M13 3v3l-6 8v3" />,
  failed: <path d="M10 4v8M10 16h.01" />,
  stale: <><circle cx="10" cy="10" r="7" strokeDasharray="2 2" /><path d="M10 6v4l3 2" /></>,
  offline: <path d="M3 8a11 11 0 0 1 14 0M6 11a6 6 0 0 1 8 0M9 14a2 2 0 0 1 2 0M3 3l14 14" />,
  ended: <><circle cx="10" cy="10" r="7" /><rect x="7" y="7" width="6" height="6" rx=".5" fill="currentColor" stroke="none" /></>,
  suspended: <><circle cx="10" cy="10" r="7" /><path d="M8 7v6M12 7v6" /></>,
  retired: <><rect x="3" y="4" width="14" height="3" rx="1" /><path d="M4 7v9h12V7M8 10h4" /></>,
  unobserved: <><circle cx="10" cy="10" r="7" strokeDasharray="2 3" /><path d="M10 7v4M10 14h.01" /></>,
}
/**
 * The row-system state marks, in a 16 box. Filled is active and hollow is at rest; every other state differs by shape or
 * stroke weight, so nothing here depends on color. Waiting is the heavier hollow ring beside idle's thin one.
 */
const marks: Record<AgentStatus, React.ReactNode> = {
  working: <circle cx="8" cy="8" r="4.75" fill="currentColor" stroke="none" />,
  idle: <circle cx="8" cy="8" r="4.75" strokeWidth="1.25" />,
  waiting: <circle cx="8" cy="8" r="4.5" strokeWidth="2" />,
  pending: <circle cx="8" cy="8" r="4.75" strokeDasharray=".01 2.98" />,
  stale: <><circle cx="8" cy="8" r="4.75" strokeDasharray="1.8 2.2" /><circle cx="8" cy="8" r="2" fill="currentColor" stroke="none" /></>,
  offline: <><circle cx="8" cy="8" r="4.75" /><path d="M4.75 4.75l6.5 6.5" /></>,
  ended: <circle cx="8" cy="8" r="2" fill="currentColor" stroke="none" />,
  suspended: <><circle cx="8" cy="8" r="4.75" /><path d="M6.75 6.25v3.5M9.25 6.25v3.5" strokeWidth="1.25" /></>,
  retired: <circle cx="8" cy="8" r="2.25" strokeWidth="1.25" />,
  unobserved: <circle cx="8" cy="8" r="4.75" strokeDasharray="1.8 2.2" />,
  failed: null,
}
/** Attention disc: filled, with a real cut-out (so any row wash shows through) that keeps ask and failure apart without color. */
function AttentionDisc({ knockout }: { readonly knockout: string }) {
  const id = React.useId()
  return <><mask id={id}><rect width="16" height="16" fill="#fff" stroke="none" /><path d={knockout} stroke="#000" /></mask><circle cx="8" cy="8" r="6.25" fill="currentColor" stroke="none" mask={`url(#${id})`} /></>
}
const needsYouKnockout = 'M6.1 6.2a1.95 1.95 0 1 1 3.05 1.6c-.75.5-1.15.95-1.15 1.8M8 11.9h.01'
const failedKnockout = 'M8 4.4v4.3M8 11.5h.01'
/** No activity is invented for observation-only states with no retained status. */
const agentGlyphs: Partial<Record<AgentStatus, string>> = {
  working: '●', idle: '○', waiting: '○', suspended: '○',
  ended: '×', retired: '×', failed: '!', pending: '~',
}
/** Shapes never collapse distinct states into an attention or completion fixture state. */
export function SidebarStatus({ status, statusLabel = statuses[status].label, variant = 'SG-1', discRefinement = 'G1', statusSince, now, freshness = 'unobserved', iconOnly = false, staleTone = 'muted', frozen: snapshotFrozen = false, decorative = false, needsYou = false }: SidebarStatusProps) {
  const frozen = snapshotFrozen || freshness === 'stale' || (variant === 'Agents' && freshness === 'unobserved')
  const boundary = statusSince === undefined ? 'Status boundary unavailable' : `Observed status since ${new Date(statusSince).toISOString()}`
  const elapsed = useClockLabel(now => statusSince === undefined || statusSince > now || iconOnly ? '' : compactTime({ at: statusSince, now }), statusSince === undefined || iconOnly ? 0 : now)
  // The roster disc stays a disc; only the G2 refinement and the stroke-only SG-3 set draw the shared ring for working.
  const liveRing = status === 'working' && (variant === 'SG-3' || (variant === 'SG-1' && discRefinement === 'G2' && !frozen))
  const shapeCue = variant === 'SG-1' && (frozen || (discRefinement === 'G3' && status !== 'working' && status !== 'idle'))
  const snapshot = frozen ? '; last-observed snapshot' : ''
  // The former native title text stays exact as the glyph's description; hosts own the only hover disclosure.
  const descriptionId = React.useId()
  if (variant === 'Marks') {
    const mark = status === 'failed' ? 'failed' : needsYou ? 'needs-you' : status
    // Every row speaks this inside its name, so it says the state alone; a qualifier only when the observation is not live.
    const qualified = freshness === 'live' && !frozen ? statusLabel : `${statusLabel}; ${freshness === 'live' ? 'last-observed snapshot' : `${freshness} observation`}`
    return <span aria-hidden={decorative || undefined} data-status={status} data-mark={mark} data-frozen={frozen || undefined} data-status-frozen={frozen || undefined} {...stylex.props(styles.root, markInk[mark], frozen && styles.frozen)}>
      <svg role="img" aria-label={qualified} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...stylex.props(styles.icon)}>{mark === 'failed' ? <AttentionDisc knockout={failedKnockout} /> : mark === 'needs-you' ? <AttentionDisc knockout={needsYouKnockout} /> : marks[mark]}</svg>
      {elapsed !== '' && !iconOnly && <time dateTime={new Date(statusSince!).toISOString()} aria-label={`Elapsed in ${statusLabel}: ${elapsed}`}>{elapsed}</time>}
    </span>
  }
  return <span aria-hidden={decorative || undefined} data-status={status} data-frozen={frozen || undefined} data-status-frozen={frozen || undefined} {...stylex.props(styles.root, status === 'working' && styles.working, status === 'waiting' && styles.waiting, status === 'failed' && styles.staleDanger, status === 'stale' && (staleTone === 'danger' ? styles.staleDanger : styles.staleMuted), frozen && styles.frozen)}>
    {variant === 'Agents' ? <span role="img" aria-label={`${statusLabel}; ${boundary}; ${freshness} observation${snapshot}`} aria-describedby={descriptionId} data-agent-glyph {...stylex.props(styles.agentGlyph)}>{agentGlyphs[status]}</span> : liveRing ? <span data-disc-refinement={variant === 'SG-1' ? discRefinement : undefined} role="img" aria-label={`${statusLabel}; ${boundary}; ${freshness} observation${snapshot}`} aria-describedby={descriptionId} {...stylex.props(styles.ringSlot)}><LiveRing spinning={!frozen} /></span> : <svg data-disc-refinement={variant === 'SG-1' ? discRefinement : undefined} role="img" aria-label={`${statusLabel}; ${boundary}; ${freshness} observation${snapshot}`} aria-describedby={descriptionId} viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...stylex.props(styles.icon)}>{variant === 'SG-2' ? <circle cx="10" cy="10" r="4" fill="currentColor" stroke="none" /> : variant === 'SG-3' ? paths[status] : <><circle cx="10" cy="10" r="5" fill="currentColor" stroke="none" opacity={shapeCue ? '.25' : undefined} />{shapeCue && paths[status]}</>}</svg>}
    <VisuallyHidden elementType="span"><span id={descriptionId}>{`${statusLabel}. ${boundary}. Observation ${freshness}.${frozen ? ' Last-observed snapshot.' : ''}`}</span></VisuallyHidden>
    {variant === 'SG-2' && !iconOnly && <span>{statusLabel}</span>}
    {/* Icon-only glyphs sit in a fixed icon column; visible text there would overrun the next column. */}
    {elapsed !== '' && !iconOnly && <time dateTime={new Date(statusSince!).toISOString()} aria-label={`Elapsed in ${statusLabel}: ${elapsed}`}>{elapsed}</time>}
  </span>
}
const styles = stylex.create({
  root: { display: 'inline-flex', alignItems: 'center', gap: s.xs, color: ink.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', flexShrink: 0 },
  icon: { width: g.icon, height: g.icon, flexShrink: 0 }, ringSlot: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, flexShrink: 0 },
  agentGlyph: { width: g.icon, textAlign: 'center', flexShrink: 0 },
  working: { color: tone.runningFg }, waiting: { color: tone.attention }, staleDanger: { color: tone.dangerFg }, staleMuted: { color: ink.fgMuted },
  // Last observed: one muted tone for every state, so nothing reads as live; the SG-1 shape cue carries the state.
  frozen: { color: ink.fgMuted },
})
const markInk = stylex.create({
  'needs-you': { color: tone.attention }, failed: { color: tone.dangerFg },
  working: { color: ink.fg }, idle: { color: ink.fgSoft }, waiting: { color: ink.fgSoft },
  pending: { color: ink.fgMuted }, stale: { color: ink.fgMuted }, offline: { color: ink.fgMuted }, ended: { color: ink.fgMuted },
  suspended: { color: ink.fgMuted }, retired: { color: ink.fgMuted }, unobserved: { color: ink.fgMuted },
})
