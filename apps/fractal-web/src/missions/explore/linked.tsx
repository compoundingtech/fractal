// EXPLORATION ONLY: the graph and the trace of variant B, refined with the interactions the interview chose:
// linked hover (dependencies light up), a time brush on the trace (steps that did not run in it dim), a replay cursor
// (the graph shows each step's state at that moment), and one step hover card anchored wherever the pointer is.
import * as React from 'react'

import { HoverCardSurface } from '../../../../../packages/fractal-ui/src/assistant-ui/entity/HoverCard.tsx'
import { C, Mark, SmallButton, STATE_LABEL } from './parts.tsx'
import { layered } from '../detail/layout.ts'
import { clock, criticalPath, lineage, ranIn, runningMs, span, stateAt, stepEnd, stepStart, type ExploreMission, type PlanStep, type StepState } from './model.ts'

/** Everything the linked views share. One instance per detail view. */
export interface Linked {
  readonly selected: string | undefined
  readonly select: (id: string) => void
  readonly hovered: string | undefined
  /** Hover a step from an element; the card anchors to it after the shared hover delay. */
  readonly hover: (id: string | undefined, anchor?: Element | null) => void
  readonly brush: readonly [number, number] | undefined
  readonly setBrush: (range: readonly [number, number] | undefined) => void
  /** Replay time; undefined shows now. */
  readonly cursor: number | undefined
  readonly setCursor: (t: number | undefined) => void
}

export const useLinked = (initial?: string, initialCursor?: number): Linked & { readonly anchor: Element | null; readonly cardOpen: boolean; readonly keepCard: (open: boolean) => void } => {
  const [selected, select] = React.useState<string | undefined>(initial)
  const [hovered, setHovered] = React.useState<string | undefined>()
  const [anchor, setAnchor] = React.useState<Element | null>(null)
  const [cardOpen, setCardOpen] = React.useState(false)
  const [brush, setBrush] = React.useState<readonly [number, number] | undefined>()
  const [cursor, setCursor] = React.useState<number | undefined>(initialCursor)
  const timer = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const hover = React.useCallback((id: string | undefined, element?: Element | null) => {
    clearTimeout(timer.current)
    if (id === undefined) { timer.current = setTimeout(() => { setHovered(undefined); setCardOpen(false) }, 300); return }
    setHovered(id)
    if (element !== undefined && element !== null) setAnchor(element)
    timer.current = setTimeout(() => setCardOpen(true), 150)
  }, [])
  return { selected, select, hovered, hover, brush, setBrush, cursor, setCursor, anchor, cardOpen, keepCard: open => { clearTimeout(timer.current); if (!open) hover(undefined) } }
}

/** What the views draw for a step: its state now, or at the replay cursor. */
const shownState = (mission: ExploreMission, step: PlanStep, cursor: number | undefined): StepState => cursor === undefined ? step.state : stateAt(step, cursor, mission.now)

/** Steps to dim: outside the hovered lineage, or not run inside the brush. */
const dimmedSet = (mission: ExploreMission, linked: Linked): ReadonlySet<string> => {
  const out = new Set<string>()
  const lit = linked.hovered === undefined ? undefined : lineage(mission.steps, linked.hovered)
  for (const step of mission.steps) {
    if (lit !== undefined && !lit.has(step.id)) out.add(step.id)
    if (linked.brush !== undefined && !ranIn(step, linked.brush[0], linked.brush[1], mission.now)) out.add(step.id)
  }
  return out
}

// ---------------------------------------------------------------------------------------------------------------------

const NODE_W = 184
const NODE_H = 46
const GAP_X = 56
const GAP_Y = 18
const agentOf = (step: PlanStep) => step.attempts.at(-1)?.agent ?? 'unassigned'

