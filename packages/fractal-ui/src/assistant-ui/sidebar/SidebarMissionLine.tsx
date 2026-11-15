/**
 * Line 2 of an agent row that works on a mission: `ring n/m · step · slug`, or `ring n/m · q143 Pick hover scope · slug` while a
 * question waits on you. The ring is a quiet 12px neutral stroke; the slug is muted and gives up its width before the step does.
 * Ad hoc work has a content hash for an id, which says nothing to a reader: its line is only `title ?? goal` (or the waiting
 * question), with no ring and no slug. A served ETA of an unfinished run is never drawn here: it rides along as the mission slot's
 * visually hidden description and is drawn on the Missions screen, so the line is identical with and without one.
 *
 * Nothing here is hued. The decision handle takes amber only for a real ask (`needsYou`, the set the counts use). Missions queued
 * behind the current one sit behind a "+N" control that opens a popover on press (touch too), never on hover alone.
 *
 * A Stale or Failed mission is the retained last observation: the ring goes dashed, text goes muted, and the ring opens a
 * "Mission info" popover that says so, even with nothing queued. There is no retry: the host offers none.
 *
 * Touch layouts grow the line-2 track to 44px for any button here (see `MilestoneAgentRow`); the visible ring stays 12px.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, DialogTrigger, VisuallyHidden } from 'react-aria-components'
import { Popover } from '../composition/Controls'
import { HumanClockProvider, HumanTime, fixedClock } from '../../time/HumanTime'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { MissionEtaLabel } from '../MissionEta'
import { isAdHocMissionSlug, type SidebarMissionContext, type SidebarMissionInfo, type SidebarMissionProgress, type SidebarOtherMission } from './model'

type Mission = Extract<SidebarMissionContext, { _tag: 'Mission' }>
const live: SidebarMissionInfo = { _tag: 'Live' }
/** A queue or retained info gives the mission one shared popover target on every layout. */
export const missionLineHasTrigger = (mission: Mission, info: SidebarMissionInfo = live): boolean =>
  info._tag !== 'Live' || mission.queued.length > 0

const ringBox = 12
const ringStroke = 1.5
const ringRadius = (ringBox - ringStroke) / 2
/** The progress mark: a thin neutral ring filling clockwise from 12 o'clock. `dashed`: a retained observation, not a live one. */
export function MissionRing({ done, total, dashed = false }: { readonly done: number; readonly total: number; readonly dashed?: boolean }) {
  const fraction = total > 0 ? Math.min(1, Math.max(0, done / total)) : 0
  const center = ringBox / 2
  return <svg aria-hidden="true" data-mission-ring={`${done}/${total}`} viewBox={`0 0 ${ringBox} ${ringBox}`} fill="none" stroke="currentColor" strokeWidth={ringStroke} {...stylex.props(styles.ring)}>
    <circle cx={center} cy={center} r={ringRadius} opacity={dashed ? '.55' : '.25'} strokeDasharray={dashed ? '1.5 1.5' : undefined} />
    {fraction > 0 && <circle cx={center} cy={center} r={ringRadius} pathLength="100" strokeDasharray={`${(fraction * 100).toFixed(2)} 100`} transform={`rotate(-90 ${center} ${center})`} />}
  </svg>
}

/** Ring and n/m when Known, never `0/0`; the retained dashed ring alone when the run is not hydrated. */
export function MissionProgressMark({ progress, retained = false }: { readonly progress: SidebarMissionProgress; readonly retained?: boolean }) {
  if (progress._tag !== 'Known') return retained ? <MissionRing done={0} total={0} dashed /> : null
  return <>
    <MissionRing done={progress.done} total={progress.total} dashed={retained} />
    <span aria-hidden="true" data-mission-progress={`${progress.done}/${progress.total}`} {...stylex.props(styles.metric)}>{progress.done}/{progress.total}</span>
    <VisuallyHidden>{progress.done} of {progress.total} steps</VisuallyHidden>
  </>
}

export interface SidebarMissionLineProps {
  readonly mission: Mission
  /** Absent: Live. Stale or Failed mutes the mission and opens the "Mission info" popover from the ring. */
  readonly info?: SidebarMissionInfo
  /** Touch layout (narrow host or coarse pointer): the ring and "+N" become one 44px-tall button. */
  readonly touch?: boolean
  /** Fixed instant for the popover's update time; omitted, it follows the shared context clock. */
  readonly now?: number
  /** Last-observed snapshot: the decision handle drops its live amber for neutral ink. */
  readonly frozen?: boolean
  /** A real ask on this row: the only thing that lets the decision handle take amber. */
  readonly needsYou?: boolean
  /** The title is already on the line above: ad hoc work does not repeat it. */
  readonly titleAbove?: boolean
  /** Makes the mission run of the line its own target, opening this mission ref. Omitted, the line is plain text. Pass a stable callback. */
  readonly onOpen?: (mission: string) => void
  readonly onOverlayOpenChange?: (open: boolean) => void
}

