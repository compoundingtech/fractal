import { describe, expect, it } from 'vitest'

import { missionDetailModel } from './detailModel.ts'
import { syncAttention, syncMission, syncNow, syncStart, syncStep, syncUsageRows, withServedHistory } from './fixtures.ts'
import { placeSpanLabels, projectedSpans, scrubKey, scrubTo, tickEvery, ticksIn, traceWindow } from './traceScale.ts'

const MIN = 60_000
const HOUR = 60 * MIN
const served = missionDetailModel({ mission: withServedHistory(syncMission), agents: [], attention: syncAttention, usage: syncUsageRows, now: syncNow })
const coarse = missionDetailModel({ mission: syncMission, agents: [], attention: syncAttention, usage: syncUsageRows, now: syncNow })

describe('trace scale: what the wall clock shows', () => {
  it('projects a step not yet started from its typical duration, after its dependencies', () => {
    // The canary waits on steps still open, so it starts no earlier than now and runs its typical two hours.
    const canary = projectedSpans(served).get(syncStep('canary'))
    expect(canary).toEqual({ start: syncNow, end: syncNow + 120 * MIN })
    // Without a typical duration nothing is invented.
    expect(projectedSpans(served).has(syncStep('rollout'))).toBe(false)
    expect(projectedSpans(coarse).size).toBe(0)
  })
  it('spans from the run start past now and every projected end', () => {
    const window = traceWindow(served, projectedSpans(served))
    expect(window.t0).toBe(syncStart)
    expect(window.t1).toBeGreaterThan(syncNow + 120 * MIN)
    const plain = traceWindow(coarse, projectedSpans(coarse))
    expect(plain.t1).toBeGreaterThan(syncNow)
    expect(plain.t1 - syncNow).toBeLessThan(2 * HOUR)
  })
  it('spaces ticks at least the minimum gap apart, on round clock times', () => {
    // 30 px an hour: an hour is too tight for 72 px labels, three hours is the first step that fits.
    expect(tickEvery(30 / HOUR)).toBe(3 * HOUR)
    expect(tickEvery(600 / HOUR)).toBe(15 * MIN)
    const ticks = ticksIn(syncStart + 10 * MIN, syncStart + 4 * HOUR, HOUR)
    expect(ticks.every(t => new Date(t).getMinutes() === 0 && new Date(t).getSeconds() === 0)).toBe(true)
    expect(ticks[0]).toBeGreaterThanOrEqual(syncStart + 10 * MIN)
  })
})

describe('trace scale: a duration beside every span', () => {
  it('puts the duration after the span when the gap to the next allows it', () => {
    expect(placeSpanLabels([{ x0: 0, x1: 100, label: 40 }, { x0: 200, x1: 260, label: 30 }], 400)).toEqual(['after', 'after'])
  })
  it('moves it inside a wide span when the next span is too close, and drops it when nothing fits', () => {
    expect(placeSpanLabels([{ x0: 0, x1: 100, label: 40 }, { x0: 110, x1: 120, label: 30 }, { x0: 125, x1: 400, label: 30 }], 600)).toEqual(['inside', 'none', 'after'])
  })
  it('puts it before the span at the right edge', () => {
    expect(placeSpanLabels([{ x0: 300, x1: 395, label: 40 }], 400)).toEqual(['inside'])
    expect(placeSpanLabels([{ x0: 300, x1: 330, label: 40 }], 340)).toEqual(['before'])
  })
})

describe('trace scale: the playhead', () => {
  const bounds = { t0: syncStart, now: syncNow, step: 15 * MIN, page: 3 * HOUR }
  it('scrubs inside the run and snaps to now at the end', () => {
    expect(scrubTo(syncStart - HOUR, bounds)).toBe(syncStart)
    expect(scrubTo(syncStart + HOUR, bounds)).toBe(syncStart + HOUR)
    expect(scrubTo(syncNow + HOUR, bounds)).toBeUndefined()
    expect(scrubTo(syncNow, bounds)).toBeUndefined()
  })
  it('moves with the keys a slider takes', () => {
    expect(scrubKey('ArrowLeft', undefined, bounds)).toEqual({ handled: true, cursor: syncNow - 15 * MIN })
    expect(scrubKey('ArrowRight', syncNow - 5 * MIN, bounds)).toEqual({ handled: true, cursor: undefined })
    expect(scrubKey('PageDown', syncStart + 4 * HOUR, bounds)).toEqual({ handled: true, cursor: syncStart + HOUR })
    expect(scrubKey('Home', undefined, bounds)).toEqual({ handled: true, cursor: syncStart })
    expect(scrubKey('End', syncStart, bounds)).toEqual({ handled: true, cursor: undefined })
    expect(scrubKey('a', syncStart, bounds)).toEqual({ handled: false, cursor: syncStart })
  })
})
