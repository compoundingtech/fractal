import { stepEnd, type MissionDetailModel } from './detailModel.ts'

// The trace's wall clock, kept pure so the view only maps times to pixels: where steps not yet started would run,
// the window the trace spans, round tick times, where each span's duration label fits, and how the playhead scrubs.

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

export interface Projection { readonly start: number; readonly end: number }

/** Where each step not yet started would run if it took its typical time once its dependencies end. Steps without a typical duration are not projected: nothing is invented. */
export const projectedSpans = (model: MissionDetailModel): ReadonlyMap<string, Projection> => {
  const byId = new Map(model.steps.map(step => [step.id, step]))
  const out = new Map<string, Projection>()
  const endOf = (id: string, seen: ReadonlySet<string>): number => {
    const step = byId.get(id)
    if (step === undefined || step.state === 'cancelled') return model.now
    if (step.state === 'done') return stepEnd(step, model.now) ?? model.now
    const projected = project(id, seen)
    return projected?.end ?? model.now
  }
  const project = (id: string, seen: ReadonlySet<string>): Projection | undefined => {
    const known = out.get(id)
    if (known !== undefined) return known
    const step = byId.get(id)
    if (step === undefined || seen.has(id) || step.attempts.length > 0 || step.expected === undefined) return undefined
    if (step.state !== 'pending' && step.state !== 'ready' && step.state !== 'waiting') return undefined
    const start = Math.max(model.now, ...step.after.map(dep => endOf(dep, new Set([...seen, id]))))
    const value = { start, end: start + (step.expected.lowMs + step.expected.highMs) / 2 }
    out.set(id, value)
    return value
  }
  for (const step of model.steps) project(step.id, new Set())
  return out
}

/** From the run's start to a little past now and every projected end. */
export const traceWindow = (model: MissionDetailModel, projected: ReadonlyMap<string, Projection>): { readonly t0: number; readonly t1: number } => {
  const t0 = model.startedAt
  const last = Math.max(model.now, ...[...projected.values()].map(each => each.end))
  return { t0, t1: last + Math.max(5 * MIN, (last - t0) * 0.02) }
}

const TICK_STEPS = [5 * MIN, 15 * MIN, 30 * MIN, HOUR, 2 * HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, DAY, 2 * DAY, 7 * DAY]
/** The finest round interval whose ticks sit at least `minGapPx` apart. */
export const tickEvery = (pxPerMs: number, minGapPx = 72): number =>
  TICK_STEPS.find(every => every * pxPerMs >= minGapPx) ?? TICK_STEPS.at(-1)!

/** Multiples of `every` inside [t0, t1], aligned to the local clock so hours land on the hour. */
export const ticksIn = (t0: number, t1: number, every: number): readonly number[] => {
  const offset = new Date(t0).getTimezoneOffset() * MIN
  const out: number[] = []
  for (let t = Math.ceil((t0 - offset) / every) * every + offset; t <= t1; t += every) if (t >= t0) out.push(t)
  return out
}

/** A span on the row in pixels, and the width its duration label needs. */
export interface SpanBox { readonly x0: number; readonly x1: number; readonly label: number }
export type LabelPlace = 'after' | 'inside' | 'before' | 'none'

const GAP = 4
/** Each span's duration goes right after it when the next span leaves room, else inside a wide enough span, else before it, else nowhere (the row's accessible name still says it). */
export const placeSpanLabels = (spans: readonly SpanBox[], limit: number): readonly LabelPlace[] =>
  spans.map((span, index) => {
    const next = spans[index + 1]?.x0 ?? limit + GAP
    const prev = index === 0 ? 0 : spans[index - 1]!.x1 + GAP
    if (span.x1 + GAP + span.label <= next - GAP) return 'after'
    if (span.x1 - span.x0 >= span.label + 2 * GAP) return 'inside'
    if (span.x0 - GAP - span.label >= prev) return 'before'
    return 'none'
  })

/** The playhead's range and its key steps. */
export interface ScrubBounds { readonly t0: number; readonly now: number; readonly step: number; readonly page: number }

/** A playhead time inside the run; at or past now it is now again (undefined). */
export const scrubTo = (t: number, bounds: ScrubBounds): number | undefined =>
  t >= bounds.now ? undefined : Math.max(bounds.t0, t)

/** What a slider key does to the playhead: arrows step, Page keys jump, Home and End go to the start and to now. */
export const scrubKey = (key: string, cursor: number | undefined, bounds: ScrubBounds): { readonly handled: boolean; readonly cursor: number | undefined } => {
  const at = cursor ?? bounds.now
  const delta = key === 'ArrowLeft' || key === 'ArrowDown' ? -bounds.step
    : key === 'ArrowRight' || key === 'ArrowUp' ? bounds.step
      : key === 'PageDown' ? -bounds.page
        : key === 'PageUp' ? bounds.page : undefined
  if (delta !== undefined) return { handled: true, cursor: scrubTo(at + delta, bounds) }
  if (key === 'Home') return { handled: true, cursor: bounds.t0 }
  if (key === 'End') return { handled: true, cursor: undefined }
  return { handled: false, cursor }
}

/** `Tue 14:05`: a moment on the trace, in the reader's time zone. */
export const clock = (at: number): string =>
  new Date(at).toLocaleString('en-GB', { weekday: 'short', hour: '2-digit', minute: '2-digit' })