/**
 * `ring n/m · step · slug`, with a question in the step's place while one waits. Ad hoc work reads `title ?? goal`, never its hash.
 * With `onOpen` that whole run is one button, its own press target that opens the mission rather than the agent. A retained ring
 * stays outside it, as the "Mission info" target, and a live "+N" follows it.
 */
export function SidebarMissionLine({ mission, info = live, touch = false, now, frozen = false, needsYou = false, titleAbove = false, onOpen, onOverlayOpenChange }: SidebarMissionLineProps) {
  const { step, queued, slug } = mission
  const muted = info._tag !== 'Live'
  const adHoc = isAdHocMissionSlug(slug)
  const adHocText = adHoc && step._tag !== 'Waiting' && !titleAbove ? mission.title : undefined
  const triggerSlot = (mark: boolean) => <span data-line2-slot="mission" data-mission-info={info._tag.toLowerCase()} data-mission-updated-at={info._tag !== 'Live' ? info.updatedAt : undefined} {...stylex.props(styles.missionSlot, touch && styles.missionSlotTouch, muted && styles.mutedInk)}>
    <MissionInfoTrigger key={mission.id} mission={mission} adHoc={adHoc} mark={mark} info={info} queued={queued} touch={touch} now={now} onOpenChange={onOverlayOpenChange} />
  </span>
  const mainMark = !muted && !adHoc && mission.progress._tag === 'Known'
  const main = <>
    {mainMark && (queued.length > 0 || onOpen !== undefined
      ? <span data-line2-slot="progress" {...stylex.props(styles.missionSlot, styles.mark)}><MissionProgressMark progress={mission.progress} /></span>
      : <span data-line2-slot="mission" data-mission-info="live" {...stylex.props(styles.missionSlot, styles.mark)}><MissionProgressMark progress={mission.progress} /></span>)}
    <span data-line2-run {...stylex.props(styles.run)}>{adHocText !== undefined
      ? <span data-line2-slot="step" {...stylex.props(styles.stepSlot, muted && styles.mutedInk)}><span data-row-field="step" {...stylex.props(styles.ellipsis)}>{adHocText}</span></span>
      : step._tag !== 'Unknown' && !(adHoc && titleAbove && step._tag === 'Step') && <span data-line2-slot="step" {...stylex.props(styles.stepSlot, muted && styles.mutedInk)}>
        {step._tag === 'Step'
          ? <span data-row-field="step" {...stylex.props(styles.ellipsis)}>{step.name}</span>
          : <>
            {step.handle !== undefined && <span data-q-handle={step.handle} {...stylex.props(styles.handle, needsYou && !frozen && !muted && styles.handleAsk)}><VisuallyHidden>Decision </VisuallyHidden>{step.handle}<VisuallyHidden> waiting on you: </VisuallyHidden></span>}
            <span data-row-field="question" {...stylex.props(styles.ellipsis)}>{step.handle === undefined && <VisuallyHidden>Waiting on you: </VisuallyHidden>}{step.question}</span>
          </>}
      </span>}
    {!adHoc && <span data-line2-slot="slug" {...stylex.props(styles.slugSlot, muted && styles.mutedInk)}><VisuallyHidden>Mission </VisuallyHidden><span data-row-field="mission" data-mission={slug} {...stylex.props(styles.ellipsis)}>{slug}</span></span>}</span>
  </>
  return <>
    {muted && triggerSlot(true)}
    {onOpen === undefined ? main : <Button data-mission-open={mission.id} onPress={() => onOpen(mission.id)} {...stylex.props(styles.open, touch && styles.openTouch)}><VisuallyHidden>Open mission </VisuallyHidden>{main}</Button>}
    {/* Absolutely positioned and clipped: it adds no flex item, so the line never moves. */}
    <MissionEtaLabel eta={mission.eta} now={now} variant="description" />
    {!muted && queued.length > 0 && triggerSlot(false)}
  </>
}

/** The row's name, for its tree item: title first, then progress and what it is doing. Never a hash. */
export const missionChildAccessibleName = (mission: SidebarOtherMission): string => {
  const parts = [mission.title ?? (isAdHocMissionSlug(mission.slug) ? 'Untitled mission' : mission.slug), 'Mission']
  if (mission.progress._tag === 'Known') parts.push(`${mission.progress.done} of ${mission.progress.total} steps`)
  if (mission.step._tag === 'Step' && mission.step.name !== mission.title) parts.push(`Step ${mission.step.name}`)
  if (mission.step._tag === 'Waiting') parts.push(`Waiting for you: ${mission.step.question}`)
  return parts.join(', ')
}

