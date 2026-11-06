/**
 * Mission runs related to one agent, drawn exactly as `GET /v1/client/agent-related-runs/{agent}` serves them. The server
 * owns the status chip, done/total, ETA, `waiting_on_you`, `late`, urgency order and `folded`; this list keeps the served
 * order, draws active rows first and finished rows (retained 24 h) behind a native fold, and never joins or polls. Hosts
 * replace `state` when a push invalidates the read and keep the previous Loaded value while they refetch.
 *
 * Line 1: status chip, mission, done/total bar, then "waiting on you" or the served ETA (danger ink when late).
 * Line 2: the current step and who it waits on.
 */
import * as stylex from '@stylexjs/stylex'
import { GridList, GridListItem, type Key } from 'react-aria-components'
import { MissionEtaLabel } from '../MissionEta'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { AgentRelatedRun, AgentRelatedRuns, RelatedRunWait } from '../../../../../clients/typescript/fractal-client/Models.generated.ts'

export type SidebarRelatedRun = AgentRelatedRun
export type SidebarRelatedRunsState =
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Failed'; readonly message: string }
  | { readonly _tag: 'Loaded'; readonly value: AgentRelatedRuns }
export interface SidebarRelatedRunsProps {
  readonly state: SidebarRelatedRunsState
  /** Accessible name of the section. */
  readonly label?: string
  /** Present when the host can open a run; every row then becomes an action. */
  readonly onOpenRun?: (run: SidebarRelatedRun) => void
  /** Fixes the ETA description's age; omitted, it follows the context clock. */
  readonly now?: number
}

type Chip = AgentRelatedRun['status_chip']
const chipLabel: Record<Chip, string> = { running: 'Running', waiting: 'Waiting', 'waiting-on-you': 'Needs you', stalled: 'Stalled', completed: 'Done', failed: 'Failed', cancelled: 'Cancelled' }
/** Last segment of a canonical ref: `mission/sidebar-polish` reads `sidebar-polish`. */
const refName = (ref: string) => ref.slice(ref.lastIndexOf('/') + 1)
const waitName = (wait: RelatedRunWait) => wait.who === undefined ? wait.kind : refName(wait.who)
const stepName = (wait: RelatedRunWait) => wait.title ?? wait.step

/** Plain-text row summary, the row's typeahead text. */
export const relatedRunText = (run: SidebarRelatedRun): string => [
  chipLabel[run.status_chip], refName(run.mission_id), `${run.progress.done} of ${run.progress.total} steps`,
  run.waiting_on_you ? 'waiting on you' : run.late ? 'late' : undefined,
  run.current_step === null ? undefined : stepName(run.current_step),
  run.waits_on.length === 0 ? undefined : `waits on ${run.waits_on.map(waitName).join(', ')}`,
].filter(part => part !== undefined).join(', ')

function RunRow({ run, now }: { readonly run: SidebarRelatedRun; readonly now: number | undefined }) {
  const { done, total } = run.progress
  const waits = run.waits_on.map(waitName).join(', ')
  return <>
    <span {...stylex.props(styles.line)}>
      <span data-chip={run.status_chip} {...stylex.props(styles.chip, chipTone[run.status_chip])}>{chipLabel[run.status_chip]}</span>
      <span {...stylex.props(styles.ellipsis, styles.mission)}>{refName(run.mission_id)}</span>
      <span aria-hidden="true" {...stylex.props(styles.track)}><span {...stylex.props(styles.fill(total === 0 ? '0%' : `${Math.min(100, (done / total) * 100)}%`))} /></span>
      <span {...stylex.props(styles.metric)}>{done}/{total}</span>
      {run.waiting_on_you
        ? <span data-waiting-on-you {...stylex.props(styles.metric, styles.attention)}>waiting on you</span>
        : run.eta !== null
          ? <span data-late={run.late || undefined} {...stylex.props(styles.metric, run.late && styles.late)}><MissionEtaLabel eta={run.eta} now={now} /></span>
          : run.late ? <span data-late {...stylex.props(styles.metric, styles.late)}>late</span> : null}
    </span>
    <span {...stylex.props(styles.line, styles.detail)}>
      <span {...stylex.props(styles.ellipsis)}>{run.current_step === null ? run.status : stepName(run.current_step)}</span>
      {waits === '' ? null : <><span aria-hidden="true">·</span><span {...stylex.props(styles.ellipsis)}>waits on {waits}</span></>}
    </span>
  </>
}

function RunList({ label, runs, onOpenRun, now }: { readonly label: string; readonly runs: readonly SidebarRelatedRun[]; readonly onOpenRun: SidebarRelatedRunsProps['onOpenRun']; readonly now: number | undefined }) {
  const onAction = onOpenRun === undefined ? undefined : (key: Key) => { const run = runs.find(candidate => candidate.id === key); if (run !== undefined) onOpenRun(run) }
  return <GridList aria-label={label} selectionMode="none" items={runs} onAction={onAction} {...stylex.props(styles.list)}>
    {run => <GridListItem id={run.id} textValue={relatedRunText(run)} data-run={run.id} {...stylex.props(styles.row, onAction !== undefined && styles.actionable)}>
      <RunRow run={run} now={now} />
    </GridListItem>}
  </GridList>
}

