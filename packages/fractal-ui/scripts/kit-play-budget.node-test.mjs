import { test } from 'node:test'
import assert from 'node:assert/strict'
import { assessKitPlayCost, kitPlayBudget, KIT_CPU_ANCHOR_SECONDS } from './kit-play-budget.mjs'

const policy = { wallGuardSeconds: 268 }
const result = (cpuSeconds, overrides = {}) => assessKitPlayCost(policy, {
  elapsedSeconds: 106, busyCoreMinutes: cpuSeconds / 60, costRequired: true, ...overrides,
})

test('wall setting accepts absent/empty values and rejects malformed limits', () => {
  assert.deepEqual(kitPlayBudget({}), { wallGuardSeconds: null })
  assert.deepEqual(kitPlayBudget({ KIT_PLAY_WALL_GUARD_SECONDS: '' }), { wallGuardSeconds: null })
  assert.deepEqual(kitPlayBudget({ KIT_PLAY_WALL_GUARD_SECONDS: '268' }), policy)
  for (const value of ['0', '-1', 'no', 'Infinity', 'NaN']) {
    assert.throws(() => kitPlayBudget({ KIT_PLAY_WALL_GUARD_SECONDS: value }), /positive finite/)
  }
})

test('single frozen CPU anchor derives exactly ten percent headroom', () => {
  assert.equal(KIT_CPU_ANCHOR_SECONDS, 413.1)
  const limit = Math.round(KIT_CPU_ANCHOR_SECONDS * 1.1 * 1000) / 1000
  assert.equal(result(limit).maxCpuSeconds, limit)
  assert.equal(result(limit).overCpuBudget, false)
  assert.equal(result(limit + 0.01).overCpuBudget, true)
})

test('all measured post-3982 171-case runs pass, while the 187-case pre-fix regression fails', () => {
  for (const cpuSeconds of [413.1, 379.94, 367.74, 381.97, 396.26, 394.16]) {
    assert.equal(result(cpuSeconds).overBudget, false, `${cpuSeconds} CPU seconds`)
  }
  assert.equal(result(469.55).overCpuBudget, true)
})

test('wall ceiling is independent of CPU cost and remains only a hang guard', () => {
  assert.equal(result(100, { elapsedSeconds: 268 }).overWallGuard, false)
  assert.equal(result(100, { elapsedSeconds: 268.01 }).overBudget, true)
  assert.equal(result(456, { elapsedSeconds: 20 }).overBudget, true)
})

test('compatible maxSeconds remains the wall guard and overBudget combines both checks', () => {
  for (const cpuSeconds of [100, 456]) {
    for (const elapsedSeconds of [100, 269]) {
      const receipt = result(cpuSeconds, { elapsedSeconds })
      assert.equal(receipt.maxSeconds, receipt.wallGuardSeconds)
      assert.equal(receipt.maxSeconds, 268)
      assert.equal(receipt.overBudget, receipt.overCpuBudget || receipt.overWallGuard)
    }
  }
  const unset = assessKitPlayCost(kitPlayBudget({}), { elapsedSeconds: 100, busyCoreMinutes: 1, costRequired: false })
  assert.equal(unset.maxSeconds, null)
  assert.equal(unset.overBudget, false)
})

test('non-commit runs retain CPU telemetry without enforcing the commit CPU policy', () => {
  const full = result(1000, { costRequired: false })
  assert.equal(full.cpuSeconds, 1000)
  assert.equal(full.cpuAnchorSeconds, null)
  assert.equal(full.maxCpuSeconds, null)
  assert.equal(full.overCpuBudget, false)
  assert.equal(full.overBudget, false)
})
