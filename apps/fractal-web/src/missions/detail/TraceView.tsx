import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { span, type DetailStep, type MissionDetailModel } from './detailModel.ts'
import type { MissionFocusControls } from './focus.ts'
import { dimmedSteps, selectedStep, shownState } from './linking.ts'
import { SmallButton, SrOnly, STATE_LABEL, StepMark } from './parts.tsx'
import { traceLine } from './traceLine.ts'
import { clock, placeSpanLabels, projectedSpans, scrubKey, scrubTo, tickEvery, ticksIn, traceWindow } from './traceScale.ts'

// The wall-clock trace: one two-line row per step (state and title; who and what it cost), its attempts as bars with
// their duration beside each, waits as hatched gaps (amber when on the person), and typical durations of steps not yet
// started as dashed outlines. The playhead lives on the time axis: click or drag the axis (or use the slider keys) to
// replay, and the graph shows each step's state at that moment. Dragging across the rows brushes a time range instead;
// a press without a drag picks the row. Hovering a row drives the inspector; there are no hover cards.

const MIN = 60_000
const HOUR = 60 * MIN
const ZOOMS = [{ id: 'all', label: 'All', ms: 0 }, { id: '12h', label: '12h', ms: 12 * HOUR }, { id: '2h', label: '2h', ms: 2 * HOUR }] as const
type ZoomId = (typeof ZOOMS)[number]['id']
const DRAG_PX = 4
/** A replay from the start takes about this many frames. */
const PLAY_FRAMES = 160
const PLAY_FRAME_MS = 50
const labelWidth = (text: string) => text.length * 6 + 4

const OUTCOME_WORD = { done: 'done', failed: 'failed', running: 'running', cancelled: 'cancelled' } as const
const WAIT_WORD = { you: 'waited on you', dependency: 'waited on dependencies', queue: 'queued', paused: 'paused', retry: 'retry backoff', other: 'waited' } as const

const rowSummary = (step: DetailStep, now: number): string => {
  const runs = step.attempts.map(attempt => `attempt ${attempt.n} ${OUTCOME_WORD[attempt.outcome]}, ${span((attempt.end ?? now) - attempt.start)}, from ${clock(attempt.start)}${attempt.reason === undefined ? '' : `: ${attempt.reason}`}`)
  const waits = step.waits.map(wait => `${WAIT_WORD[wait.kind]} ${span((wait.end ?? now) - wait.start)}`)
  const all = [...runs, ...waits]
  return all.length === 0 ? `${step.title}: not started` : `${step.title}: ${all.join('; ')}`
}