export const LinkedDag = ({ mission, linked, height = 300 }: { readonly mission: ExploreMission; readonly linked: Linked; readonly height?: number }) => {
  const steps = mission.steps.filter(step => step.state !== 'cancelled')
  const layout = React.useMemo(() => layered(steps), [mission])
  const critical = React.useMemo(() => criticalPath(mission), [mission])
  const [showCritical, setShowCritical] = React.useState(false)
  const dim = dimmedSet(mission, linked)
  const lit = linked.hovered === undefined ? undefined : lineage(mission.steps, linked.hovered)
  const pos = (id: string) => { const p = layout.nodes.get(id)!; return { x: 24 + p.layer * (NODE_W + GAP_X), y: 12 + p.order * (NODE_H + GAP_Y) } }
  const width = 48 + layout.layers * (NODE_W + GAP_X)
  const contentH = 24 + layout.widest * (NODE_H + GAP_Y)
  const [view, setView] = React.useState({ x: 0, y: 0, k: 1 })
  const drag = React.useRef<{ x: number; y: number; vx: number; vy: number } | undefined>(undefined)
  const svgRef = React.useRef<SVGSVGElement>(null)
  React.useEffect(() => {
    const box = svgRef.current?.getBoundingClientRect()
    if (box === undefined || box.width === 0) return
    const k = Math.min(1, box.width / width, height / contentH)
    setView({ x: (box.width - width * k) / 2, y: Math.max(0, (height - contentH * k) / 2), k })
  }, [width, contentH, height])
  const zoom = (factor: number, cx = (svgRef.current?.getBoundingClientRect().width ?? 0) / 2, cy = height / 2) =>
    setView(v => { const k = Math.min(2.5, Math.max(0.3, v.k * factor)); return { k, x: cx - (cx - v.x) * (k / v.k), y: cy - (cy - v.y) * (k / v.k) } })
  const onKey = (event: React.KeyboardEvent) => {
    const here = layout.nodes.get(linked.selected ?? steps[0]!.id)
    if (here === undefined) return
    const nodes = [...layout.nodes.values()]
    const pick = (layer: number) => nodes.filter(n => n.layer === layer).sort((a, b) => Math.abs(a.order - here.order) - Math.abs(b.order - here.order))[0]?.id
    const target = event.key === 'ArrowRight' ? pick(here.layer + 1) : event.key === 'ArrowLeft' ? pick(here.layer - 1)
      : event.key === 'ArrowDown' ? nodes.find(n => n.layer === here.layer && n.order === here.order + 1)?.id
        : event.key === 'ArrowUp' ? nodes.find(n => n.layer === here.layer && n.order === here.order - 1)?.id : undefined
    if (target !== undefined) { event.preventDefault(); linked.select(target); linked.hover(target, svgRef.current?.querySelector(`[data-dag-node="${target}"]`)) }
    if (event.key === 'Escape') linked.hover(undefined)
    if (event.key === '+' || event.key === '=') zoom(1.2)
    if (event.key === '-') zoom(1 / 1.2)
  }
  return <div style={{ position: 'relative', border: `1px solid ${C.line}`, borderRadius: 8, overflow: 'hidden', background: C.canvas }}>
    <div style={{ position: 'absolute', top: 8, right: 8, zIndex: 1, display: 'flex', gap: 4, alignItems: 'center' }}>
      {linked.cursor !== undefined ? <span style={{ fontSize: 11, color: C.fg, marginInlineEnd: 6 }}>Replaying {clock(linked.cursor)}</span> : null}
      <label style={{ display: 'inline-flex', alignItems: 'center', gap: 4, fontSize: 11, color: C.soft, marginInlineEnd: 6 }}><input type="checkbox" checked={showCritical} onChange={event => setShowCritical(event.target.checked)} /> Critical path</label>
      <SmallButton label="Zoom out" onPress={() => zoom(1 / 1.2)}>−</SmallButton>
      <SmallButton label="Zoom in" onPress={() => zoom(1.2)}>+</SmallButton>
    </div>
    <svg ref={svgRef} role="application" aria-roledescription="dependency graph" aria-label={`${mission.title} plan, ${steps.length} steps. Arrow keys move between steps, plus and minus zoom.`} tabIndex={0} onKeyDown={onKey}
      width="100%" height={height}
      onWheel={event => { event.preventDefault(); const box = svgRef.current!.getBoundingClientRect(); zoom(event.deltaY < 0 ? 1.1 : 1 / 1.1, event.clientX - box.left, event.clientY - box.top) }}
      onPointerDown={event => { if ((event.target as Element).closest('[data-dag-node]')) return; drag.current = { x: event.clientX, y: event.clientY, vx: view.x, vy: view.y }; (event.currentTarget as Element).setPointerCapture(event.pointerId) }}
      onPointerMove={event => { const d = drag.current; if (d !== undefined) setView(v => ({ ...v, x: d.vx + event.clientX - d.x, y: d.vy + event.clientY - d.y })) }}
      onPointerUp={() => { drag.current = undefined }}
      style={{ display: 'block', cursor: 'grab', outline: 'none', touchAction: 'none' }}>
      <defs><marker id="arrow-b" viewBox="0 0 6 6" refX="6" refY="3" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0 L6 3 L0 6 z" fill={C.strong} /></marker></defs>
      <g transform={`translate(${view.x} ${view.y}) scale(${view.k})`}>
        {steps.flatMap(step => step.after.filter(dep => layout.nodes.has(dep)).map(dep => {
          const a = pos(dep)
          const b = pos(step.id)
          const x1 = a.x + NODE_W, y1 = a.y + NODE_H / 2, x2 = b.x, y2 = b.y + NODE_H / 2, mid = (x1 + x2) / 2
          const on = lit !== undefined && lit.has(dep) && lit.has(step.id)
          const crit = showCritical && critical.has(dep) && critical.has(step.id)
          return <path key={`${dep}->${step.id}`} data-dag-edge={`${dep}->${step.id}`} data-lit={on || undefined} d={`M${x1} ${y1} C${mid} ${y1} ${mid} ${y2} ${x2} ${y2}`} fill="none" stroke={on ? C.fg : C.strong} strokeWidth={on || crit ? 2.25 : 1.25} strokeOpacity={lit !== undefined && !on ? 0.25 : 0.85} markerEnd="url(#arrow-b)" />
        }))}
        {steps.map(step => {
          const { x, y } = pos(step.id)
          const state = shownState(mission, step, linked.cursor)
          const isSel = linked.selected === step.id
          const ran = linked.cursor === undefined ? runningMs(step, mission.now) : step.attempts.reduce((sum, a) => sum + Math.max(0, Math.min(a.end ?? mission.now, linked.cursor!) - a.start), 0)
          const stroke = state === 'waiting-you' ? C.you : state === 'failed' ? C.bad : isSel || linked.hovered === step.id ? C.fg : C.strong
          return <g key={step.id} data-dag-node={step.id} data-state={state} data-dimmed={dim.has(step.id) || undefined} role="button" aria-pressed={isSel} aria-label={`${step.title}, ${STATE_LABEL[state]}`}
            transform={`translate(${x} ${y})`} opacity={dim.has(step.id) ? 0.3 : 1} style={{ cursor: 'pointer', transition: 'opacity 120ms' }}
            onClick={() => linked.select(step.id)} onPointerEnter={event => linked.hover(step.id, event.currentTarget)} onPointerLeave={() => linked.hover(undefined)}>
            <rect width={NODE_W} height={NODE_H} rx={7} fill={isSel ? C.active : C.canvas} stroke={stroke} strokeWidth={stroke === C.strong ? 1 : 1.75} />
            {showCritical && critical.has(step.id) ? <rect width={3} height={NODE_H} rx={1.5} fill={C.fg} /> : null}
            <foreignObject x={8} y={6} width={NODE_W - 14} height={NODE_H - 10}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2, fontSize: 12, lineHeight: '16px', color: C.fg }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}><Mark state={state} /><span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{step.title}</span></span>
                <span style={{ fontSize: 11, color: C.muted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {state === 'pending' ? (linked.cursor === undefined ? `~${span(step.expectedMs ?? 0)}` : 'not started') : `${span(ran)}${step.attempts.length > 1 ? ` · ${step.attempts.length} tries` : ''}`} · {agentOf(step)}
                </span>
              </div>
            </foreignObject>
          </g>
        })}
      </g>
    </svg>
  </div>
}

