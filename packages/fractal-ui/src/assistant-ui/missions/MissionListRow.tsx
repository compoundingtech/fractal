/**
 * One mission as a 48px row, shared by the left sidebar's Missions tab and the right panel's per-agent Missions section.
 *
 *   line 1: state mark · title (`title ?? goal`, never a hash) · ETA or age
 *   line 2: ring n/m · current step · agents (names, or a small stack past two)
 *
 * Colour only for action: the mark is amber when the mission needs you and red on failure. Every other state is a neutral
 * dot, filled while it moves (a live ring while working) and hollow while it waits or has settled. The row is display
 * only: the host owns activation (a list item's onAction, or spread `onClick`/`role` for an openable), like
 * `SidebarMissionChildRow`.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria-components'
import { LiveRing } from '../composition/LiveRing'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { MissionEta } from '../MissionEta'
import { MissionProgressMark } from '../sidebar/SidebarMissionLine'
import type { SidebarMissionProgress } from '../sidebar/model'

/** What the state mark says. Only `needs-you` and `failed` are hued. */
export type MissionRowTone = 'needs-you' | 'failed' | 'working' | 'waiting' | 'settled'

export interface MissionRowAgent {
  readonly id: string
  readonly label: string
}

/** Everything a mission row draws. Built by the app's adapter from the served mission and its presentation. */
export interface MissionRowData {
  /** `mission/<id>`: the ref the host opens. */
  readonly id: string
  /** Reader text: the mission's title, or its open step's `title ?? goal` when the title is a machine hash. */
  readonly title: string
  readonly tone: MissionRowTone
  /** The state in words, e.g. "Needs you", "Working", "Queued"; spoken and shown on hover. */
  readonly stateLabel: string
  readonly progress: SidebarMissionProgress
  /** What happens now: the current step, the question waiting on you, or why it is stuck. */
  readonly step?: string | undefined
  readonly agents: readonly MissionRowAgent[]
  /** The served ETA of the open run; settled missions pass none. */
  readonly eta?: MissionEta | null | undefined
  /** Last change, Unix ms: drawn as an age when there is no ETA. */
  readonly updatedAt: number
}

export interface MissionListRowProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'className' | 'style'> {
  readonly mission: MissionRowData
  readonly now: number
  /** The agent's own panel already names the agent: hide the agents column there. */
  readonly showAgents?: boolean
  /** The mission the workspace is showing. */
  readonly active?: boolean
  /** Last-observed snapshot: no live ring, muted ink. */
  readonly frozen?: boolean
}

const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const short = (ms: number): string => ms >= DAY ? `${Math.round(ms / DAY)}d` : ms >= HOUR ? `${Math.round(ms / HOUR)}h` : ms >= MINUTE ? `${Math.round(ms / MINUTE)}m` : '<1m'

/** `~2–4h` from the served p25–p75 bounds; `Finishing` for an open run with nothing left; null without an ETA. */
export const compactMissionEta = (eta: MissionEta | null | undefined): string | null => {
  if (eta === null || eta === undefined) return null
  if (eta.basis.samples === 0 && eta.remaining_low_ms === 0 && eta.remaining_high_ms === 0) return 'Finishing'
  const low = short(eta.remaining_low_ms)
  const high = short(eta.remaining_high_ms)
  return low === high ? `~${high}` : low.replace(/[a-z<]+$/, '') !== '' && low.slice(-1) === high.slice(-1) ? `~${low.slice(0, -1)}–${high}` : `~${low}–${high}`
}

/** Compact age since `at`: `4m`, `3h`, `2d`. */
export const compactAge = (at: number, now: number): string => short(Math.max(0, now - at))

/** `Ada`, `Ada, Bo`, `Ada +2`: names while they fit, a count past two. */
export const missionAgentsText = (agents: readonly MissionRowAgent[]): string =>
  agents.length <= 2 ? agents.map(agent => agent.label).join(', ') : `${agents[0]!.label} +${agents.length - 1}`

/** The row's accessible name: title, state, progress, step, agents, time. Never a hash. */
export const missionRowAccessibleName = (mission: MissionRowData, now: number): string => {
  const parts = [mission.title, mission.stateLabel]
  if (mission.progress._tag === 'Known') parts.push(`${mission.progress.done} of ${mission.progress.total} steps`)
  if (mission.step !== undefined && mission.step !== '') parts.push(mission.step)
  if (mission.agents.length > 0) parts.push(mission.agents.map(agent => agent.label).join(', '))
  const eta = compactMissionEta(mission.eta)
  parts.push(eta === null ? `updated ${compactAge(mission.updatedAt, now)} ago` : `about ${eta.replace(/^~/, '')} left`)
  return parts.join(', ')
}

/** The state mark: amber diamond for you, red for failure, a live ring while working, neutral dots otherwise. */
export function MissionStateMark({ tone: state, label, frozen = false }: { readonly tone: MissionRowTone; readonly label: string; readonly frozen?: boolean }) {
  return <span data-mission-tone={state} title={label} {...stylex.props(styles.markBox)}>
    {state === 'working' && !frozen ? <LiveRing size="sm" />
      : <span aria-hidden="true" {...stylex.props(styles.dot, state === 'needs-you' && styles.dotYou, state === 'failed' && styles.dotFailed, state === 'waiting' && styles.dotHollow, state === 'settled' && styles.dotSettled, state === 'working' && styles.dotWorking)} />}
    <VisuallyHidden>{label}</VisuallyHidden>
  </span>
}

