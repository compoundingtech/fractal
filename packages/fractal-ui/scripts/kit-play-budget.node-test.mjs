import { test } from 'node:test'
import assert from 'node:assert/strict'
import { assessKitPlayCost, kitPlayBudget, KIT_CPU_ANCHOR_SECONDS_PER_PLAY, KIT_CPU_MARGIN } from './kit-play-budget.mjs'

const policy = { wallGuardSeconds: 268 }
const result = (cpuSeconds, overrides = {}) => assessKitPlayCost(policy, {
  elapsedSeconds: 106, busyCoreMinutes: cpuSeconds / 60, costRequired: true, plays: 176, ...overrides,
})

test('wall setting accepts absent/empty values and rejects malformed limits', () => {
  assert.deepEqual(kitPlayBudget({}), { wallGuardSeconds: null })
  assert.deepEqual(kitPlayBudget({ KIT_PLAY_WALL_GUARD_SECONDS: '' }), { wallGuardSeconds: null })
  assert.deepEqual(kitPlayBudget({ KIT_PLAY_WALL_GUARD_SECONDS: '268' }), policy)
  for (const value of ['0', '-1', 'no', 'Infinity', 'NaN']) {
    assert.throws(() => kitPlayBudget({ KIT_PLAY_WALL_GUARD_SECONDS: value }), /positive finite/)
  }
})

test('frozen per-play anchor derives the measured fifteen percent margin', () => {
  assert.equal(KIT_CPU_ANCHOR_SECONDS_PER_PLAY, 413.1 / 171)
  assert.equal(KIT_CPU_MARGIN, 0.15)
  const limit = KIT_CPU_ANCHOR_SECONDS_PER_PLAY * 1.15
  assert.equal(result(400).maxCpuSecondsPerPlay, limit)
  // Compare at millisecond precision in total CPU without rounding the per-play limit.
  const totalLimit = Math.floor(limit * 176 * 1000) / 1000
  assert.equal(result(totalLimit).overCpuBudget, false)
  assert.equal(result(totalLimit + 0.001).overCpuBudget, true)
})

test('adding plays at the same per-play cost does not require a new anchor', () => {
  for (const plays of [171, 176, 180, 352]) {
    const receipt = result(2.6 * plays, { plays })
    assert.equal(receipt.cpuSecondsPerPlay, 2.6)
    assert.equal(receipt.overBudget, false)
    assert.equal(result(3 * plays, { plays }).overCpuBudget, true)
  }
})

test('twenty successful hosted samples and the unrelated failing PR pass; a real 2x regression fails', () => {
  for (const [cpuSeconds, plays] of [
    [404.07, 176], [435.72, 176], [297.57, 176], [314.56, 176], [427.25, 176],
    [311.53, 176], [426.07, 176], [268.37, 176], [326.07, 176], [287.69, 176],
    [417.48, 180], [395.02, 176], [310.91, 176], [426.21, 176], [429.81, 176],
    [402.89, 176], [248.02, 171], [424.18, 176], [303.84, 176], [406.54, 176],
    [459.18, 176],
  ]) {
    assert.equal(result(cpuSeconds, { plays }).overBudget, false, `${cpuSeconds} CPU seconds / ${plays} plays`)
    assert.equal(result(cpuSeconds * 2, { plays }).overCpuBudget, true)
  }
})

test('no completed plays reports no per-play measurement, not Infinity or NaN', () => {
  assert.equal(result(10, { plays: 0 }).cpuSecondsPerPlay, null)
})

test('wall ceiling is independent of CPU cost and remains only a hang guard', () => {
  assert.equal(result(100, { elapsedSeconds: 268 }).overWallGuard, false)
  assert.equal(result(100, { elapsedSeconds: 268.01 }).overBudget, true)
  assert.equal(result(600, { elapsedSeconds: 20 }).overBudget, true)
})

test('compatible maxSeconds remains the wall guard and overBudget combines both checks', () => {
  for (const cpuSeconds of [100, 600]) {
    for (const elapsedSeconds of [100, 269]) {
      const receipt = result(cpuSeconds, { elapsedSeconds })
      assert.equal(receipt.maxSeconds, receipt.wallGuardSeconds)
      assert.equal(receipt.maxSeconds, 268)
      assert.equal(receipt.overBudget, receipt.overCpuBudget || receipt.overWallGuard)
    }
  }
  const unset = assessKitPlayCost(kitPlayBudget({}), { elapsedSeconds: 100, busyCoreMinutes: 1, costRequired: false, plays: 20 })
  assert.equal(unset.maxSeconds, null)
  assert.equal(unset.overBudget, false)
})

test('non-commit runs retain CPU telemetry without enforcing the commit CPU policy', () => {
  const full = result(1000, { costRequired: false })
  assert.equal(full.cpuSeconds, 1000)
  assert.equal(full.cpuAnchorSeconds, null)
  assert.equal(full.maxCpuSeconds, null)
  assert.equal(full.cpuSecondsPerPlay, 1000 / 176)
  assert.equal(full.cpuAnchorSecondsPerPlay, null)
  assert.equal(full.maxCpuSecondsPerPlay, null)
  assert.equal(full.overCpuBudget, false)
  assert.equal(full.overBudget, false)
})