/**
 * A further current mission of an agent, as its own 48px tree row indented under the agent: title, then the same
 * `ring n/m · step · slug` line. It carries no state mark of its own, which belongs to the agent. Activation belongs to the tree item.
 */
export function SidebarMissionChildRow({ mission, active = false, frozen = false, retained = false, ...rowProps }: { readonly mission: SidebarOtherMission; readonly active?: boolean; readonly frozen?: boolean; readonly retained?: boolean } & Omit<React.HTMLAttributes<HTMLDivElement>, 'className' | 'style'>) {
  const title = mission.title ?? (isAdHocMissionSlug(mission.slug) ? 'Untitled mission' : mission.slug)
  return <div {...rowProps} data-mission-child={mission.id} data-frozen={frozen || undefined} {...stylex.props(styles.childRow, active && styles.childActive, (retained || frozen) && styles.mutedInk)}>
    <span data-row-column="title" {...stylex.props(styles.childTitle)}>{title}</span>
    <span data-row-column="line2" {...stylex.props(styles.childLine)}><SidebarMissionLine mission={{ _tag: 'Mission', ...mission, title, queued: [] }} frozen={frozen} titleAbove /></span>
  </div>
}

const queuedNoun = (count: number) => count === 1 ? 'mission' : 'missions'

/**
 * The "+N" or retained ring as a button: on touch layouts it fills the 44px line-2 track, so the hit box is the painted button box (no
 * pseudo-element), stays inside its own row and ends before the trailing columns. Its name comes from its content: progress,
 * the queue and, for retained info, why it is muted.
 */
function MissionInfoTrigger({ mission, adHoc, mark, info, queued, touch, now, onOpenChange }: { readonly mission: Mission; readonly adHoc: boolean; readonly mark: boolean; readonly info: SidebarMissionInfo; readonly queued: Mission['queued']; readonly touch: boolean; readonly now: number | undefined; readonly onOpenChange?: (open: boolean) => void }) {
  const lifecycle = React.useRef({ open: false, onOpenChange })
  lifecycle.current.onOpenChange = onOpenChange
  React.useEffect(() => () => {
    if (lifecycle.current.open) lifecycle.current.onOpenChange?.(false)
  }, [])
  const retained = info._tag !== 'Live'
  const hasProgress = mark && !adHoc && mission.progress._tag === 'Known'
  const clock = React.useMemo(() => now === undefined ? undefined : fixedClock(now), [now])
  const body = <MissionInfoBody info={info} queued={queued} />
  return <DialogTrigger onOpenChange={open => {
    lifecycle.current.open = open
    onOpenChange?.(open)
  }}>
    <Button data-mission-trigger={retained ? 'info' : 'queued'} data-queued-missions={queued.length > 0 ? queued.length : undefined} {...stylex.props(styles.trigger, touch && styles.triggerTouch)}>
      {mark && <MissionProgressMark progress={adHoc ? { _tag: 'Unknown' } : mission.progress} retained={retained} />}
      {queued.length > 0 && <span data-queued-count={queued.length} {...stylex.props(styles.count)}>{hasProgress && <VisuallyHidden>, </VisuallyHidden>}+{queued.length}<VisuallyHidden> more {queuedNoun(queued.length)} queued</VisuallyHidden></span>}
      {info._tag === 'Stale' && <VisuallyHidden>{hasProgress || queued.length > 0 ? ', ' : ''}mission info may be out of date</VisuallyHidden>}
      {info._tag === 'Failed' && <VisuallyHidden>{hasProgress || queued.length > 0 ? ', ' : ''}couldn’t refresh mission info</VisuallyHidden>}
    </Button>
    <Popover placement="bottom start">
      <Dialog aria-label={retained ? 'Mission info' : `Queued ${queuedNoun(queued.length)}`} data-mission-info-dialog={info._tag.toLowerCase()} {...stylex.props(styles.queued)}>
        {clock === undefined ? body : <HumanClockProvider clock={clock}>{body}</HumanClockProvider>}
      </Dialog>
    </Popover>
  </DialogTrigger>
}

/** Retained info says why, with its real update time when reported (never an invented one), then any queue. */
function MissionInfoBody({ info, queued }: { readonly info: SidebarMissionInfo; readonly queued: Mission['queued'] }) {
  const updated = info._tag !== 'Live' && info.updatedAt !== undefined
    ? <> Updated <HumanTime at={info.updatedAt} kind="elapsed" maxUnits={1} suffix=" ago" data-testid="mission-info-updated" />.</>
    : null
  return <>
    {info._tag === 'Stale' && <p data-mission-info-note="stale" {...stylex.props(styles.note)}>Mission info may be out of date.{updated}</p>}
    {info._tag === 'Failed' && <p data-mission-info-note="failed" {...stylex.props(styles.note)}>Couldn’t refresh mission info.{updated}</p>}
    {queued.length > 0 && <QueuedList queued={queued} />}
  </>
}

