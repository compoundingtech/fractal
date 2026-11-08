// EXPLORATION ONLY: the parts the detail variants are built from. Plain SVG and inline styles over the kit's tokens, no
// new dependency. Colour marks only what needs the person (amber) or failed (red); everything else is neutral ink.
import * as React from 'react'

import { borderVars as border, statusVars as tone, surfaceVars as surface, textVars as ink } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { MissionRing } from '../../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarMissionLine.tsx'
import { layered } from '../detail/layout.ts'
import { clock, criticalPath, progressOf, runningMs, span, stepEnd, stepStart, type ExploreMission, type PlanStep, type StepState } from './model.ts'

export const C = { fg: ink.fg, soft: ink.fgSoft, muted: ink.fgMuted, line: border.border, strong: border.borderStrong, hover: surface.rowHover, active: surface.rowActive, canvas: surface.canvas, you: tone.attention, bad: tone.dangerFg }

export const STATE_LABEL: Record<StepState, string> = { done: 'Done', running: 'Running', failed: 'Failed', 'waiting-you': 'Needs you', waiting: 'Waiting', ready: 'Ready', pending: 'Pending', cancelled: 'Removed' }

/** Step mark: filled when done, a ring while running, amber when it needs you, red when it failed, hollow otherwise. */
export const Mark = ({ state, size = 10 }: { readonly state: StepState; readonly size?: number }) => {
  const r = size / 2 - 1
  const c = size / 2
  return <svg aria-hidden="true" width={size} height={size} viewBox={`0 0 ${size} ${size}`} style={{ flexShrink: 0 }}>
    {state === 'done' ? <circle cx={c} cy={c} r={r} fill={C.soft} />
      : state === 'running' ? <><circle cx={c} cy={c} r={r} fill="none" stroke={C.strong} strokeWidth={1.5} /><path d={`M${c} ${c - r} A${r} ${r} 0 0 1 ${c + r} ${c}`} fill="none" stroke={C.fg} strokeWidth={1.5} /></>
        : state === 'failed' ? <circle cx={c} cy={c} r={r} fill={C.bad} />
          : state === 'waiting-you' ? <rect x={c - r * 0.8} y={c - r * 0.8} width={r * 1.6} height={r * 1.6} fill={C.you} transform={`rotate(45 ${c} ${c})`} />
            : <circle cx={c} cy={c} r={r} fill="none" stroke={C.muted} strokeWidth={1.25} strokeDasharray={state === 'cancelled' ? '2 2' : undefined} />}
  </svg>
}

// ---------------------------------------------------------------------------------------------------------------------
// Overview header: how is it going overall.

