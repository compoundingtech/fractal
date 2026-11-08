import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { criticalPath, runningMs, span, stateAt, type DetailStep, type MissionDetailModel } from './detailModel.ts'
import type { MissionFocusControls } from './focus.ts'
import { layered } from './layout.ts'
import { dimmedSteps, selectedStep } from './linking.ts'
import { SmallButton, SrOnly, STATE_LABEL, StepMark } from './parts.tsx'

// The plan as a layered dependency graph: pan by dragging, zoom with the buttons, Ctrl or Cmd and the wheel, or + and
// -; arrow keys walk the dependencies and speak each step. Hovering a node lights its lineage and shows it in the
// inspector; there are no hover cards. During replay each node shows its state at the cursor.

const NODE_W = 184
const NODE_H = 46
const GAP_X = 48
const GAP_Y = 16
const PAD = 12
const MAX_HEIGHT = 300
/** Below this the node labels stop being legible; a narrower pane pans instead. */
const MIN_FIT = 0.6

const nodeLabel = (step: DetailStep, state: ReturnType<typeof stateAt>, ran: number) =>
  `${step.title}, ${STATE_LABEL[state]}${ran > 0 ? `, ran ${span(ran)}` : ''}${step.attemptCount > 1 ? `, ${step.attemptCount} attempts` : ''}`