function QueuedList({ queued }: { readonly queued: Mission['queued'] }) {
  return <ul aria-label={`Queued ${queuedNoun(queued.length)}`} {...stylex.props(styles.queuedList)}>{queued.map(item => <li key={item.id} data-queued-mission={item.slug} {...stylex.props(styles.queuedItem)}>
    <span {...stylex.props(styles.ellipsis)}>{item.title ?? item.slug}</span>
  </li>)}</ul>
}

const styles = stylex.create({
  childRow: { display: 'grid', gridTemplateRows: `${g.controlSm} ${g.controlSm}`, alignItems: 'center', height: g.sidebarRow, minWidth: 0, width: '100%', boxSizing: 'border-box', paddingInline: s.sm, borderRadius: r.sm, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, ':hover': { backgroundColor: surface.rowHover } },
  childActive: { backgroundColor: surface.rowActive },
  childTitle: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fg, fontSize: t.uiSize, lineHeight: t.uiLeading },
  childLine: { display: 'flex', alignItems: 'center', minWidth: 0, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  // The ring and n/m never shrink; the step takes what is left, and the slug gives up its width first.
  missionSlot: { display: 'inline-flex', alignItems: 'center', flex: '0 0 auto', minWidth: 0, marginInlineEnd: s.sm },
  // Touch: the slot is the full line-2 track, so its button box can be the 44px hit box.
  missionSlotTouch: { alignSelf: 'stretch' },
  mark: { display: 'inline-flex', alignItems: 'center', gap: s.xs },
  stepSlot: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flex: '0 1 auto', minWidth: 0, maxWidth: '100%', height: g.controlSm, overflow: 'hidden' },
  // Step then slug on one 24px line that wraps: the slug claims a line only when 48px of it fits beside the step, and
  // otherwise wraps out of sight instead of showing a stub. The step always keeps the first line.
  run: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', alignContent: 'flex-start', flex: '1 1 0', minWidth: 0, height: g.controlSm, overflow: 'hidden' },
  slugSlot: { display: 'inline-flex', alignItems: 'center', flex: '1 1 48px', minWidth: '48px', maxWidth: 'max-content', height: g.controlSm, overflow: 'hidden', paddingInlineStart: s.sm, boxSizing: 'border-box', color: ink.fgSoft },
  ellipsis: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  metric: { flexShrink: 0, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  ring: { display: 'block', width: `${ringBox}px`, height: `${ringBox}px`, flexShrink: 0 },
  // The mission run as one target: as wide as its content, so the rest of the line still selects the agent. Hover draws its box.
  open: {
    display: 'inline-flex', alignItems: 'center', flex: '0 1 auto', minWidth: 0, margin: 0, padding: 0, paddingInline: s.xs, marginInlineStart: `calc(-1 * ${s.xs})`, height: g.controlSm, boxSizing: 'border-box', borderWidth: 0, borderRadius: r.sm,
    backgroundColor: { default: surface.transparent, ':is(:hover, [data-hovered])': surface.rowActive }, color: { default: 'inherit', ':is(:hover, [data-hovered])': ink.fg }, fontFamily: 'inherit', fontSize: 'inherit', lineHeight: 'inherit', cursor: 'pointer', outline: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` },
  },
  openTouch: { height: g.touchTarget },
  // The mark's own box on fine pointers; the rounded focus ring follows it.
  trigger: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, maxWidth: '100%', margin: 0, padding: 0, borderWidth: 0, borderRadius: r.sm,
    backgroundColor: surface.transparent, color: 'inherit', fontFamily: 'inherit', fontSize: 'inherit', lineHeight: 'inherit', cursor: 'pointer', outline: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '1px' },
  },
  // Touch: at least 44 × 44 and the whole line-2 track tall; the ring stays 12px, centred in it.
  triggerTouch: { minHeight: g.touchTarget, minWidth: g.touchTarget, justifyContent: 'center' },
  mutedInk: { color: ink.fgMuted },
  handle: { flexShrink: 0, color: ink.fgSoft, fontFamily: t.fontMono, fontSize: t.microSize, fontWeight: t.weightMedium, whiteSpace: 'nowrap' },
  handleAsk: { color: tone.attention },
  count: { flexShrink: 0, color: { default: ink.fgSoft, ':hover': ink.fg }, fontSize: t.denseSize, lineHeight: '16px', fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  queued: { outline: 'none', maxWidth: g.specimenAside, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
  note: { margin: 0, padding: s.sm, color: ink.fg },
  queuedList: { display: 'flex', flexDirection: 'column', gap: s.xs, margin: 0, padding: s.sm, listStyle: 'none', maxWidth: g.specimenAside },
  queuedItem: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, minHeight: g.controlSm },
})
