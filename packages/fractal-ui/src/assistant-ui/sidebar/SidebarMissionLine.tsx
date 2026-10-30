/**
 * Line 2 of an agent row that works on a mission (J2 · V3 · T1 · P2): `[mission pill] · step`, or
 * `[mission pill] · q143 Pick hover scope` while a question waits on you.
 *
 * The pill is the row's only box: 16px tall, fully rounded, starting at line 2's first pixel so every pill shares one x
 * and one height. The mission side holds at most 56% of the line and the slug truncates inside the pill, so the step
 * keeps the rest. Inside: the mission glyph, the slug and, when the run reports it, n/m. One of five hues, picked from the
 * mission name, so every row of a mission matches; only the glyph and a 14% wash take it, text stays on the ink tokens.
 * The decision handle is amber text, never boxed. Missions queued behind the current one sit behind a "+N" control that
 * opens a popover on press (touch too), never on hover alone.
 *
 * A Stale or Failed mission is the retained last observation: glyph, wash and text go muted, and the pill opens a
 * "Mission info" popover that says so, even with nothing queued. There is no retry: the host offers none.
 *
 * The pill and "+N" share one button on every layout, `[pill +N] · step`. Touch layouts grow the line-2
 * track to 44px (see `MilestoneAgentRow`); the visible pill stays 16px on every pointer type.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, DialogTrigger, VisuallyHidden } from 'react-aria-components'
import { Popover } from '../composition/Controls'
import { HumanClockProvider, HumanTime, fixedClock } from '../../time/HumanTime'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { missionHueVars as hue } from './mission-hue.stylex'
import type { SidebarMissionContext, SidebarMissionInfo } from './model'

type Mission = Extract<SidebarMissionContext, { _tag: 'Mission' }>
export const missionHues = ['teal', 'violet', 'pink', 'olive', 'cyan'] as const
export type MissionHue = (typeof missionHues)[number]
const missionHash = (slug: string) => [...slug].reduce((hash, char) => (hash * 31 + char.charCodeAt(0)) >>> 0, 7)
/** Stable per mission name: every row and every run of a mission gets the same hue and glyph. */
export const missionHue = (slug: string): MissionHue => missionHues[missionHash(slug) % missionHues.length]!
const live: SidebarMissionInfo = { _tag: 'Live' }
/** A queue or retained info gives the mission one shared popover target on every layout. */
export const missionLineHasTrigger = (mission: Mission, info: SidebarMissionInfo = live): boolean =>
  info._tag !== 'Live' || mission.queued.length > 0

type MarkShape = 'triangle' | 'square' | 'diamond'
const markShapes: readonly MarkShape[] = ['triangle', 'square', 'diamond']
const markPath: Readonly<Record<MarkShape, string>> = { diamond: 'M4 0.5 L7.5 4 L4 7.5 L0.5 4 Z', square: 'M1 1 H7 V7 H1 Z', triangle: 'M4 0.8 L7.5 7 H0.5 Z' }

/** The mission glyph: shape hashed from the slug (so hue is never the only cue), filled in the mission hue, or muted ink for retained info. */
function MissionMark({ slug, muted = false }: { readonly slug: string; readonly muted?: boolean }) {
  const shape = markShapes[missionHash(slug) % markShapes.length]!
  return <svg aria-hidden="true" data-mission-mark={shape} viewBox="0 0 8 8" width="8" height="8" {...stylex.props(styles.mark, muted ? styles.mutedMark : hueInk[missionHue(slug)])}><path d={markPath[shape]} fill="currentColor" /></svg>
}