export const PlanGraph = ({ model, controls, agentLabel }: { readonly model: MissionDetailModel; readonly controls: MissionFocusControls; readonly agentLabel: (id: string | undefined) => string | undefined }) => {
  const { focus } = controls
  const steps = React.useMemo(() => model.steps.filter(step => step.state !== 'cancelled'), [model.steps])
  const layout = React.useMemo(() => layered(steps), [steps])
  const critical = React.useMemo(() => criticalPath(steps, model.now), [steps, model.now])
  const [showCritical, setShowCritical] = React.useState(false)
  const [announce, setAnnounce] = React.useState('')
  const dim = dimmedSteps(model, focus)
  const selected = selectedStep(model, focus)
  const pos = (id: string) => {
    const placed = layout.nodes.get(id)!
    return { x: PAD + placed.layer * (NODE_W + GAP_X), y: PAD + placed.order * (NODE_H + GAP_Y) }
  }
  const contentW = PAD * 2 + layout.layers * NODE_W + Math.max(0, layout.layers - 1) * GAP_X
  const contentH = PAD * 2 + layout.widest * NODE_H + Math.max(0, layout.widest - 1) * GAP_Y
  const box = React.useRef<HTMLDivElement>(null)
  const [width, setWidth] = React.useState(0)
  React.useLayoutEffect(() => {
    const element = box.current
    if (element === null) return
    setWidth(element.clientWidth)
    const observer = new ResizeObserver(() => setWidth(element.clientWidth))
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  // Fit: never larger than life, never wider than the pane, never so small its labels cannot be read; past that a
  // narrow pane pans. A narrow pane also gets a shorter graph.
  const fit = width === 0 ? 1 : Math.max(MIN_FIT, Math.min(1, width / contentW, MAX_HEIGHT / contentH))
  const height = Math.max(120, Math.min(MAX_HEIGHT, Math.ceil(contentH * fit)))
  const [view, setView] = React.useState<{ readonly x: number; readonly y: number; readonly k: number } | undefined>(undefined)
  const fitted = { k: fit, x: Math.max(0, (width - contentW * fit) / 2), y: Math.max(0, (height - contentH * fit) / 2) }
  const shown = view ?? fitted
  const zoom = (factor: number, cx = width / 2, cy = height / 2) => setView(current => {
    const at = current ?? fitted
    const k = Math.min(2.5, Math.max(0.25, at.k * factor))
    return { k, x: cx - (cx - at.x) * (k / at.k), y: cy - (cy - at.y) * (k / at.k) }
  })
  const drag = React.useRef<{ readonly x: number; readonly y: number; readonly vx: number; readonly vy: number } | undefined>(undefined)
  const svg = React.useRef<SVGSVGElement>(null)
  const idBase = React.useId()
  const nodeDomId = (id: string) => `${idBase}-n${steps.findIndex(step => step.id === id)}`
  const stateOf = (step: DetailStep) => focus.cursor === undefined ? step.state : stateAt(step, focus.cursor, model.now)
  const ranOf = (step: DetailStep) => focus.cursor === undefined ? runningMs(step, model.now)
    : step.attempts.reduce((sum, attempt) => sum + Math.max(0, Math.min(attempt.end ?? model.now, focus.cursor!) - attempt.start), 0)
  const move = (target: string | undefined) => {
    if (target === undefined) return
    controls.select(target)
    const step = steps.find(each => each.id === target)!
    setAnnounce(nodeLabel(step, stateOf(step), ranOf(step)))
  }
  const onKey = (event: React.KeyboardEvent) => {
    const here = selected === undefined ? undefined : layout.nodes.get(selected)
    const nodes = [...layout.nodes.values()]
    if (here === undefined) { if (event.key.startsWith('Arrow')) { event.preventDefault(); move(steps[0]?.id) } return }
    const nearest = (layer: number) => nodes.filter(node => node.layer === layer).sort((a, b) => Math.abs(a.order - here.order) - Math.abs(b.order - here.order))[0]?.id
    const target = event.key === 'ArrowRight' ? nearest(here.layer + 1) : event.key === 'ArrowLeft' ? nearest(here.layer - 1)
      : event.key === 'ArrowDown' ? nodes.find(node => node.layer === here.layer && node.order === here.order + 1)?.id
        : event.key === 'ArrowUp' ? nodes.find(node => node.layer === here.layer && node.order === here.order - 1)?.id : undefined
    if (event.key.startsWith('Arrow')) { event.preventDefault(); move(target) }
    else if (event.key === '+' || event.key === '=') { event.preventDefault(); zoom(1.2) }
    else if (event.key === '-') { event.preventDefault(); zoom(1 / 1.2) }
    else if (event.key === '0') { event.preventDefault(); setView(undefined) }
  }
  return <div ref={box} data-plan-graph="" {...stylex.props(styles.frame)}>
    <div role="toolbar" aria-label="Graph controls" {...stylex.props(styles.toolbar)}>
      <span {...stylex.props(styles.label)}>Plan</span>
      {focus.cursor !== undefined ? <span data-graph-replaying="" {...stylex.props(styles.replaying)}>Replaying</span> : null}
      <SmallButton label="Critical path" pressed={showCritical} onPress={() => setShowCritical(value => !value)}><span aria-hidden="true">Critical path</span></SmallButton>
      <SmallButton label="Zoom out" onPress={() => zoom(1 / 1.2)}>−</SmallButton>
      <SmallButton label="Zoom in" onPress={() => zoom(1.2)}>+</SmallButton>
      <SmallButton label="Fit graph" onPress={() => setView(undefined)}><span aria-hidden="true">Fit</span></SmallButton>
    </div>
    <svg ref={svg} role="application" aria-roledescription="dependency graph" tabIndex={0}
      aria-label={`Plan, ${steps.length} steps. Arrow keys move between steps, plus and minus zoom, zero fits.`}
      {...(selected === undefined ? {} : { 'aria-activedescendant': nodeDomId(selected) })}
      width="100%" height={height} onKeyDown={onKey}
      onWheel={event => {
        if (!event.ctrlKey && !event.metaKey) return
        event.preventDefault()
        const rect = svg.current!.getBoundingClientRect()
        zoom(event.deltaY < 0 ? 1.1 : 1 / 1.1, event.clientX - rect.left, event.clientY - rect.top)
      }}
      onPointerDown={event => {
        if ((event.target as Element).closest('[data-dag-node]') !== null) return
        drag.current = { x: event.clientX, y: event.clientY, vx: shown.x, vy: shown.y }
        event.currentTarget.setPointerCapture(event.pointerId)
      }}
      onPointerMove={event => {
        const from = drag.current
        if (from !== undefined) setView({ k: shown.k, x: from.vx + event.clientX - from.x, y: from.vy + event.clientY - from.y })
      }}
      onPointerUp={() => { drag.current = undefined }}
      {...stylex.props(styles.svg)}>
      <defs><marker id={`${idBase}-arrow`} viewBox="0 0 6 6" refX="6" refY="3" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0 L6 3 L0 6 z" {...stylex.props(styles.arrow)} /></marker></defs>
      <g transform={`translate(${shown.x} ${shown.y}) scale(${shown.k})`}>
        {steps.flatMap(step => step.after.filter(dep => layout.nodes.has(dep)).map(dep => {
          const a = pos(dep)
          const b = pos(step.id)
          const x1 = a.x + NODE_W
          const y1 = a.y + NODE_H / 2
          const x2 = b.x
          const y2 = b.y + NODE_H / 2
          const mid = (x1 + x2) / 2
          const lit = focus.hovered !== undefined && !dim.has(dep) && !dim.has(step.id)
          const onPath = showCritical && critical.has(dep) && critical.has(step.id)
          return <path key={`${dep}->${step.id}`} data-dag-edge="" data-lit={lit || undefined} d={`M${x1} ${y1} C${mid} ${y1} ${mid} ${y2} ${x2} ${y2}`} fill="none" markerEnd={`url(#${CSS.escape(`${idBase}-arrow`)})`}
            {...stylex.props(styles.edge, (lit || onPath) && styles.edgeLit, focus.hovered !== undefined && !lit && styles.edgeDim)} />
        }))}
        {steps.map(step => {
          const { x, y } = pos(step.id)
          const state = stateOf(step)
          const ran = ranOf(step)
          const isSelected = selected === step.id
          const dimmed = dim.has(step.id)
          const sub = state === 'pending' || state === 'ready'
            ? (focus.cursor === undefined ? step.expected === undefined ? 'not started' : `usually ${span((step.expected.lowMs + step.expected.highMs) / 2)}` : 'not started')
            : `${ran > 0 ? span(ran) : STATE_LABEL[state]}${step.attemptCount > 1 ? ` · ${step.attemptCount} tries` : ''}`
          const who = agentLabel(step.agent)
          return <g key={step.id} id={nodeDomId(step.id)} role="button" aria-pressed={isSelected}
            data-dag-node={step.id} data-state={state} data-dimmed={dimmed || undefined} data-selected={isSelected || undefined}
            transform={`translate(${x} ${y})`} onClick={() => controls.select(step.id)}
            onPointerEnter={() => controls.hover(step.id)} onPointerLeave={() => controls.hover(undefined)}
            {...stylex.props(styles.node, dimmed && styles.dimmed)}>
            <rect width={NODE_W} height={NODE_H} rx={7} {...stylex.props(styles.nodeBox, isSelected && styles.nodeSelected, focus.hovered === step.id && styles.nodeHovered, state === 'waiting-you' && styles.nodeYou, state === 'failed' && styles.nodeBad)} />
            {showCritical && critical.has(step.id) ? <rect data-critical="" width={3} height={NODE_H} rx={1.5} {...stylex.props(styles.criticalBar)} /> : null}
            <foreignObject x={8} y={5} width={NODE_W - 14} height={NODE_H - 8}>
              <div {...stylex.props(styles.nodeBody)}>
                <span {...stylex.props(styles.nodeTitle)}><StepMark state={state} /><span {...stylex.props(styles.ellipsis)}>{step.title}</span><SrOnly>, {STATE_LABEL[state]}{step.attemptCount > 1 ? `, ${step.attemptCount} attempts` : ''},</SrOnly></span>
                <span {...stylex.props(styles.nodeSub, styles.ellipsis)}>{sub}{who === undefined ? '' : ` · ${who}`}</span>
              </div>
            </foreignObject>
          </g>
        })}
      </g>
    </svg>
    <div role="status" aria-live="polite"><SrOnly>{announce}</SrOnly></div>
  </div>
}

const styles = stylex.create({
  frame: { position: 'relative', minWidth: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden', backgroundColor: surface.canvas },
  toolbar: { display: 'flex', flexWrap: 'wrap', justifyContent: 'flex-end', alignItems: 'center', gap: s.xs, paddingInline: s.md, paddingBlock: s.sm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  label: { marginInlineEnd: 'auto', color: ink.fgMuted, fontSize: t.denseSize, fontWeight: 500 },
  replaying: { marginInlineEnd: s.xs, color: ink.fg, fontSize: t.denseSize },
  svg: { display: 'block', cursor: 'grab', touchAction: 'none', outline: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: '-2px' } },
  arrow: { fill: ink.fgMuted },
  edge: { stroke: ink.fgMuted, strokeWidth: 1.25, strokeOpacity: 0.6, transitionProperty: 'stroke-opacity', transitionDuration: '120ms' },
  edgeLit: { stroke: ink.fg, strokeWidth: 2, strokeOpacity: 1 },
  edgeDim: { strokeOpacity: 0.2 },
  node: { cursor: 'pointer', opacity: 1, transitionProperty: 'opacity', transitionDuration: '120ms' },
  dimmed: { opacity: 0.3 },
  nodeBox: { fill: surface.canvas, stroke: border.borderStrong, strokeWidth: 1 },
  nodeSelected: { fill: surface.rowActive, stroke: ink.fgSoft, strokeWidth: 1.5 },
  nodeHovered: { stroke: ink.fg, strokeWidth: 1.5 },
  nodeYou: { stroke: tone.attention, strokeWidth: 1.75 },
  nodeBad: { stroke: tone.dangerFg, strokeWidth: 1.75 },
  criticalBar: { fill: ink.fg },
  nodeBody: { display: 'flex', flexDirection: 'column', gap: '2px', color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, pointerEvents: 'none' },
  nodeTitle: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0 },
  nodeSub: { color: ink.fgMuted, fontSize: t.denseSize },
  ellipsis: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
})