export const TraceView = ({ model, controls, agentLabel, visible }: {
  readonly model: MissionDetailModel
  readonly controls: MissionFocusControls
  readonly agentLabel: (id: string | undefined) => string | undefined
  /** False while the tab is hidden: replay pauses. */
  readonly visible: boolean
}) => {
  const { focus } = controls
  const steps = React.useMemo(() => model.steps.filter(step => step.state !== 'cancelled'), [model.steps])
  const projected = React.useMemo(() => projectedSpans(model), [model])
  const { t0, t1 } = traceWindow(model, projected)
  const [zoomId, setZoom] = React.useState<ZoomId>('all')
  const scroller = React.useRef<HTMLDivElement>(null)
  const [width, setWidth] = React.useState(0)
  React.useLayoutEffect(() => {
    const element = scroller.current
    if (element === null) return
    setWidth(element.clientWidth)
    const observer = new ResizeObserver(() => setWidth(element.clientWidth))
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  const header = width > 0 && width < 560 ? 156 : 272
  const plot = Math.max(120, width - header)
  const zoom = ZOOMS.find(each => each.id === zoomId)!
  const pxPerMs = plot / (zoom.ms === 0 ? t1 - t0 : zoom.ms)
  const total = Math.max(plot, (t1 - t0) * pxPerMs)
  const x = (at: number) => (at - t0) * pxPerMs
  const timeAt = (px: number) => Math.min(t1, Math.max(t0, t0 + px / pxPerMs))
  const every = tickEvery(pxPerMs)
  const ticks = ticksIn(t0, t1, every)
  const bounds = { t0, now: model.now, step: Math.max(MIN, every / 4), page: every }
  const playhead = focus.cursor ?? model.now
  const dim = dimmedSteps(model, focus)
  const selected = selectedStep(model, focus)

  React.useEffect(() => {
    if (zoom.ms === 0) return
    scroller.current?.scrollTo({ left: Math.max(0, x(playhead) - plot * 0.7) })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only a zoom change re-centres; scrubbing must not fight the reader's scroll.
  }, [zoomId])

  const [playing, setPlaying] = React.useState(false)
  React.useEffect(() => {
    if (!playing || !visible) return
    const id = setTimeout(() => {
      const next = (focus.cursor ?? t0) + (model.now - t0) / PLAY_FRAMES
      if (next >= model.now) { controls.setCursor(undefined); setPlaying(false) } else controls.setCursor(next)
    }, PLAY_FRAME_MS)
    return () => clearTimeout(id)
  }, [playing, visible, focus.cursor, t0, model.now, controls])

  // The axis scrubs; the rows brush. Each keeps its own pointer capture so the two never mix.
  const thumb = React.useRef<HTMLDivElement>(null)
  const scrubbing = React.useRef(false)
  const scrubFrom = (event: React.PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    controls.setCursor(scrubTo(timeAt(event.clientX - rect.left), bounds))
  }
  const press = React.useRef<{ readonly px: number; readonly row: string | undefined; moved: boolean } | undefined>(undefined)
  // Keyboard focus on a row drives the inspector like hover; a click's focus does not, so it never sticks after a pick.
  const pressed = React.useRef(false)
  const focused = React.useRef<string | undefined>(undefined)
  const plotX = (event: React.PointerEvent<HTMLDivElement>) => event.clientX - event.currentTarget.getBoundingClientRect().left - header

  return <section aria-label="Trace" data-trace="" {...stylex.props(styles.frame)}>
    <div role="toolbar" aria-label="Trace controls" {...stylex.props(styles.toolbar)}>
      <SmallButton label={playing ? 'Pause replay' : 'Play replay'} pressed={playing} onPress={() => {
        if (!playing && focus.cursor === undefined) controls.setCursor(t0)
        setPlaying(!playing)
      }}><span aria-hidden="true">{playing ? 'Pause' : 'Play'}</span></SmallButton>
      <SmallButton label="Back to now" isDisabled={focus.cursor === undefined} onPress={() => { setPlaying(false); controls.setCursor(undefined) }}><span aria-hidden="true">Now</span></SmallButton>
      {focus.cursor === undefined ? null : <span data-trace-time="" aria-hidden="true" {...stylex.props(styles.time)}>{clock(focus.cursor)}</span>}
      <span {...stylex.props(styles.zooms)}>
        {ZOOMS.map(each => <SmallButton key={each.id} label={each.ms === 0 ? 'All: fit the whole run' : `Zoom to ${each.label}`} pressed={each.id === zoomId} onPress={() => setZoom(each.id)}><span aria-hidden="true">{each.label}</span></SmallButton>)}
      </span>
      {focus.brush !== undefined
        ? <span data-trace-brush-label="" {...stylex.props(styles.brushLabel)}>{clock(focus.brush[0])} – {clock(focus.brush[1])}<SmallButton label="Clear time range" onPress={() => controls.setBrush(undefined)}><span aria-hidden="true">Clear</span></SmallButton></span>
        : <span {...stylex.props(styles.hint)}>Click or drag the time axis to replay; drag across the rows to pick a time range.</span>}
    </div>
    <div ref={scroller} {...stylex.props(styles.scroller)}>
      <div role="grid" aria-label={`Trace of ${model.title}`} aria-rowcount={steps.length + 1} aria-colcount={2}
        onKeyDown={event => { if (event.key === 'Escape' && focus.brush !== undefined) { event.preventDefault(); controls.setBrush(undefined) } }}
        style={{ width: header + total }} {...stylex.props(styles.grid)}>
        <div role="rowgroup">
          <div role="row" aria-rowindex={1} {...stylex.props(styles.axisRow)}>
            <div role="columnheader" style={{ width: header }} {...stylex.props(styles.corner)}>Step</div>
            <div role="columnheader" data-trace-axis="" aria-label="Time" {...stylex.props(styles.axis)}
              onPointerDown={event => {
                if (event.button !== 0) return
                scrubbing.current = true
                event.currentTarget.setPointerCapture(event.pointerId)
                setPlaying(false)
                scrubFrom(event)
                thumb.current?.focus({ preventScroll: true })
              }}
              onPointerMove={event => { if (scrubbing.current) scrubFrom(event) }}
              onPointerUp={() => { scrubbing.current = false }}
              onPointerCancel={() => { scrubbing.current = false }}>
              {ticks.map(tick => <span key={tick} aria-hidden="true" style={{ left: x(tick) }} {...stylex.props(styles.tick)}>{clock(tick)}</span>)}
              <div ref={thumb} role="slider" tabIndex={0} aria-label="Replay time" data-trace-playhead={focus.cursor === undefined ? 'now' : 'replay'}
                aria-valuemin={t0} aria-valuemax={model.now} aria-valuenow={playhead} aria-valuetext={focus.cursor === undefined ? `Now, ${clock(model.now)}` : clock(focus.cursor)}
                onKeyDown={event => {
                  const next = scrubKey(event.key, focus.cursor, bounds)
                  if (!next.handled) return
                  event.preventDefault()
                  setPlaying(false)
                  controls.setCursor(next.cursor)
                }}
                style={{ left: x(playhead) }} {...stylex.props(styles.thumb, focus.cursor !== undefined && styles.thumbReplaying)} />
            </div>
          </div>
        </div>
        <div role="rowgroup" data-trace-rows="" {...stylex.props(styles.rows)}
          onPointerDown={event => {
            if (event.button !== 0 || (event.target as Element).closest('[role="rowheader"]') !== null) return
            press.current = { px: plotX(event), row: (event.target as Element).closest('[data-trace-row]')?.getAttribute('data-trace-row') ?? undefined, moved: false }
            event.currentTarget.setPointerCapture(event.pointerId)
          }}
          onPointerMove={event => {
            const from = press.current
            if (from === undefined) return
            const px = plotX(event)
            if (!from.moved && Math.abs(px - from.px) <= DRAG_PX) return
            from.moved = true
            const [a, b] = [timeAt(from.px), timeAt(px)]
            controls.setBrush(a < b ? [a, b] : [b, a])
          }}
          onPointerUp={() => {
            const from = press.current
            press.current = undefined
            if (from !== undefined && !from.moved && from.row !== undefined) controls.select(from.row)
          }}
          onPointerCancel={() => { press.current = undefined }}>
          {steps.map((step, index) => {
            const state = shownState(model, step.id, focus.cursor)
            const line = traceLine(step, agentLabel(step.agent), model.now)
            const isSelected = selected === step.id
            const bars = step.attempts.map(attempt => {
              const x0 = x(attempt.start)
              const x1 = Math.max(x0 + 3, x(attempt.end ?? model.now))
              const text = span((attempt.end ?? model.now) - attempt.start)
              return { attempt, x0, x1, text, label: labelWidth(text) }
            })
            const places = placeSpanLabels(bars, total)
            const projection = projected.get(step.id)
            return <div key={step.id} role="row" aria-rowindex={index + 2} aria-selected={isSelected} data-trace-row={step.id} data-state={state}
              data-dimmed={dim.has(step.id) || undefined} data-selected={isSelected || undefined}
              onPointerEnter={() => controls.hover(step.id)} onPointerLeave={() => controls.hover(undefined)}
              {...stylex.props(styles.row, isSelected && styles.rowSelected, focus.hovered === step.id && styles.rowHovered, dim.has(step.id) && styles.dimmed)}>
              <div role="rowheader" style={{ width: header }} {...stylex.props(styles.rowHeader, isSelected && styles.rowHeaderSelected)}>
                <button type="button" aria-pressed={isSelected} onClick={() => controls.select(step.id)} onPointerDown={() => { pressed.current = true }}
                  onFocus={() => { if (!pressed.current) { focused.current = step.id; controls.hover(step.id) } pressed.current = false }}
                  onBlur={() => { if (focused.current === step.id) { focused.current = undefined; controls.hover(undefined) } }} {...stylex.props(styles.rowButton)}>
                  <span {...stylex.props(styles.line1)}><StepMark state={state} /><span {...stylex.props(styles.ellipsis)}>{step.title}</span><SrOnly>, {STATE_LABEL[state]}</SrOnly></span>
                  <span data-trace-line="" {...stylex.props(styles.line2, isSelected && styles.line2Selected)}>
                    <span {...stylex.props(styles.ellipsis)}>{line.parts.join(' · ')}{line.you === undefined ? null : <> · <span data-trace-you="" {...stylex.props(styles.you)}>{line.you}</span></>}</span>
                  </span>
                </button>
              </div>
              <div role="gridcell" {...stylex.props(styles.cell)}>
                <SrOnly>{rowSummary(step, model.now)}</SrOnly>
                {step.waits.map((wait, waitIndex) => wait.kind === 'you' ? null : <span key={`w${waitIndex}`} data-trace-wait={wait.kind} aria-hidden="true"
                  style={{ left: x(wait.start), width: Math.max(2, x(wait.end ?? model.now) - x(wait.start)) }} {...stylex.props(styles.wait)} />)}
                {projection === undefined ? null : <span data-trace-projected="" aria-hidden="true" style={{ left: x(projection.start), width: Math.max(3, x(projection.end) - x(projection.start)) }} {...stylex.props(styles.projected)} />}
                {bars.map((bar, barIndex) => <React.Fragment key={bar.attempt.n}>
                  <span data-trace-span={bar.attempt.outcome} aria-hidden="true" style={{ left: bar.x0, width: bar.x1 - bar.x0 }}
                    {...stylex.props(styles.bar, bar.attempt.outcome === 'failed' && styles.barFailed, bar.attempt.outcome === 'running' && styles.barRunning, bar.attempt.outcome === 'cancelled' && styles.barCancelled)} />
                  {places[barIndex] === 'none' ? null : <span data-trace-duration={places[barIndex]} aria-hidden="true"
                    style={places[barIndex] === 'after' ? { left: bar.x1 + 4 } : places[barIndex] === 'inside' ? { left: bar.x0 + 4 } : { left: bar.x0 - 4 - bar.label }}
                    {...stylex.props(styles.duration, places[barIndex] === 'inside' && styles.durationInside, places[barIndex] === 'inside' && bar.attempt.outcome === 'failed' && styles.durationOnFailed)}>{bar.text}</span>}
                </React.Fragment>)}
                {/* A wait on the person can fall inside an open attempt; it draws on top so the amber always shows. */}
                {step.waits.map((wait, waitIndex) => wait.kind !== 'you' ? null : <span key={`y${waitIndex}`} data-trace-wait="you" aria-hidden="true"
                  style={{ left: x(wait.start), width: Math.max(2, x(wait.end ?? model.now) - x(wait.start)) }} {...stylex.props(styles.wait, styles.waitYou)} />)}
              </div>
            </div>
          })}
          <div aria-hidden="true" style={{ left: header, width: total }} {...stylex.props(styles.overlay)}>
            {ticks.map(tick => <span key={tick} style={{ left: x(tick) }} {...stylex.props(styles.gridLine)} />)}
            {focus.brush === undefined ? null : <span data-trace-brush="" style={{ left: x(focus.brush[0]), width: Math.max(1, x(focus.brush[1]) - x(focus.brush[0])) }} {...stylex.props(styles.brush)} />}
            <span data-trace-now="" style={{ left: x(model.now) }} {...stylex.props(styles.nowLine)} />
            {focus.cursor === undefined ? null : <span data-replay-cursor="" style={{ left: x(focus.cursor) }} {...stylex.props(styles.cursorLine)} />}
          </div>
        </div>
      </div>
    </div>
  </section>
}

const ROW = '40px'

const styles = stylex.create({
  frame: { display: 'flex', flexDirection: 'column', minWidth: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden', backgroundColor: surface.canvas },
  toolbar: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, paddingInline: s.md, paddingBlock: s.sm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, color: ink.fgMuted, fontSize: t.denseSize },
  time: { minWidth: '6.5rem', color: ink.fg, fontWeight: 600, fontVariantNumeric: 'tabular-nums' },
  zooms: { display: 'inline-flex', gap: s.xs },
  hint: { flex: '1 1 14rem', minWidth: 0, color: ink.fgMuted },
  brushLabel: { display: 'inline-flex', alignItems: 'center', gap: s.sm, color: ink.fg, fontVariantNumeric: 'tabular-nums' },
  scroller: { position: 'relative', overflowX: 'auto', overflowY: 'hidden', minWidth: 0 },
  grid: { position: 'relative', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, userSelect: 'none', outline: 'none' },
  axisRow: { display: 'flex', height: g.controlSm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  corner: { position: 'sticky', left: 0, zIndex: 3, flexShrink: 0, display: 'flex', alignItems: 'center', paddingInline: s.md, backgroundColor: surface.canvas, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: 500 },
  axis: { position: 'relative', flex: '1 1 auto', cursor: 'ew-resize', backgroundColor: surface.rowHover, touchAction: 'none' },
  tick: { position: 'absolute', top: '5px', paddingInlineStart: '3px', color: ink.fgMuted, fontSize: t.denseSize, whiteSpace: 'nowrap', pointerEvents: 'none', fontVariantNumeric: 'tabular-nums' },
  thumb: { position: 'absolute', top: 0, bottom: 0, width: '10px', marginInlineStart: '-5px', zIndex: 2, borderRadius: r.xs, backgroundColor: ink.fgMuted, opacity: 0.6, cursor: 'grab', outline: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, opacity: 1 } },
  thumbReplaying: { backgroundColor: ink.fg, opacity: 1 },
  rows: { position: 'relative', touchAction: 'pan-y' },
  row: { position: 'relative', display: 'flex', height: ROW, cursor: 'pointer', opacity: 1, transitionProperty: 'opacity', transitionDuration: '120ms', borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  rowSelected: { backgroundColor: surface.rowActive },
  rowHovered: { backgroundColor: surface.rowHover },
  dimmed: { opacity: 0.35 },
  rowHeader: { position: 'sticky', left: 0, zIndex: 2, flexShrink: 0, display: 'flex', minWidth: 0, backgroundColor: surface.canvas, borderInlineEndWidth: g.hairline, borderInlineEndStyle: 'solid', borderInlineEndColor: border.border },
  rowHeaderSelected: { backgroundColor: surface.rowActive },
  rowButton: { display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: '1px', width: '100%', minWidth: 0, margin: 0, paddingInline: s.md, borderWidth: 0, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: 'inherit', textAlign: 'start', cursor: 'pointer',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: '-2px' } },
  line1: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0 },
  line2: { display: 'flex', minWidth: 0, paddingInlineStart: `calc(10px + ${s.sm})`, color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  line2Selected: { color: ink.fgSoft },
  you: { color: `color-mix(in srgb, ${tone.attention} 80%, ${ink.fg})` },
  ellipsis: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  cell: { position: 'relative', flex: '1 1 auto' },
  wait: { position: 'absolute', top: '15px', height: '10px', borderRadius: r.xs, backgroundImage: `repeating-linear-gradient(135deg, ${ink.fgMuted} 0 1.5px, transparent 1.5px 5px)`, opacity: 0.5 },
  waitYou: { backgroundImage: `repeating-linear-gradient(135deg, ${tone.attention} 0 1.5px, transparent 1.5px 5px)`, opacity: 0.95 },
  projected: { position: 'absolute', top: '12px', height: '16px', boxSizing: 'border-box', borderWidth: g.hairline, borderStyle: 'dashed', borderColor: ink.fgMuted, borderRadius: r.xs },
  bar: { position: 'absolute', top: '12px', height: '16px', borderRadius: r.xs, backgroundColor: ink.fgMuted },
  barRunning: { backgroundColor: ink.fg },
  barFailed: { backgroundColor: tone.dangerFg },
  barCancelled: { backgroundColor: 'transparent', borderWidth: g.hairline, borderStyle: 'dashed', borderColor: ink.fgMuted, boxSizing: 'border-box' },
  duration: { position: 'absolute', top: '12px', height: '16px', display: 'flex', alignItems: 'center', color: ink.fgMuted, fontSize: t.denseSize, whiteSpace: 'nowrap', pointerEvents: 'none', fontVariantNumeric: 'tabular-nums' },
  durationInside: { color: surface.canvas, fontWeight: 600 },
  durationOnFailed: { color: surface.canvas },
  overlay: { position: 'absolute', top: 0, bottom: 0, pointerEvents: 'none' },
  gridLine: { position: 'absolute', top: 0, bottom: 0, width: '1px', backgroundColor: border.border },
  brush: { position: 'absolute', top: 0, bottom: 0, backgroundColor: interactionVars.wash, borderInlineWidth: g.hairline, borderInlineStyle: 'solid', borderInlineColor: border.railStrong },
  nowLine: { position: 'absolute', top: 0, bottom: 0, width: '1px', backgroundColor: ink.fgMuted, opacity: 0.6 },
  cursorLine: { position: 'absolute', top: 0, bottom: 0, width: '2px', marginInlineStart: '-1px', backgroundColor: ink.fg },
})