/** Glyph + slug (+ n/m when Known). The slug truncates inside the pill; n/m never does. */
export function MissionPill({ mission, info = live }: { readonly mission: Pick<Mission, 'slug' | 'progress'>; readonly info?: SidebarMissionInfo }) {
  const { slug, progress } = mission
  const missionTone = missionHue(slug)
  const muted = info._tag !== 'Live'
  return <span data-box="pill" data-mission-pill={slug} data-mission-hue={missionTone} data-mission-info={info._tag.toLowerCase()} data-mission-updated-at={info._tag !== 'Live' ? info.updatedAt : undefined}
    {...stylex.props(styles.pill, muted ? styles.mutedPill : hueWash[missionTone])}>
    <MissionMark slug={slug} muted={muted} />
    <VisuallyHidden>Mission </VisuallyHidden>
    <span data-row-field="mission" {...stylex.props(styles.ellipsis)}>{slug}</span>
    {progress._tag === 'Known' && <>
      <span aria-hidden="true" data-mission-progress={`${progress.done}/${progress.total}`} {...stylex.props(styles.metric)}>{progress.done}/{progress.total}</span>
      <VisuallyHidden>, {progress.done} of {progress.total} steps</VisuallyHidden>
    </>}
  </span>
}

export interface SidebarMissionLineProps {
  readonly mission: Mission
  /** Absent: Live. Stale or Failed mutes the mission and opens the "Mission info" popover from the pill. */
  readonly info?: SidebarMissionInfo
  /** Touch layout (narrow host or coarse pointer): the pill and "+N" become one 44px-tall button. */
  readonly touch?: boolean
  /** Fixed instant for the popover's update time; omitted, it follows the shared context clock. */
  readonly now?: number
  /** Last-observed snapshot: the decision handle drops its live amber for neutral ink. */
  readonly frozen?: boolean
}

/** `[pill] · step`, `[pill] · q143 question`, `[pill] · question`, or the bare pill when the step is Unknown. */
export function SidebarMissionLine({ mission, info = live, touch = false, now, frozen = false }: SidebarMissionLineProps) {
  const { step, queued } = mission
  const muted = info._tag !== 'Live'
  const trigger = missionLineHasTrigger(mission, info)
  const merged = touch && trigger
  return <>
    <span data-line2-slot="mission" data-mission-info={info._tag.toLowerCase()} {...stylex.props(styles.missionSlot, merged && styles.missionSlotTouch)}>
      {trigger
        ? <MissionInfoTrigger mission={mission} info={info} queued={queued} touch={touch} now={now} />
        : <MissionPill mission={mission} info={info} />}
    </span>
    {step._tag !== 'Unknown' && <>
      <span aria-hidden="true" {...stylex.props(styles.sep)}>·</span>
      <span data-line2-slot="step" {...stylex.props(styles.stepSlot, muted && styles.mutedInk)}>
        {step._tag === 'Step'
          ? <span data-row-field="step" {...stylex.props(styles.ellipsis)}>{step.name}</span>
          : <>
            {step.handle !== undefined && <span data-q-handle={step.handle} {...stylex.props(styles.handle, frozen && styles.frozenInk, muted && styles.mutedInk)}><VisuallyHidden>Decision </VisuallyHidden>{step.handle}<VisuallyHidden> waiting on you: </VisuallyHidden></span>}
            <span data-row-field="question" {...stylex.props(styles.ellipsis)}>{step.handle === undefined && <VisuallyHidden>Waiting on you: </VisuallyHidden>}{step.question}</span>
          </>}
      </span>
    </>}
  </>
}

const queuedNoun = (count: number) => count === 1 ? 'mission' : 'missions'

/**
 * The pill as a button: on touch layouts it also carries "+N" and fills the 44px line-2 track, so the hit box is the
 * painted button box (no pseudo-element), stays inside its own row and ends before the trailing columns. Its name comes
 * from its content: the slug, progress, the queue and, for retained info, why it is muted.
 */