export function SidebarRelatedRuns({ state, label = 'Related missions', onOpenRun, now }: SidebarRelatedRunsProps) {
  const body = state._tag === 'Loading'
    ? <p role="status" {...stylex.props(styles.note)}>Loading related missions…</p>
    : state._tag === 'Failed'
      ? <p role="status" data-failed {...stylex.props(styles.note, styles.late)}>Related missions unavailable: {state.message}</p>
      : <Loaded value={state.value} label={label} onOpenRun={onOpenRun} now={now} />
  return <section aria-label={label} aria-busy={state._tag === 'Loading' || undefined} {...stylex.props(styles.root)}>{body}</section>
}

function Loaded({ value, label, onOpenRun, now }: { readonly value: AgentRelatedRuns; readonly label: string; readonly onOpenRun: SidebarRelatedRunsProps['onOpenRun']; readonly now: number | undefined }) {
  // Server order is kept: filtering partitions without reordering.
  const active = value.rows.filter(run => !run.folded)
  const finished = value.rows.filter(run => run.folded)
  return <>
    {value.rows.length === 0 ? <p {...stylex.props(styles.note)}>No related missions</p> : null}
    {active.length === 0 ? null : <RunList label={label} runs={active} onOpenRun={onOpenRun} now={now} />}
    {finished.length === 0 ? null : <details key={`finished:${value.agent_id}`} data-fold="finished" {...stylex.props(styles.fold)}>
      <summary {...stylex.props(styles.summary)}>Finished · {finished.length}</summary>
      <RunList label={`${label}, finished in the last 24 hours`} runs={finished} onOpenRun={onOpenRun} now={now} />
    </details>}
    {value.truncated ? <p data-truncated {...stylex.props(styles.note)}>Showing {value.rows.length} of {value.total_rows} runs</p> : null}
  </>
}

// Focus rings are inset so the row's rounded box stays inside the sidebar's clip.
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
  list: { display: 'flex', flexDirection: 'column', minWidth: 0, outline: 'none' },
  row: {
    display: 'flex', flexDirection: 'column', gap: s.xs2, minWidth: 0, paddingBlock: s.xs, paddingInline: s.sm, borderRadius: r.sm, outline: 'none', cursor: 'default',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: `calc(${g.focusRing} * -1)` },
  },
  actionable: { cursor: 'pointer', backgroundColor: { default: surface.transparent, ':hover': surface.rowHover } },
  line: { display: 'flex', alignItems: 'center', gap: s.xs, minWidth: 0 },
  detail: { color: ink.fgMuted },
  ellipsis: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  mission: { flex: '1 1 0' },
  chip: { flexShrink: 0, paddingInline: s.xs, borderRadius: r.full, fontSize: t.denseSize, lineHeight: '16px', fontWeight: t.weightMedium, whiteSpace: 'nowrap' },
  track: { flex: '0 0 28px', height: '4px', overflow: 'hidden', borderRadius: r.full, backgroundColor: `color-mix(in srgb, ${ink.fgMuted} 28%, transparent)` },
  fill: (width: string) => ({ display: 'block', width, height: '100%', backgroundColor: ink.fgSoft }),
  metric: { flexShrink: 0, color: ink.fgMuted, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  attention: { color: tone.attention },
  late: { color: tone.dangerFg },
  note: { margin: 0, paddingInline: s.sm, color: ink.fgMuted },
  fold: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  summary: {
    paddingBlock: s.xs, paddingInline: s.sm, borderRadius: r.sm, color: ink.fgMuted, cursor: 'pointer', outline: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: `calc(${g.focusRing} * -1)` },
  },
})
// Chip text keeps its hue; the box takes a 14% wash of it, as the mission pill does.
const chipTone = stylex.create({
  running: { color: tone.runningFg, backgroundColor: `color-mix(in srgb, ${tone.runningFg} 14%, transparent)` },
  waiting: { color: ink.fgMuted, backgroundColor: `color-mix(in srgb, ${ink.fgMuted} 14%, transparent)` },
  'waiting-on-you': { color: tone.attention, backgroundColor: `color-mix(in srgb, ${tone.attention} 14%, transparent)` },
  stalled: { color: tone.attention, backgroundColor: `color-mix(in srgb, ${tone.attention} 14%, transparent)` },
  completed: { color: tone.done, backgroundColor: `color-mix(in srgb, ${tone.done} 14%, transparent)` },
  failed: { color: tone.dangerFg, backgroundColor: `color-mix(in srgb, ${tone.dangerFg} 14%, transparent)` },
  cancelled: { color: ink.fgMuted, backgroundColor: `color-mix(in srgb, ${ink.fgMuted} 14%, transparent)` },
})