// ---------------------------------------------------------------------------------------------------------------------

const ZOOMS = [{ id: 'all', label: 'All', ms: 0 }, { id: '12h', label: '12h', ms: 12 * 3_600_000 }, { id: '2h', label: '2h', ms: 2 * 3_600_000 }] as const

export const LinkedTrace = ({ mission, linked, rowHeight = 22, labelWidth = 190 }: { readonly mission: ExploreMission; readonly linked: Linked; readonly rowHeight?: number; readonly labelWidth?: number }) => {
  const [zoomId, setZoom] = React.useState<(typeof ZOOMS)[number]['id']>('all')
  const layout = React.useMemo(() => layered(mission.steps), [mission])
  const ordered = [...mission.steps].filter(s => s.state !== 'cancelled').sort((a, b) => layout.nodes.get(a.id)!.layer - layout.nodes.get(b.id)!.layer || (stepStart(a) ?? Infinity) - (stepStart(b) ?? Infinity))
  const dim = dimmedSet(mission, linked)
  const projectedEnd = new Map<string, number>()
  const projectedStart = new Map<string, number>()
  const endOf = (step: PlanStep): number => {
    const known = projectedEnd.get(step.id)
    if (known !== undefined) return known
    if (step.state === 'done' || step.state === 'failed') { const end = stepEnd(step, mission.now) ?? mission.now; projectedEnd.set(step.id, end); return end }
    const deps = step.after.map(d => mission.steps.find(s => s.id === d)).filter((s): s is PlanStep => s !== undefined && s.state !== 'cancelled')
    const start = step.state === 'pending' ? Math.max(mission.now, ...deps.map(endOf)) : stepStart(step) ?? mission.now
    const end = Math.max(mission.now, start + (step.expectedMs ?? 0) - (step.state === 'pending' ? 0 : runningMs(step, mission.now)))
    projectedStart.set(step.id, start)
    projectedEnd.set(step.id, end)
    return end
  }
  ordered.forEach(endOf)
  const t0 = mission.startedAt
  const t1 = Math.max(mission.now, ...projectedEnd.values()) + 30 * 60_000
  const zoom = ZOOMS.find(z => z.id === zoomId)!
  const scrollRef = React.useRef<HTMLDivElement>(null)
  const [width, setWidth] = React.useState(800)
  React.useEffect(() => {
    const element = scrollRef.current
    if (element === null) return
    const observer = new ResizeObserver(() => setWidth(element.clientWidth - labelWidth))
    observer.observe(element)
    return () => observer.disconnect()
  }, [labelWidth])
  const pxPerMs = zoom.ms === 0 ? Math.max(1, width) / (t1 - t0) : Math.max(1, width) / zoom.ms
  const total = Math.max(width, (t1 - t0) * pxPerMs)
  const x = (t: number) => (t - t0) * pxPerMs
  const tAt = (px: number) => Math.min(t1, Math.max(t0, t0 + px / pxPerMs))
  React.useEffect(() => { if (zoom.ms > 0) scrollRef.current?.scrollTo({ left: Math.max(0, x(linked.cursor ?? mission.now) - width * 0.7) }) }, [zoomId])
  const tickEvery = zoom.ms === 0 ? 4 * 3_600_000 : zoom.ms > 6 * 3_600_000 ? 3_600_000 : 15 * 60_000
  const ticks: number[] = []
  for (let t = Math.ceil(t0 / tickEvery) * tickEvery; t < t1; t += tickEvery) ticks.push(t)
  // Brush: drag across the time axis. A click without a drag clears it.
  const brushing = React.useRef<number | undefined>(undefined)
  const axisX = (event: React.PointerEvent) => event.clientX - (event.currentTarget as HTMLElement).getBoundingClientRect().left
  const cursor = linked.cursor
  return <div style={{ border: `1px solid ${C.line}`, borderRadius: 8, overflow: 'hidden' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 8px', borderBottom: `1px solid ${C.line}`, fontSize: 11, color: C.soft, flexWrap: 'wrap' }}>
      <span>Zoom</span>{ZOOMS.map(z => <SmallButton key={z.id} label={`Zoom to ${z.label}`} pressed={z.id === zoomId} onPress={() => setZoom(z.id)}>{z.label}</SmallButton>)}
      <span style={{ marginInlineStart: 8 }}>Drag the time axis to brush a range.</span>
      {linked.brush !== undefined ? <span data-trace-brush-label style={{ color: C.fg }}>{clock(linked.brush[0])} – {clock(linked.brush[1])} <SmallButton label="Clear range" onPress={() => linked.setBrush(undefined)}>×</SmallButton></span> : null}
    </div>
    <div ref={scrollRef} style={{ overflowX: 'auto', position: 'relative' }}>
      <div role="grid" aria-label={`${mission.title} trace`} style={{ position: 'relative', width: labelWidth + total, fontSize: 12, userSelect: 'none' }}>
        <div role="row" style={{ display: 'flex', height: 22, borderBottom: `1px solid ${C.line}` }}>
          <div style={{ width: labelWidth, flexShrink: 0, position: 'sticky', left: 0, background: C.canvas, zIndex: 2 }} />
          <div data-trace-axis aria-label="Time axis: drag to select a range" style={{ position: 'relative', flex: 1, cursor: 'col-resize', background: C.hover }}
            onPointerDown={event => { brushing.current = tAt(axisX(event)); (event.currentTarget as Element).setPointerCapture(event.pointerId) }}
            onPointerMove={event => { const a = brushing.current; if (a === undefined) return; const b = tAt(axisX(event)); if (Math.abs(x(b) - x(a)) > 4) linked.setBrush(a < b ? [a, b] : [b, a]) }}
            onPointerUp={event => { const a = brushing.current; brushing.current = undefined; if (a !== undefined && Math.abs(x(tAt(axisX(event))) - x(a)) <= 4) linked.setBrush(undefined) }}>
            {ticks.map(t => <span key={t} style={{ position: 'absolute', left: x(t) + 3, top: 4, fontSize: 10, color: C.muted, whiteSpace: 'nowrap', pointerEvents: 'none' }}>{clock(t)}</span>)}
          </div>
        </div>
        {ordered.map(step => {
          const isSel = linked.selected === step.id
          const isHover = linked.hovered === step.id
          const state = shownState(mission, step, cursor)
          return <div key={step.id} role="row" aria-selected={isSel} data-trace-row={step.id} data-dimmed={dim.has(step.id) || undefined}
            onClick={() => linked.select(step.id)} onPointerEnter={event => linked.hover(step.id, event.currentTarget.querySelector('[role="rowheader"]'))} onPointerLeave={() => linked.hover(undefined)}
            style={{ display: 'flex', height: rowHeight, alignItems: 'center', cursor: 'pointer', background: isSel ? C.active : isHover ? C.hover : 'transparent', opacity: dim.has(step.id) ? 0.3 : 1, transition: 'opacity 120ms' }}>
            <div role="rowheader" style={{ width: labelWidth, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 6, paddingInline: 8, position: 'sticky', left: 0, background: isSel ? C.active : C.canvas, zIndex: 1, height: '100%' }}>
              <Mark state={state} /><button type="button" onFocus={event => linked.hover(step.id, event.currentTarget)} onBlur={() => linked.hover(undefined)} onClick={() => linked.select(step.id)} style={{ border: 0, background: 'transparent', color: C.fg, font: 'inherit', padding: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', cursor: 'pointer', textAlign: 'start' }}>{step.title}</button>
            </div>
            <div role="gridcell" style={{ position: 'relative', flex: 1, height: '100%' }}>
              {ticks.map(t => <span key={t} aria-hidden="true" style={{ position: 'absolute', left: x(t), top: 0, bottom: 0, width: 1, background: C.line, opacity: 0.5 }} />)}
              {step.waits.map((w, i) => <span key={`w${i}`} style={{ position: 'absolute', left: x(w.start), width: Math.max(2, x(w.end ?? mission.now) - x(w.start)), top: rowHeight * 0.3, height: rowHeight * 0.4, backgroundImage: `repeating-linear-gradient(135deg, ${w.kind === 'you' ? C.you : C.muted} 0 1.5px, transparent 1.5px 5px)`, opacity: w.kind === 'you' ? 0.9 : 0.5, borderRadius: 2 }} />)}
              {step.attempts.map(a => {
                const left = x(a.start)
                const w = Math.max(3, x(a.end ?? mission.now) - left)
                const failed = a.outcome === 'failed'
                return <span key={a.n} style={{ position: 'absolute', left, width: w, top: rowHeight * 0.2, height: rowHeight * 0.6, background: failed ? C.bad : a.outcome === 'running' ? C.fg : C.soft, borderRadius: 3, display: 'flex', alignItems: 'center', overflow: 'hidden' }}>
                  {w > 70 ? <span style={{ fontSize: 10, color: C.canvas, paddingInline: 4, whiteSpace: 'nowrap' }}>{failed ? `✕ ${a.reason ?? ''}` : a.agent}</span> : null}
                </span>
              })}
              {step.state === 'pending' && projectedStart.has(step.id) ? <span style={{ position: 'absolute', left: x(projectedStart.get(step.id)!), width: Math.max(3, x(projectedEnd.get(step.id)!) - x(projectedStart.get(step.id)!)), top: rowHeight * 0.25, height: rowHeight * 0.5, border: `1px dashed ${C.muted}`, borderRadius: 3 }} /> : null}
            </div>
          </div>
        })}
        {linked.brush !== undefined ? <span aria-hidden="true" style={{ position: 'absolute', top: 0, bottom: 0, left: labelWidth + x(linked.brush[0]), width: x(linked.brush[1]) - x(linked.brush[0]), background: C.fg, opacity: 0.06, border: `1px solid ${C.strong}`, pointerEvents: 'none' }} /> : null}
        <span aria-hidden="true" style={{ position: 'absolute', top: 0, bottom: 0, left: labelWidth + x(mission.now), width: 1.5, background: C.fg, opacity: 0.4, pointerEvents: 'none' }} />
        {cursor !== undefined ? <span data-replay-cursor aria-hidden="true" style={{ position: 'absolute', top: 0, bottom: 0, left: labelWidth + x(cursor), width: 2, background: C.fg, pointerEvents: 'none' }} /> : null}
      </div>
    </div>
  </div>
}

// ---------------------------------------------------------------------------------------------------------------------

/** The replay scrubber: a slider over the mission's wall clock, play and pause, back to now. */
export const Replay = ({ mission, linked }: { readonly mission: ExploreMission; readonly linked: Linked }) => {
  const [playing, setPlaying] = React.useState(false)
  const t = linked.cursor ?? mission.now
  React.useEffect(() => {
    if (!playing) return
    const id = setInterval(() => {
      const next = (linked.cursor ?? mission.startedAt) + 20 * 60_000
      if (next >= mission.now) { linked.setCursor(undefined); setPlaying(false) } else linked.setCursor(next)
    }, 60)
    return () => clearInterval(id)
  })
  return <div role="group" aria-label="Replay" style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12, color: C.soft }}>
    <SmallButton label={playing ? 'Pause replay' : 'Play replay from the start'} onPress={() => { if (!playing && linked.cursor === undefined) linked.setCursor(mission.startedAt); setPlaying(!playing) }}>{playing ? '❚❚' : '▶'}</SmallButton>
    <input type="range" aria-label="Replay time" aria-valuetext={clock(t)} min={mission.startedAt} max={mission.now} step={5 * 60_000} value={t}
      onChange={event => { const v = Number(event.target.value); linked.setCursor(v >= mission.now ? undefined : v) }} style={{ flex: 1, minWidth: 160 }} />
    <span style={{ minWidth: 90, color: linked.cursor === undefined ? C.muted : C.fg, fontVariantNumeric: 'tabular-nums' }}>{linked.cursor === undefined ? 'Now' : clock(t)}</span>
    {linked.cursor !== undefined ? <SmallButton label="Back to now" onPress={() => { setPlaying(false); linked.setCursor(undefined) }}>Now</SmallButton> : null}
  </div>
}

/** The shared step card, anchored to whichever element the pointer or focus rests on. */
export const StepCardHost = ({ anchor, open, onOpenChange, children }: { readonly anchor: Element | null; readonly open: boolean; readonly onOpenChange: (open: boolean) => void; readonly children: React.ReactNode }) => {
  const ref = React.useMemo(() => ({ current: anchor }), [anchor])
  return <HoverCardSurface triggerRef={ref} isOpen={open && anchor !== null} onOpenChange={onOpenChange} placement="right top" label="Step details">
    <div onPointerEnter={() => onOpenChange(true)} onPointerLeave={() => onOpenChange(false)}>{children}</div>
  </HoverCardSurface>
}