export const Overview = ({ mission, compact = false, onSelect }: { readonly mission: ExploreMission; readonly compact?: boolean; readonly onSelect: (id: string) => void }) => {
  const { done, total } = progressOf(mission)
  const elapsed = mission.now - mission.startedAt
  const live = mission.steps.filter(step => step.state !== 'cancelled')
  const running = live.filter(step => step.state === 'running')
  const you = live.filter(step => step.state === 'waiting-you')
  const failed = live.filter(step => step.state === 'failed')
  const doneIds = new Set(live.filter(step => step.state === 'done').map(step => step.id))
  const next = live.find(step => step.state === 'pending' && step.after.every(dep => doneIds.has(dep) || live.find(s => s.id === dep)?.state !== 'pending'))
  const over = elapsed > mission.expectedMs
  const Chip = ({ step, color }: { readonly step: PlanStep; readonly color?: string }) =>
    <button type="button" onClick={() => onSelect(step.id)} style={{ display: 'inline-flex', alignItems: 'center', gap: 6, border: 0, background: 'transparent', color: color ?? C.fg, font: 'inherit', padding: '2px 4px', borderRadius: 4, cursor: 'pointer', textAlign: 'start' }}>
      <Mark state={step.state} />{step.title}
    </button>
  return <header aria-label="Mission overview" style={{ display: 'flex', flexDirection: 'column', gap: compact ? 6 : 10, padding: compact ? '12px 16px' : '16px 20px', borderBottom: `1px solid ${C.line}` }}>
    <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' }}>
      <h1 style={{ margin: 0, fontSize: 16, fontWeight: 600 }}>{mission.title}</h1>
      <span style={{ color: C.soft, fontSize: 12, display: 'inline-flex', alignItems: 'center', gap: 6 }}><MissionRing done={done} total={total} /> {done}/{total} steps</span>
      <span style={{ color: over ? C.you : C.soft, fontSize: 12 }}>{span(elapsed)} of ~{span(mission.expectedMs)} expected</span>
      <span style={{ color: C.muted, fontSize: 12 }}>started {clock(mission.startedAt)}</span>
    </div>
    {/* Time spent against expected: one thin bar, the expected end as a tick. */}
    <div aria-hidden="true" style={{ position: 'relative', height: 4, background: C.hover, borderRadius: 2, maxWidth: 560 }}>
      <div style={{ position: 'absolute', inset: 0, width: `${Math.min(100, elapsed / Math.max(elapsed, mission.expectedMs) * 100)}%`, background: C.soft, borderRadius: 2 }} />
      <div style={{ position: 'absolute', top: -2, bottom: -2, left: `${Math.min(100, mission.expectedMs / Math.max(elapsed, mission.expectedMs) * 100)}%`, width: 1, background: C.fg }} />
    </div>
    <div style={{ display: 'grid', gridTemplateColumns: compact ? 'repeat(auto-fit, minmax(14rem, 1fr))' : 'repeat(auto-fit, minmax(16rem, 1fr))', gap: compact ? 4 : 8, fontSize: 13 }}>
      <div><div style={{ color: C.muted, fontSize: 11 }}>Running now</div>{running.length === 0 ? <span style={{ color: C.muted }}>nothing</span> : running.map(step => <Chip key={step.id} step={step} />)}</div>
      <div><div style={{ color: C.muted, fontSize: 11 }}>Needs you · blocked</div>
        {you.map(step => <Chip key={step.id} step={step} color={C.you} />)}{failed.map(step => <Chip key={step.id} step={step} color={C.bad} />)}
        {you.length + failed.length === 0 ? <span style={{ color: C.muted }}>nothing</span> : null}</div>
      <div><div style={{ color: C.muted, fontSize: 11 }}>Next</div>{next === undefined ? <span style={{ color: C.muted }}>—</span> : <Chip step={next} />}</div>
    </div>
  </header>
}

// ---------------------------------------------------------------------------------------------------------------------
// DAG: layered, pan and zoom, keyboard, critical path, durations. Optional agent swimlanes.

const NODE_W = 184
const NODE_H = 46
const GAP_X = 56
const GAP_Y = 18

const agentOf = (step: PlanStep) => step.attempts.at(-1)?.agent ?? 'unassigned'