function MissionInfoTrigger({ mission, info, queued, touch, now }: { readonly mission: Mission; readonly info: SidebarMissionInfo; readonly queued: Mission['queued']; readonly touch: boolean; readonly now: number | undefined }) {
  const retained = info._tag !== 'Live'
  const clock = React.useMemo(() => now === undefined ? undefined : fixedClock(now), [now])
  const body = <MissionInfoBody info={info} queued={queued} />
  return <DialogTrigger>
    <Button data-mission-trigger={retained ? 'info' : 'queued'} data-queued-missions={queued.length > 0 ? queued.length : undefined} {...stylex.props(styles.trigger, touch && styles.triggerTouch)}>
      <MissionPill mission={mission} info={info} />
      {queued.length > 0 && <span data-queued-count={queued.length} {...stylex.props(styles.count)}><VisuallyHidden>, </VisuallyHidden>+{queued.length}<VisuallyHidden> more {queuedNoun(queued.length)} queued</VisuallyHidden></span>}
      {info._tag === 'Stale' && <VisuallyHidden>, mission info may be out of date</VisuallyHidden>}
      {info._tag === 'Failed' && <VisuallyHidden>, couldn’t refresh mission info</VisuallyHidden>}
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
    <MissionMark slug={item.slug} /><span {...stylex.props(styles.ellipsis)}>{item.slug}</span>
  </li>)}</ul>
}


const styles = stylex.create({
  // The mission side stops at 56% of line 2, so after the separator the step keeps the rest.
  missionSlot: { display: 'inline-flex', alignItems: 'center', flex: '0 1 auto', minWidth: 0, maxWidth: '56%' },
  // Touch: the slot is the full line-2 track, so its button box can be the 44px hit box.
  missionSlotTouch: { alignSelf: 'stretch' },
  stepSlot: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flex: '1 1 0', minWidth: 0, overflow: 'hidden' },
  sep: { flexShrink: 0, paddingInline: s.xs, color: ink.fgMuted },
  ellipsis: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  metric: { flexShrink: 0, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  pill: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, flex: '0 1 auto', minWidth: 0, maxWidth: '100%', height: '16px', paddingInline: s.sm, boxSizing: 'border-box',
    overflow: 'hidden', borderRadius: r.full, color: ink.fgSoft, lineHeight: '16px',
  },
  mutedPill: { color: ink.fgMuted, backgroundColor: `color-mix(in srgb, ${ink.fgMuted} 12%, transparent)` },
  mark: { flexShrink: 0 },
  mutedMark: { color: ink.fgMuted },
  // The pill's own box on fine pointers; the rounded focus ring follows it.
  trigger: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, maxWidth: '100%', margin: 0, padding: 0, borderWidth: 0, borderRadius: r.full,
    backgroundColor: surface.transparent, color: 'inherit', fontFamily: 'inherit', fontSize: 'inherit', lineHeight: 'inherit', cursor: 'pointer', outline: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '1px' },
  },
  // Touch: at least 44 × 44 and the whole line-2 track tall; the pill stays 16px, centred in it.
  triggerTouch: { minHeight: g.touchTarget, minWidth: g.touchTarget, borderRadius: r.sm },
  mutedInk: { color: ink.fgMuted },
  handle: { flexShrink: 0, color: tone.attention, fontFamily: t.fontMono, fontSize: t.microSize, fontWeight: t.weightMedium, whiteSpace: 'nowrap' },
  frozenInk: { color: ink.fgSoft },
  count: { flexShrink: 0, color: { default: ink.fgSoft, ':hover': ink.fg }, fontSize: t.denseSize, lineHeight: '16px', fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  queued: { outline: 'none', maxWidth: g.specimenAside, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
  note: { margin: 0, padding: s.sm, color: ink.fg },
  queuedList: { display: 'flex', flexDirection: 'column', gap: s.xs, margin: 0, padding: s.sm, listStyle: 'none', maxWidth: g.specimenAside },
  queuedItem: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, minHeight: g.controlSm },
})
const hueWash = stylex.create({
  teal: { backgroundColor: `color-mix(in srgb, ${hue.teal} 14%, transparent)` },
  violet: { backgroundColor: `color-mix(in srgb, ${hue.violet} 14%, transparent)` },
  pink: { backgroundColor: `color-mix(in srgb, ${hue.pink} 14%, transparent)` },
  olive: { backgroundColor: `color-mix(in srgb, ${hue.olive} 14%, transparent)` },
  cyan: { backgroundColor: `color-mix(in srgb, ${hue.cyan} 14%, transparent)` },
})
const hueInk = stylex.create({
  teal: { color: hue.teal },
  violet: { color: hue.violet },
  pink: { color: hue.pink },
  olive: { color: hue.olive },
  cyan: { color: hue.cyan },
})
