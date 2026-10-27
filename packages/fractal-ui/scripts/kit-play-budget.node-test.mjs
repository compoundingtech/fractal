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
  assert.equal(KIT_CPU_ANCHOR_SECONDS, 413.9)
  const limit = KIT_CPU_ANCHOR_SECONDS * 1.1
  assert.equal(result(limit).maxCpuSeconds, limit)
  assert.equal(result(limit).overCpuBudget, false)
  assert.equal(result(limit + 0.01).overCpuBudget, true)
})

test('all measured 162-case runs pass, while the 187-case pre-fix regression fails', () => {
  for (const cpuSeconds of [348.77, 423.31, 413.93, 404.91, 416.91]) {
    assert.equal(result(cpuSeconds).overBudget, false, `${cpuSeconds} CPU seconds`)
  }
  assert.equal(result(469.55).overCpuBudget, true)
})

test('wall ceiling is independent of CPU cost and remains only a hang guard', () => {
  assert.equal(result(100, { elapsedSeconds: 268 }).overWallGuard, false)
  assert.equal(result(100, { elapsedSeconds: 268.01 }).overBudget, true)
  assert.equal(result(456, { elapsedSeconds: 20 }).overBudget, true)
})

test('non-commit runs retain CPU telemetry without enforcing the commit CPU policy', () => {
  const full = result(1000, { costRequired: false })
  assert.equal(full.cpuSeconds, 1000)
  assert.equal(full.cpuAnchorSeconds, null)
  assert.equal(full.maxCpuSeconds, null)
  assert.equal(full.overCpuBudget, false)
  assert.equal(full.overBudget, false)
})
