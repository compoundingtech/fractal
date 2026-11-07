import assert from 'node:assert/strict'
import { test } from 'node:test'
import { idleFramePeriod, transcriptFrameBudget } from './transcript-frame-budget.ts'

const idle = Array(24).fill(16.7)
const period = idleFramePeriod(idle)

test('an isolated four-period spike is long but does not raise p95', () => {
  const result = transcriptFrameBudget([...idle.slice(1), 66.7], period)
  assert.equal(result.longFrames, 1)
  assert.equal(result.p95Periods, 1)
  assert.ok(result.maxPeriods > 3.5)
})

test('sustained slowdown cannot recalibrate away four-period frames', () => {
  const result = transcriptFrameBudget(Array(24).fill(66.7), period)
  assert.equal(result.periodMs, 16.7)
  assert.equal(result.longFrames, 24)
  assert.equal(result.p95Periods, 4)
  assert.ok(result.maxPeriods > 3.5)
})

test('quantized three-period boundary tolerates timestamp jitter without becoming a long frame', () => {
  for (const milliseconds of [50, 50.1]) {
    const result = transcriptFrameBudget(Array(24).fill(milliseconds), period)
    assert.equal(result.longFrames, 0)
    assert.equal(result.p95Periods, 3)
  }
  assert.equal(transcriptFrameBudget(Array(24).fill(3.5 * period), period).longFrames, 0)
  assert.equal(transcriptFrameBudget(Array(24).fill(3.5 * period + 0.01), period).longFrames, 24)
})
