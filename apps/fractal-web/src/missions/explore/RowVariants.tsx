// EXPLORATION ONLY: three sparklines for the 48px Missions-tab row at 256px. Line 1 stays the shipped row's line 1
// (mark, title, time); line 2 is the sparkline plus elapsed against expected.
import * as React from 'react'

import { C, Mark } from './parts.tsx'
import { layered } from './layout.ts'
import { progressOf, span, stepEnd, stepStart, type ExploreMission, type PlanStep, type StepState } from './model.ts'

const cellColor = (state: StepState) => state === 'failed' ? C.bad : state === 'waiting-you' ? C.you : state === 'done' ? C.soft : state === 'running' ? C.fg : 'transparent'
const missionMark = (mission: ExploreMission): StepState => {
  const states = mission.steps.map(s => s.state)
  return states.includes('waiting-you') ? 'waiting-you' : states.includes('failed') ? 'failed' : states.includes('running') ? 'running' : states.every(s => s === 'done' || s === 'cancelled') ? 'done' : 'pending'
}
const live = (mission: ExploreMission) => mission.steps.filter(s => s.state !== 'cancelled')
const planOrder = (mission: ExploreMission): readonly PlanStep[] => {
  const layout = layered(live(mission))
  return [...live(mission)].sort((a, b) => layout.nodes.get(a.id)!.layer - layout.nodes.get(b.id)!.layer || layout.nodes.get(a.id)!.order - layout.nodes.get(b.id)!.order)
}
/** The strip's window: start to now (or the end, once done), never shorter than expected. */
const windowEnd = (mission: ExploreMission) => live(mission).every(s => s.state === 'done') ? Math.max(...mission.steps.map(s => stepEnd(s, mission.now) ?? 0)) : Math.max(mission.now, mission.startedAt + mission.expectedMs)
const elapsedText = (mission: ExploreMission) => {
  const done = live(mission).every(s => s.state === 'done')
  const end = done ? Math.max(...mission.steps.map(s => stepEnd(s, mission.now) ?? 0)) : mission.now
  return done ? span(end - mission.startedAt) : `${span(mission.now - mission.startedAt)} / ~${span(mission.expectedMs)}`
}

const Row = ({ mission, children, trailing }: { readonly mission: ExploreMission; readonly children: React.ReactNode; readonly trailing: string }) =>
  <div role="row" aria-label={`${mission.title}, ${progressOf(mission).done} of ${progressOf(mission).total} steps, ${trailing}`} style={{ display: 'grid', gridTemplateRows: '24px 20px', alignContent: 'center', height: 48, boxSizing: 'border-box', padding: '0 12px', color: C.fg, fontSize: 13, cursor: 'pointer' }}>
    <span style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
      <Mark state={missionMark(mission)} />
      <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{mission.title}</span>
    </span>
    <span style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0, paddingInlineStart: 18, fontSize: 11, color: C.soft }}>
      <span aria-hidden="true" style={{ flex: 1, minWidth: 0, display: 'flex', alignItems: 'center' }}>{children}</span>
      <span style={{ flexShrink: 0, fontVariantNumeric: 'tabular-nums' }}>{trailing}</span>
    </span>
  </div>

/** R1. Step strip: one equal cell per step in plan order. Reads as "how many, which are stuck". */
export const RowCells = ({ mission }: { readonly mission: ExploreMission }) =>
  <Row mission={mission} trailing={elapsedText(mission)}>
    <span style={{ display: 'flex', gap: 2, width: '100%', maxWidth: 140 }}>
      {planOrder(mission).map(step => <span key={step.id} title={`${step.title}: ${step.state}`} style={{ flex: 1, height: 8, borderRadius: 2, background: cellColor(step.state), boxShadow: step.state === 'pending' || step.state === 'ready' ? `inset 0 0 0 1px ${C.strong}` : undefined }} />)}
    </span>
  </Row>