export const Dag = ({ mission, selected, onSelect, lanes = false, height = 360, showRemoved = false }: {
  readonly mission: ExploreMission
  readonly selected: string | undefined
  readonly onSelect: (id: string) => void
  /** One horizontal lane per agent instead of crossing-minimised order. */
  readonly lanes?: boolean
  readonly height?: number
  readonly showRemoved?: boolean
}) => {
  const steps = mission.steps.filter(step => showRemoved || step.state !== 'cancelled')
  const layout = React.useMemo(() => layered(steps), [steps])
  const critical = React.useMemo(() => criticalPath(mission), [mission])
  const [showCritical, setShowCritical] = React.useState(true)
  const agents = [...new Set(steps.map(agentOf))]
  const laneIndex = new Map(agents.map((agent, index) => [agent, index]))
  // In lanes, steps of one agent in one layer stack inside the lane.
  const laneSlots = new Map<string, number>()
  const laneDepth = new Map<string, number>()
  if (lanes) for (const step of steps) {
    const key = `${agentOf(step)}:${layout.nodes.get(step.id)!.layer}`
    const slot = laneDepth.get(key) ?? 0
    laneSlots.set(step.id, slot)
    laneDepth.set(key, slot + 1)
  }
  const laneHeight = (agent: string) => Math.max(1, ...[...laneDepth.entries()].filter(([key]) => key.startsWith(`${agent}:`)).map(([, depth]) => depth)) * (NODE_H + GAP_Y)
  const laneTop = new Map<string, number>()
  let y0 = 0
  for (const agent of agents) { laneTop.set(agent, y0); y0 += laneHeight(agent) }
  const pos = (id: string) => {
    const placed = layout.nodes.get(id)!
    const step = steps.find(s => s.id === id)!
    const x = 24 + placed.layer * (NODE_W + GAP_X)
    const y = lanes ? 12 + laneTop.get(agentOf(step))! + laneSlots.get(id)! * (NODE_H + GAP_Y) : 12 + placed.order * (NODE_H + GAP_Y)
    return { x, y }
  }
  const width = 48 + layout.layers * (NODE_W + GAP_X)
  const contentH = lanes ? y0 + 24 : 24 + layout.widest * (NODE_H + GAP_Y)
  const [view, setView] = React.useState({ x: 0, y: 0, k: 1 })
  const drag = React.useRef<{ x: number; y: number; vx: number; vy: number } | undefined>(undefined)
  const svgRef = React.useRef<SVGSVGElement>(null)
  // Fit once.
  React.useEffect(() => {
    const box = svgRef.current?.getBoundingClientRect()
    if (box === undefined || box.width === 0) return
    const k = Math.min(1, box.width / width, height / contentH)
    setView({ x: (box.width - width * k) / 2, y: Math.max(0, (height - contentH * k) / 2), k })
  }, [width, contentH, height])
  const zoom = (factor: number, cx = (svgRef.current?.getBoundingClientRect().width ?? 0) / 2, cy = height / 2) =>
    setView(v => { const k = Math.min(2.5, Math.max(0.3, v.k * factor)); return { k, x: cx - (cx - v.x) * (k / v.k), y: cy - (cy - v.y) * (k / v.k) } })
  // Keyboard: arrows move along dependencies (left/right) and within a layer (up/down).
  const onKey = (event: React.KeyboardEvent) => {
    const current = selected ?? steps[0]?.id
    if (current === undefined) return
    const here = layout.nodes.get(current)
    if (here === undefined) return
    const pick = (layer: number, order: number) => {
      const row = [...layout.nodes.values()].filter(node => node.layer === layer).sort((a, b) => Math.abs(a.order - order) - Math.abs(b.order - order))
      return row[0]?.id
    }
    const target = event.key === 'ArrowRight' ? pick(here.layer + 1, here.order) : event.key === 'ArrowLeft' ? pick(here.layer - 1, here.order)
      : event.key === 'ArrowDown' ? [...layout.nodes.values()].find(node => node.layer === here.layer && node.order === here.order + 1)?.id
        : event.key === 'ArrowUp' ? [...layout.nodes.values()].find(node => node.layer === here.layer && node.order === here.order - 1)?.id : undefined
    if (target !== undefined) { event.preventDefault(); onSelect(target) }
    if (event.key === '+' || event.key === '=') zoom(1.2)
    if (event.key === '-') zoom(1 / 1.2)
  }
  return <div style={{ position: 'relative', border: `1px solid ${C.line}`, borderRadius: 8, overflow: 'hidden', background: C.canvas }}>
    <div style={{ position: 'absolute', top: 8, right: 8, zIndex: 1, display: 'flex', gap: 4 }}>
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
      <g transform={`translate(${view.x} ${view.y}) scale(${view.k})`}>
        {lanes ? agents.map((agent, index) => <g key={agent}>
          <rect x={0} y={laneTop.get(agent)! + 2} width={width} height={laneHeight(agent) - 4} fill={index % 2 === 0 ? C.hover : 'transparent'} opacity={0.5} rx={6} />
          <text x={6} y={laneTop.get(agent)! + 14} fontSize={10} fill={C.muted}>{agent}</text>
        </g>) : null}
        {steps.flatMap(step => step.after.filter(dep => layout.nodes.has(dep)).map(dep => {
          const a = pos(dep)
          const b = pos(step.id)
          const x1 = a.x + NODE_W
          const y1 = a.y + NODE_H / 2
          const x2 = b.x
          const y2 = b.y + NODE_H / 2
          const mid = (x1 + x2) / 2
          const onPath = showCritical && critical.has(dep) && critical.has(step.id)
          const lit = selected !== undefined && (selected === dep || selected === step.id)
          return <path key={`${dep}->${step.id}`} d={`M${x1} ${y1} C${mid} ${y1} ${mid} ${y2} ${x2} ${y2}`} fill="none" stroke={lit ? C.fg : C.strong} strokeWidth={onPath ? 2.5 : 1.25} strokeOpacity={onPath || lit ? 1 : 0.7} markerEnd="url(#arrow)" />
        }))}
        <defs><marker id="arrow" viewBox="0 0 6 6" refX="6" refY="3" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0 L6 3 L0 6 z" fill={C.strong} /></marker></defs>
        {steps.map(step => {
          const { x, y } = pos(step.id)
          const isSel = selected === step.id
          const ran = runningMs(step, mission.now)
          const tries = step.attempts.length
          const stroke = step.state === 'waiting-you' ? C.you : step.state === 'failed' ? C.bad : isSel ? C.fg : C.strong
          return <g key={step.id} data-dag-node={step.id} role="button" aria-pressed={isSel} aria-label={`${step.title}, ${STATE_LABEL[step.state]}${ran > 0 ? `, ran ${span(ran)}` : ''}${tries > 1 ? `, ${tries} attempts` : ''}`}
            transform={`translate(${x} ${y})`} onClick={() => onSelect(step.id)} style={{ cursor: 'pointer' }} opacity={step.state === 'cancelled' ? 0.5 : 1}>
            <rect width={NODE_W} height={NODE_H} rx={7} fill={isSel ? C.active : C.canvas} stroke={stroke} strokeWidth={isSel || stroke !== C.strong ? 1.75 : 1} strokeDasharray={step.state === 'cancelled' ? '4 3' : undefined} />
            {showCritical && critical.has(step.id) ? <rect x={0} y={0} width={3} height={NODE_H} rx={1.5} fill={C.fg} /> : null}
            <foreignObject x={8} y={6} width={NODE_W - 14} height={NODE_H - 10}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2, fontSize: 12, lineHeight: '16px', color: C.fg, fontFamily: 'inherit' }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}><Mark state={step.state} /><span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{step.title}</span></span>
                <span style={{ fontSize: 11, color: C.muted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {step.state === 'pending' ? `~${span(step.expectedMs ?? 0)}` : `${span(ran)}${tries > 1 ? ` · ${tries} tries` : ''}`}{lanes ? '' : ` · ${agentOf(step)}`}
                </span>
              </div>
            </foreignObject>
          </g>
        })}
      </g>
    </svg>
  </div>
}

