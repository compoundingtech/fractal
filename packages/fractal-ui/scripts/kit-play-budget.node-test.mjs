import { test } from 'node:test'
import assert from 'node:assert/strict'
import { assessKitPlayCost, kitPlayBudget } from './kit-play-budget.mjs'

const sample = wallMilliseconds => ({ version: 1, cpuCount: 4, workload: { memoryBytes: 33554432, steps: 33554432 }, all: { wallMilliseconds } })
const budget = { costAnchor: 100, wallGuardSeconds: 268 }
const result = (overrides = {}, policy = budget) => assessKitPlayCost(policy, {
  elapsedSeconds: 89, busyCoreMinutes: 5, before: sample(3000), after: sample(3000), costRequired: true, ...overrides,
})

test('cost and wall settings accept empty values and reject malformed limits', () => {
  assert.deepEqual(kitPlayBudget({}), { costAnchor: null, wallGuardSeconds: null })
  assert.deepEqual(kitPlayBudget({ KIT_PLAY_COST_ANCHOR: '', KIT_PLAY_WALL_GUARD_SECONDS: '' }), { costAnchor: null, wallGuardSeconds: null })
  assert.deepEqual(kitPlayBudget({ KIT_PLAY_COST_ANCHOR: '100', KIT_PLAY_WALL_GUARD_SECONDS: '268' }), budget)
  for (const name of ['KIT_PLAY_COST_ANCHOR', 'KIT_PLAY_WALL_GUARD_SECONDS']) {
    for (const value of ['0', '-1', 'no', 'Infinity', 'NaN']) assert.throws(() => kitPlayBudget({ [name]: value }), /positive finite/)
  }
})

test('mean all-CPU calibration normalizes CPU cost without using wall time as the regression signal', () => {
  const quiet = result()
  const slow = result({ elapsedSeconds: 120, busyCoreMinutes: 6, before: sample(3600), after: sample(3600) })
  assert.equal(quiet.normalizedCost, 100)
  assert.equal(slow.normalizedCost, quiet.normalizedCost)
  assert.equal(slow.costStatus, 'within_budget')
  assert.equal(slow.overBudget, false)
  assert.equal(result({ before: sample(2800), after: sample(3200) }).calibration.meanAllCpuMilliseconds, 3000)
})

test('the frozen anchor admits exactly ten percent cost headroom, not a tuned wall allowance', () => {
  assert.equal(result({ busyCoreMinutes: 5.5 }).overCostBudget, false)
  assert.equal(result({ busyCoreMinutes: 5.50001 }).costStatus, 'over_budget')
  assert.equal(result({ elapsedSeconds: 20, busyCoreMinutes: 6 }).overBudget, true)
})

test('unstable VM skips only the cost check and keeps the wall ceiling', () => {
  const unstable = result({ before: sample(2000), after: sample(4000), busyCoreMinutes: 20 })
  assert.equal(unstable.costStatus, 'unstable_vm')
  assert.equal(unstable.overCostBudget, false)
  assert.equal(unstable.overBudget, false)
  const hung = result({ before: sample(2000), after: sample(4000), elapsedSeconds: 269 })
  assert.equal(hung.overWallGuard, true)
  assert.equal(hung.overBudget, true)
  assert.equal(result({ before: sample(2625), after: sample(3375) }).costStatus, 'within_budget', 'exactly 25% drift is stable')
})

test('wall guard boundary is independent of CPU cost', () => {
  assert.equal(result({ elapsedSeconds: 268 }).overWallGuard, false)
  assert.equal(result({ elapsedSeconds: 268.01, busyCoreMinutes: 0.01 }).overWallGuard, true)
})

test('commit runs fail closed until a stable anchor and both calibration samples exist', () => {
  assert.equal(result({}, { ...budget, costAnchor: null }).costStatus, 'anchor_required')
  assert.equal(result({ before: undefined }).costStatus, 'calibration_missing')
  assert.equal(result({ before: undefined }).overBudget, true)
  assert.equal(result({ before: undefined, after: undefined, costRequired: false }).overBudget, false)
  assert.throws(() => result({ after: { ...sample(3000), cpuCount: 2 } }), /workload changed/)
  assert.throws(() => result({ after: sample(0) }), /positive and finite/)
})