/** R2. Time strip: the mission's wall clock squeezed onto one line; parallel work overlaps, waits hatch, failures tick red. */
export const RowTimeStrip = ({ mission }: { readonly mission: ExploreMission }) => {
  const t0 = mission.startedAt
  const t1 = windowEnd(mission)
  const pct = (t: number) => `${((t - t0) / (t1 - t0)) * 100}%`
  const attempts = live(mission).flatMap(step => step.attempts)
  const waits = live(mission).flatMap(step => step.waits)
  return <Row mission={mission} trailing={elapsedText(mission)}>
    <span style={{ position: 'relative', width: '100%', maxWidth: 150, height: 10, background: C.hover, borderRadius: 2, overflow: 'hidden' }}>
      {waits.map((w, i) => <span key={`w${i}`} style={{ position: 'absolute', left: pct(w.start), width: `calc(${pct(w.end ?? mission.now)} - ${pct(w.start)})`, top: 0, bottom: 0, backgroundImage: `repeating-linear-gradient(135deg, ${w.kind === 'you' ? C.you : C.muted} 0 1px, transparent 1px 3px)`, opacity: 0.8 }} />)}
      {attempts.map((a, i) => <span key={`a${i}`} style={{ position: 'absolute', left: pct(a.start), width: `max(1px, calc(${pct(a.end ?? mission.now)} - ${pct(a.start)}))`, top: 2, bottom: 2, background: a.outcome === 'failed' ? C.bad : a.outcome === 'running' ? C.fg : C.soft, opacity: 0.8 }} />)}
      <span style={{ position: 'absolute', left: pct(mission.now), top: 0, bottom: 0, width: 1, background: C.fg }} />
    </span>
  </Row>
}

/** R3. Mini trace: a few hairline lanes (parallel steps stack), the classic sparkline-of-a-Gantt. Densest, least legible. */
export const RowMiniTrace = ({ mission }: { readonly mission: ExploreMission }) => {
  const t0 = mission.startedAt
  const t1 = windowEnd(mission)
  const W = 150
  const x = (t: number) => ((t - t0) / (t1 - t0)) * W
  // Greedy lane packing of each step's active span.
  const spans = live(mission).map(step => ({ step, s: stepStart(step), e: stepEnd(step, mission.now) })).filter((v): v is { step: PlanStep; s: number; e: number } => v.s !== undefined && v.e !== undefined).sort((a, b) => a.s - b.s)
  const laneEnds: number[] = []
  const placed = spans.map(v => { let lane = laneEnds.findIndex(end => end <= v.s); if (lane === -1) { lane = laneEnds.length; laneEnds.push(0) } laneEnds[lane] = v.e; return { ...v, lane } })
  const lanes = Math.max(1, Math.min(4, laneEnds.length))
  return <Row mission={mission} trailing={elapsedText(mission)}>
    <svg width={W} height={12} viewBox={`0 0 ${W} 12`} style={{ maxWidth: '100%' }}>
      {placed.filter(p => p.lane < lanes).map(p => {
        const y = 1 + (p.lane * 10) / lanes + 10 / lanes / 2
        return <g key={p.step.id}>
          {p.step.waits.map((w, i) => <line key={i} x1={x(w.start)} x2={x(w.end ?? mission.now)} y1={y} y2={y} stroke={w.kind === 'you' ? C.you : C.muted} strokeWidth={1} strokeDasharray="1 2" />)}
          {p.step.attempts.map(a => <line key={a.n} x1={x(a.start)} x2={Math.max(x(a.start) + 1, x(a.end ?? mission.now))} y1={y} y2={y} stroke={a.outcome === 'failed' ? C.bad : a.outcome === 'running' ? C.fg : C.soft} strokeWidth={Math.max(1.5, 8 / lanes)} />)}
        </g>
      })}
      <line x1={x(mission.now)} x2={x(mission.now)} y1={0} y2={12} stroke={C.fg} strokeWidth={1} />
    </svg>
  </Row>
}

export const ROW_VARIANTS = { cells: RowCells, time: RowTimeStrip, trace: RowMiniTrace } as const