export function MissionListRow({ mission, now, showAgents = true, active = false, frozen = false, ...rowProps }: MissionListRowProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const eta = compactMissionEta(mission.eta)
  const agents = showAgents ? missionAgentsText(mission.agents) : ''
  return <div {...rowProps} data-mission-row={mission.id} data-mission-tone={mission.tone} data-current={active || undefined} data-frozen={frozen || undefined} {...stylex.props(styles.row, narrow && styles.rowTouch, active && styles.active, frozen && styles.muted)}>
    <span {...stylex.props(styles.line1)}>
      <MissionStateMark tone={mission.tone} label={mission.stateLabel} frozen={frozen} />
      <span data-row-field="title" {...stylex.props(styles.title)}>{mission.title}</span>
      <span data-row-field={eta === null ? 'age' : 'eta'} aria-hidden="true" {...stylex.props(styles.trailing)}>{eta ?? compactAge(mission.updatedAt, now)}</span>
    </span>
    <span {...stylex.props(styles.line2)}>
      <span {...stylex.props(styles.progress)}><MissionProgressMark progress={mission.progress} retained={frozen} /></span>
      {mission.step !== undefined && mission.step !== '' ? <span data-row-field="step" {...stylex.props(styles.step, mission.tone === 'needs-you' && !frozen && styles.stepYou)}>{mission.step}</span>
        : <span data-row-field="step" {...stylex.props(styles.step)}>{mission.stateLabel}</span>}
      {agents !== '' ? <span data-row-field="agents" title={mission.agents.map(agent => agent.label).join(', ')} {...stylex.props(styles.agents)}>{agents}</span> : null}
    </span>
  </div>
}

/** A group's header in a grouped mission list: name and count, collapsible by the host. */
export function MissionGroupHeader({ label, count, tone: state, collapsed, onToggle }: { readonly label: string; readonly count: number; readonly tone?: MissionRowTone; readonly collapsed?: boolean; readonly onToggle?: () => void }) {
  const content = <>
    {onToggle !== undefined ? <span aria-hidden="true" {...stylex.props(styles.chevron, collapsed === true && styles.chevronCollapsed)}>▾</span> : null}
    <span {...stylex.props(styles.groupLabel, state === 'needs-you' && styles.groupYou, state === 'failed' && styles.groupFailed)}>{label}</span>
    <span {...stylex.props(styles.groupCount)}>{count}</span>
  </>
  return onToggle === undefined
    ? <div role="heading" aria-level={3} data-mission-group={label} {...stylex.props(styles.group)}>{content}</div>
    : <button type="button" aria-expanded={collapsed !== true} data-mission-group={label} onClick={onToggle} {...stylex.props(styles.group, styles.groupButton)}>{content}</button>
}

const styles = stylex.create({
  row: { display: 'grid', gridTemplateRows: `${g.controlSm} ${g.controlSm}`, alignContent: 'center', height: g.sidebarRow, boxSizing: 'border-box', minWidth: 0, width: '100%', paddingInline: s.md, color: ink.fgSoft, fontFamily: t.fontSans, cursor: 'pointer', outlineStyle: 'none', ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible],:focus-visible)': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  rowTouch: { minHeight: g.touchTarget },
  active: { backgroundColor: surface.rowActive, ':hover': { backgroundColor: surface.rowActive } },
  muted: { color: ink.fgMuted },
  line1: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, fontSize: t.uiSize, lineHeight: t.uiLeading, color: ink.fg },
  line2: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, paddingInlineStart: `calc(${g.statusSm} + ${s.sm})`, fontSize: t.denseSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  markBox: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.statusSm, height: g.statusSm },
  dot: { display: 'block', width: '7px', height: '7px', borderRadius: r.full, boxSizing: 'border-box', backgroundColor: ink.fgMuted },
  dotWorking: { backgroundColor: ink.fg },
  dotHollow: { backgroundColor: 'transparent', borderWidth: g.hairline, borderStyle: 'solid', borderColor: ink.fgMuted },
  dotSettled: { backgroundColor: border.borderStrong },
  dotYou: { width: '8px', height: '8px', borderRadius: '1px', transform: 'rotate(45deg)', backgroundColor: tone.attention },
  dotFailed: { backgroundColor: tone.dangerFg },
  title: { flex: '1 1 0', minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' },
  trailing: { flexShrink: 0, color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  progress: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0 },
  step: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' },
  stepYou: { color: tone.attention },
  agents: { flex: '0 1000 auto', minWidth: 0, marginInlineStart: 'auto', overflow: 'hidden', textOverflow: 'ellipsis', color: ink.fgMuted },
  group: { display: 'flex', alignItems: 'center', gap: s.xs, width: '100%', boxSizing: 'border-box', height: g.controlMd, margin: 0, paddingInline: s.md, borderWidth: 0, backgroundColor: 'transparent', color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, fontWeight: 600, letterSpacing: '0.02em', textAlign: 'start' },
  groupButton: { cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  chevron: { display: 'inline-block', width: '10px', transitionProperty: 'transform', transitionDuration: '120ms' },
  chevronCollapsed: { transform: 'rotate(-90deg)' },
  groupLabel: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  groupYou: { color: tone.attention },
  groupFailed: { color: tone.dangerFg },
  groupCount: { fontWeight: 400, color: ink.fgSoft, fontVariantNumeric: 'tabular-nums' },
})