export const SmallButton = ({ label, onPress, children, pressed }: { readonly label: string; readonly onPress: () => void; readonly children: React.ReactNode; readonly pressed?: boolean }) =>
  <button type="button" aria-label={label} aria-pressed={pressed} onClick={onPress} style={{ minWidth: 24, height: 24, padding: '0 6px', border: `1px solid ${C.strong}`, borderRadius: 5, background: pressed ? C.active : C.canvas, color: C.fg, font: 'inherit', fontSize: 12, cursor: 'pointer' }}>{children}</button>

// ---------------------------------------------------------------------------------------------------------------------
// Trace: wall-clock Gantt. Bars per attempt; waits hatched (amber on you); failures marked with their reason.

const ZOOMS = [{ id: 'all', label: 'All', ms: 0 }, { id: '12h', label: '12h', ms: 12 * 3_600_000 }, { id: '2h', label: '2h', ms: 2 * 3_600_000 }] as const

export const Trace = ({ mission, selected, onSelect, rowHeight = 26, labelWidth = 200, groupByAgent = false, projected = true }: {
  readonly mission: ExploreMission
  readonly selected: string | undefined
  readonly onSelect: (id: string) => void
  readonly rowHeight?: number
  readonly labelWidth?: number
  readonly groupByAgent?: boolean
  /** Draw pending steps' expected bars after their dependencies, dashed. */
  readonly projected?: boolean
}) => {
  const [zoomId, setZoom] = React.useState<(typeof ZOOMS)[number]['id']>('all')
  const layout = React.useMemo(() => layered(mission.steps), [mission])
  const ordered = [...mission.steps].filter(step => step.state !== 'cancelled')
    .sort(groupByAgent ? (a, b) => agentOf(a).localeCompare(agentOf(b)) || (stepStart(a) ?? Infinity) - (stepStart(b) ?? Infinity)
      : (a, b) => layout.nodes.get(a.id)!.layer - layout.nodes.get(b.id)!.layer || (stepStart(a) ?? Infinity) - (stepStart(b) ?? Infinity))
  // Projection: a pending step starts when its latest dependency is expected to end.
  const projectedEnd = new Map<string, number>()
  const projectedStart = new Map<string, number>()
  const endOf = (step: PlanStep): number => {
    const known = projectedEnd.get(step.id)
    if (known !== undefined) return known
    if (step.state === 'done' || step.state === 'failed') { const end = stepEnd(step, mission.now) ?? mission.now; projectedEnd.set(step.id, end); return end }
    const deps = step.after.map(dep => mission.steps.find(s => s.id === dep)).filter((s): s is PlanStep => s !== undefined && s.state !== 'cancelled')
    const start = step.state === 'pending' ? Math.max(mission.now, ...deps.map(endOf)) : stepStart(step) ?? mission.now
    const end = Math.max(mission.now, start + (step.expectedMs ?? 0) - (step.state === 'pending' ? 0 : runningMs(step, mission.now)))
    projectedStart.set(step.id, start)
    projectedEnd.set(step.id, end)
    return end
  }
  ordered.forEach(endOf)
  const t0 = mission.startedAt
  const t1 = Math.max(mission.now, ...(projected ? [...projectedEnd.values()] : [])) + 30 * 60_000
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
  // Keep "now" in view when zoomed in.
  React.useEffect(() => { if (zoom.ms > 0) scrollRef.current?.scrollTo({ left: Math.max(0, x(mission.now) - width * 0.7) }) }, [zoomId])
  React.useEffect(() => { if (selected === undefined || scrollRef.current === null) return; const step = mission.steps.find(s => s.id === selected); const start = step === undefined ? undefined : stepStart(step) ?? projectedStart.get(step.id); if (start !== undefined && zoom.ms > 0) scrollRef.current.scrollTo({ left: Math.max(0, x(start) - 40), behavior: 'smooth' }) }, [selected])
  const tickEvery = zoom.ms === 0 ? 4 * 3_600_000 : zoom.ms > 6 * 3_600_000 ? 3_600_000 : 15 * 60_000
  const ticks: number[] = []
  for (let t = Math.ceil(t0 / tickEvery) * tickEvery; t < t1; t += tickEvery) ticks.push(t)
  let lastAgent: string | undefined
  return <div style={{ border: `1px solid ${C.line}`, borderRadius: 8, overflow: 'hidden' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 8px', borderBottom: `1px solid ${C.line}`, fontSize: 11, color: C.soft }}>
      <span>Zoom</span>{ZOOMS.map(z => <SmallButton key={z.id} label={`Zoom to ${z.label}`} pressed={z.id === zoomId} onPress={() => setZoom(z.id)}>{z.label}</SmallButton>)}
      <span style={{ marginInlineStart: 'auto', display: 'inline-flex', gap: 12, alignItems: 'center' }}>
        <Legend swatch={<span style={{ width: 14, height: 6, background: C.soft, borderRadius: 2 }} />}>ran</Legend>
        <Legend swatch={<span style={{ width: 14, height: 6, background: C.bad, borderRadius: 2 }} />}>failed</Legend>
        <Legend swatch={<Hatch color={C.you} />}>waited on you</Legend>
        <Legend swatch={<Hatch color={C.muted} />}>waited</Legend>
        {projected ? <Legend swatch={<span style={{ width: 14, height: 6, border: `1px dashed ${C.muted}`, borderRadius: 2 }} />}>expected</Legend> : null}
      </span>
    </div>
    <div ref={scrollRef} style={{ overflowX: 'auto', position: 'relative' }}>
      <div role="grid" aria-label={`${mission.title} trace`} style={{ position: 'relative', width: labelWidth + total, fontSize: 12 }}>
        <div role="row" style={{ display: 'flex', height: 20, position: 'sticky', top: 0, borderBottom: `1px solid ${C.line}` }}>
          <div style={{ width: labelWidth, flexShrink: 0, position: 'sticky', left: 0, background: C.canvas, zIndex: 2 }} />
          <div style={{ position: 'relative', flex: 1 }}>{ticks.map(t => <span key={t} style={{ position: 'absolute', left: x(t) + 3, top: 3, fontSize: 10, color: C.muted, whiteSpace: 'nowrap' }}>{clock(t)}</span>)}</div>
        </div>
        {ordered.map(step => {
          const isSel = selected === step.id
          const agent = agentOf(step)
          const header = groupByAgent && agent !== lastAgent ? agent : undefined
          lastAgent = agent
          return <React.Fragment key={step.id}>
            {header !== undefined ? <div style={{ height: 18, paddingInlineStart: 8, fontSize: 10, color: C.muted, display: 'flex', alignItems: 'end', position: 'sticky', left: 0 }}>{header}</div> : null}
            <div role="row" aria-selected={isSel} data-trace-row={step.id} onClick={() => onSelect(step.id)} style={{ display: 'flex', height: rowHeight, alignItems: 'center', cursor: 'pointer', background: isSel ? C.active : 'transparent' }}>
              <div role="rowheader" style={{ width: labelWidth, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 6, paddingInline: 8, position: 'sticky', left: 0, background: isSel ? C.active : C.canvas, zIndex: 1, height: '100%' }}>
                <Mark state={step.state} /><button type="button" onClick={() => onSelect(step.id)} style={{ border: 0, background: 'transparent', color: C.fg, font: 'inherit', padding: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', cursor: 'pointer', textAlign: 'start' }}>{step.title}</button>
              </div>
              <div role="gridcell" style={{ position: 'relative', flex: 1, height: '100%' }}>
                {ticks.map(t => <span key={t} aria-hidden="true" style={{ position: 'absolute', left: x(t), top: 0, bottom: 0, width: 1, background: C.line, opacity: 0.5 }} />)}
                {step.waits.map((wait, index) => <span key={`w${index}`} title={`${wait.kind === 'you' ? 'Waited on you' : 'Waited'}: ${wait.reason} (${span((wait.end ?? mission.now) - wait.start)})`}
                  style={{ position: 'absolute', left: x(wait.start), width: Math.max(2, x(wait.end ?? mission.now) - x(wait.start)), top: rowHeight * 0.3, height: rowHeight * 0.4, backgroundImage: `repeating-linear-gradient(135deg, ${wait.kind === 'you' ? C.you : C.muted} 0 1.5px, transparent 1.5px 5px)`, opacity: wait.kind === 'you' ? 0.9 : 0.5, borderRadius: 2 }} />)}
                {step.attempts.map(attempt => {
                  const left = x(attempt.start)
                  const w = Math.max(3, x(attempt.end ?? mission.now) - left)
                  const failed = attempt.outcome === 'failed'
                  return <span key={attempt.n} title={`Attempt ${attempt.n} by ${attempt.agent}: ${attempt.outcome}${attempt.reason ? ` — ${attempt.reason}` : ''} (${span((attempt.end ?? mission.now) - attempt.start)})`}
                    style={{ position: 'absolute', left, width: w, top: rowHeight * 0.2, height: rowHeight * 0.6, background: failed ? C.bad : attempt.outcome === 'running' ? C.fg : C.soft, opacity: attempt.outcome === 'running' ? 0.85 : 1, borderRadius: 3, display: 'flex', alignItems: 'center', overflow: 'hidden' }}>
                    {w > 70 ? <span style={{ fontSize: 10, color: C.canvas, paddingInline: 4, whiteSpace: 'nowrap' }}>{failed ? `✕ ${attempt.reason ?? ''}` : attempt.agent}</span> : null}
                  </span>
                })}
                {projected && step.state === 'pending' && projectedStart.has(step.id) ? <span title={`Expected: ${span(step.expectedMs ?? 0)}`} style={{ position: 'absolute', left: x(projectedStart.get(step.id)!), width: Math.max(3, x(projectedEnd.get(step.id)!) - x(projectedStart.get(step.id)!)), top: rowHeight * 0.25, height: rowHeight * 0.5, border: `1px dashed ${C.muted}`, borderRadius: 3 }} /> : null}
              </div>
            </div>
          </React.Fragment>
        })}
        <span aria-label="now" style={{ position: 'absolute', top: 0, bottom: 0, left: labelWidth + x(mission.now), width: 1.5, background: C.fg, opacity: 0.6, pointerEvents: 'none' }} />
      </div>
    </div>
  </div>
}

const Hatch = ({ color }: { readonly color: string }) => <span style={{ width: 14, height: 8, backgroundImage: `repeating-linear-gradient(135deg, ${color} 0 1.5px, transparent 1.5px 4px)` }} />
const Legend = ({ swatch, children }: { readonly swatch: React.ReactNode; readonly children: React.ReactNode }) => <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>{swatch}{children}</span>

// ---------------------------------------------------------------------------------------------------------------------
// Inspector, Problems, History.

export const Inspector = ({ mission, stepId }: { readonly mission: ExploreMission; readonly stepId: string | undefined }) => {
  const step = mission.steps.find(s => s.id === stepId)
  if (step === undefined) return <aside style={{ padding: 16, color: C.muted, fontSize: 12 }}>Pick a step in the graph or the trace.</aside>
  const ran = runningMs(step, mission.now)
  const waited = step.waits.reduce((sum, w) => sum + ((w.end ?? mission.now) - w.start), 0)
  const revision = mission.revisions.find(r => r.changes.some(c => c.step === step.id && c.kind !== 'added' || (c.kind === 'added' && c.step === step.id && r.n > 1)))
  return <aside aria-label={`${step.title} details`} style={{ display: 'flex', flexDirection: 'column', gap: 10, padding: 16, fontSize: 12 }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}><Mark state={step.state} size={12} /><strong style={{ fontSize: 14 }}>{step.title}</strong></div>
    <div style={{ color: C.soft }}>{STATE_LABEL[step.state]} · {agentOf(step)} · ran {span(ran)}{waited > 0 ? ` · waited ${span(waited)}` : ''}{step.expectedMs ? ` · usually ${span(step.expectedMs)}` : ''}</div>
    {step.after.length > 0 ? <div style={{ color: C.muted }}>After {step.after.map(dep => mission.steps.find(s => s.id === dep)?.title ?? dep).join(', ')}</div> : null}
    {revision !== undefined ? <div style={{ color: C.muted }}>Changed in revision {revision.n}: {revision.why}</div> : null}
    {step.attempts.length > 0 ? <ol style={{ margin: 0, paddingInlineStart: 18, display: 'flex', flexDirection: 'column', gap: 4 }}>
      {step.attempts.map(a => <li key={a.n}><span style={{ color: a.outcome === 'failed' ? C.bad : C.fg }}>Attempt {a.n}: {a.outcome}</span> <span style={{ color: C.muted }}>· {a.agent} · {clock(a.start)} · {span((a.end ?? mission.now) - a.start)}</span>{a.reason ? <div style={{ color: C.soft }}>{a.reason}</div> : null}{a.turn ? <div><a href="#" onClick={e => e.preventDefault()} style={{ color: C.soft }}>Open the turn in {a.agent}’s conversation</a></div> : null}</li>)}
    </ol> : null}
    {step.waits.map((w, i) => <div key={i} style={{ color: w.kind === 'you' ? C.you : C.soft }}>{w.kind === 'you' ? 'Waiting on you' : 'Waited'}: {w.reason} ({span((w.end ?? mission.now) - w.start)})</div>)}
  </aside>
}

export const Problems = ({ mission, onSelect, stacked = false }: { readonly mission: ExploreMission; readonly onSelect: (id: string) => void; readonly stacked?: boolean }) =>
  <ol aria-label="Problems" style={{ margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column' }}>
    {[...mission.problems].sort((a, b) => b.at - a.at).map((p, i) => <li key={i} style={{ display: 'grid', gridTemplateColumns: stacked ? '1fr' : '90px 1fr auto', gap: stacked ? 2 : 12, padding: '8px 12px', borderBottom: `1px solid ${C.line}`, fontSize: 12, alignItems: 'baseline' }}>
      <span style={{ color: p.kind === 'failure' ? C.bad : p.kind === 'wait-you' ? C.you : C.soft }}>{p.kind === 'failure' ? 'Failed' : p.kind === 'retry' ? 'Retried' : p.kind === 'wait-you' ? 'Waits on you' : 'Stalled'}</span>
      <span><button type="button" onClick={() => onSelect(p.step)} style={{ border: 0, padding: 0, background: 'transparent', color: C.fg, font: 'inherit', cursor: 'pointer', textAlign: 'start' }}>{p.text}</button>
        <span style={{ color: C.muted }}> · {mission.steps.find(s => s.id === p.step)?.title}{p.agent ? ` · ${p.agent}` : ''}{p.turn ? ' · ' : ''}</span>{p.turn ? <a href="#" onClick={e => e.preventDefault()} style={{ color: C.soft }}>conversation turn</a> : null}</span>
      <span style={{ color: C.muted, whiteSpace: 'nowrap' }}>{clock(p.at)}{p.resolvedAt ? ` · resolved after ${span(p.resolvedAt - p.at)}` : ' · open'}</span>
    </li>)}
  </ol>

export const History = ({ mission, onSelect }: { readonly mission: ExploreMission; readonly onSelect: (id: string) => void }) =>
  <ol aria-label="Plan history" style={{ margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 12 }}>
    {[...mission.revisions].reverse().map(r => <li key={r.n} style={{ padding: '8px 12px', borderInlineStart: `2px solid ${C.strong}`, fontSize: 12 }}>
      <div><strong>Revision {r.n}</strong> <span style={{ color: C.muted }}>· {clock(r.at)} · by {r.by}</span></div>
      <div style={{ color: C.soft, marginBlock: 4 }}>{r.why}</div>
      <ul style={{ margin: 0, paddingInlineStart: 0, listStyle: 'none', fontFamily: 'ui-monospace, monospace', fontSize: 11 }}>
        {r.changes.map((c, i) => <li key={i}>
          {c.kind === 'removed' ? <span style={{ color: C.muted, textDecoration: 'line-through' }}>− {c.before}</span>
            : c.kind === 'added' ? <button type="button" onClick={() => onSelect(c.step)} style={{ border: 0, padding: 0, background: 'transparent', color: C.fg, font: 'inherit', cursor: 'pointer' }}>+ {c.after}</button>
              : <><span style={{ color: C.muted, textDecoration: 'line-through' }}>~ {c.before}</span> → <button type="button" onClick={() => onSelect(c.step)} style={{ border: 0, padding: 0, background: 'transparent', color: C.fg, font: 'inherit', cursor: 'pointer' }}>{c.after}</button></>}
        </li>)}
      </ul>
    </li>)}
  </ol>

export const Tabs = <T extends string>({ tabs, current, onChange }: { readonly tabs: ReadonlyArray<{ readonly id: T; readonly label: string; readonly count?: number }>; readonly current: T; readonly onChange: (id: T) => void }) =>
  <div role="tablist" style={{ display: 'flex', gap: 2, paddingInline: 16, borderBottom: `1px solid ${C.line}` }}>
    {tabs.map(tab => <button key={tab.id} type="button" role="tab" aria-selected={tab.id === current} onClick={() => onChange(tab.id)}
      style={{ border: 0, borderBottom: `2px solid ${tab.id === current ? C.fg : 'transparent'}`, background: 'transparent', color: tab.id === current ? C.fg : C.soft, font: 'inherit', fontSize: 13, padding: '8px 10px', cursor: 'pointer' }}>
      {tab.label}{tab.count !== undefined ? <span style={{ color: C.muted, marginInlineStart: 6 }}>{tab.count}</span> : null}
    </button>)}
  </div>
