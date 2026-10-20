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
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, DialogTrigger, VisuallyHidden } from 'react-aria-components'
import { Popover } from '../composition/Controls'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { missionHueVars as hue } from './mission-hue.stylex'
import type { SidebarMissionContext } from './model'

type Mission = Extract<SidebarMissionContext, { _tag: 'Mission' }>
export const missionHues = ['teal', 'violet', 'pink', 'olive', 'cyan'] as const
export type MissionHue = (typeof missionHues)[number]
const missionHash = (slug: string) => [...slug].reduce((hash, char) => (hash * 31 + char.charCodeAt(0)) >>> 0, 7)
/** Stable per mission name: every row and every run of a mission gets the same hue and glyph. */
export const missionHue = (slug: string): MissionHue => missionHues[missionHash(slug) % missionHues.length]!

type MarkShape = 'triangle' | 'square' | 'diamond'
const markShapes: readonly MarkShape[] = ['triangle', 'square', 'diamond']
const markPath: Readonly<Record<MarkShape, string>> = { diamond: 'M4 0.5 L7.5 4 L4 7.5 L0.5 4 Z', square: 'M1 1 H7 V7 H1 Z', triangle: 'M4 0.8 L7.5 7 H0.5 Z' }

/** The mission glyph: shape hashed from the slug (so hue is never the only cue), filled in the mission hue. */
function MissionMark({ slug }: { readonly slug: string }) {
  const shape = markShapes[missionHash(slug) % markShapes.length]!
  return <svg aria-hidden="true" data-mission-mark={shape} viewBox="0 0 8 8" width="8" height="8" {...stylex.props(styles.mark, hueInk[missionHue(slug)])}><path d={markPath[shape]} fill="currentColor" /></svg>
}

/** Glyph + slug (+ n/m when Known). The slug truncates inside the pill; n/m never does. */
export function MissionPill({ mission }: { readonly mission: Pick<Mission, 'slug' | 'progress'> }) {
  const { slug, progress } = mission
  const missionTone = missionHue(slug)
  return <span data-box="pill" data-mission-pill={slug} data-mission-hue={missionTone} {...stylex.props(styles.pill, hueWash[missionTone])}>
    <MissionMark slug={slug} />
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
  /** Last-observed snapshot: the decision handle drops its live amber for neutral ink. */
  readonly frozen?: boolean
}

/** `[pill] · step`, `[pill] · q143 question`, `[pill] · question`, or the bare pill when the step is Unknown. */
export function SidebarMissionLine({ mission, frozen = false }: SidebarMissionLineProps) {
  const { step, queued } = mission
  return <>
    <span data-line2-slot="mission" {...stylex.props(styles.missionSlot)}><MissionPill mission={mission} /></span>
    {step._tag !== 'Unknown' && <>
      <span aria-hidden="true" {...stylex.props(styles.sep)}>·</span>
      <span data-line2-slot="step" {...stylex.props(styles.stepSlot)}>
        {step._tag === 'Step'
          ? <span data-row-field="step" {...stylex.props(styles.ellipsis)}>{step.name}</span>
          : <>
            {step.handle !== undefined && <span data-q-handle={step.handle} {...stylex.props(styles.handle, frozen && styles.frozenInk)}><VisuallyHidden>Decision </VisuallyHidden>{step.handle}<VisuallyHidden> waiting on you: </VisuallyHidden></span>}
            <span data-row-field="question" {...stylex.props(styles.ellipsis)}>{step.handle === undefined && <VisuallyHidden>Waiting on you: </VisuallyHidden>}{step.question}</span>
          </>}
      </span>
    </>}
    {queued.length > 0 && <QueuedMissions queued={queued} />}
  </>
}

/** "+N": plain text, not a box; the focus ring is its only outline. On a phone or coarse pointer an invisible ::before grows the hit area to 44 × 44 without moving the text. */
function QueuedMissions({ queued }: { readonly queued: Mission['queued'] }) {
  const narrow = React.useContext(NarrowLayoutContext)
  const noun = queued.length === 1 ? 'mission' : 'missions'
  return <DialogTrigger>
    <Button data-queued-missions={queued.length} {...stylex.props(styles.more, narrow && styles.moreTouch)}>+{queued.length}<VisuallyHidden> more {noun} queued</VisuallyHidden></Button>
    <Popover placement="bottom end">
      <Dialog aria-label={`Queued ${noun}`} {...stylex.props(styles.queued)}>
        <ul {...stylex.props(styles.queuedList)}>{queued.map(item => <li key={item.id} data-queued-mission={item.slug} {...stylex.props(styles.queuedItem)}>
          <MissionMark slug={item.slug} /><span {...stylex.props(styles.ellipsis)}>{item.slug}</span>
        </li>)}</ul>
      </Dialog>
    </Popover>
  </DialogTrigger>
}

const styles = stylex.create({
  // The mission side stops at 56% of line 2, so after the separator the step keeps the rest.
  missionSlot: { display: 'inline-flex', alignItems: 'center', flex: '0 1 auto', minWidth: 0, maxWidth: '56%' },
  stepSlot: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flex: '1 1 0', minWidth: 0, overflow: 'hidden' },
  sep: { flexShrink: 0, paddingInline: s.xs, color: ink.fgMuted },
  ellipsis: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  metric: { flexShrink: 0, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  pill: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, flex: '0 1 auto', minWidth: 0, maxWidth: '100%', height: '16px', paddingInline: s.sm, boxSizing: 'border-box',
    overflow: 'hidden', borderRadius: r.full, color: ink.fgSoft, lineHeight: '16px',
  },
  mark: { flexShrink: 0 },
  handle: { flexShrink: 0, color: tone.attention, fontFamily: t.fontMono, fontSize: t.microSize, fontWeight: t.weightMedium, whiteSpace: 'nowrap' },
  frozenInk: { color: ink.fgSoft },
  more: {
    position: 'relative', flexShrink: 0, display: 'inline-flex', alignItems: 'center', height: '16px', marginInlineStart: 'auto', paddingInline: s.xs2, borderWidth: 0, borderRadius: r.keyboard,
    backgroundColor: surface.transparent, color: { default: ink.fgSoft, ':hover': ink.fg }, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: '16px', fontVariantNumeric: 'tabular-nums',
    cursor: 'pointer', outline: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '1px' },
    '::before': { content: '""', position: 'absolute', inset: { default: 0, '@media (pointer: coarse)': `calc(50% - ${g.touchTarget} / 2)` } },
  },
  moreTouch: { '::before': { content: '""', position: 'absolute', inset: `calc(50% - ${g.touchTarget} / 2)` } },
  queued: { outline: 'none', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
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
