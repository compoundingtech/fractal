// A frozen stable hosted run supplies the anchor; policy adds exactly 10%.
export const KIT_COST_HEADROOM = 1.1
export const KIT_CALIBRATION_MAX_DRIFT = 0.25

export function kitPlayBudget(env) {
  const positive = name => {
    const raw = env[name]
    if (raw === undefined || raw === '') return null
    const value = Number(raw)
    if (!(Number.isFinite(value) && value > 0)) throw new Error(`${name} must be a positive finite number`)
    return value
  }
  return {
    costAnchor: positive('KIT_PLAY_COST_ANCHOR'),
    wallGuardSeconds: positive('KIT_PLAY_WALL_GUARD_SECONDS'),
  }
}

export function assessKitPlayCost({ costAnchor, wallGuardSeconds }, { elapsedSeconds, busyCoreMinutes, before, after, costRequired }) {
  const overWallGuard = wallGuardSeconds !== null && elapsedSeconds > wallGuardSeconds
  let calibration = null, normalizedCost = null, costStatus = 'not_required'
  if (before && after) {
    if (before.version !== after.version || before.cpuCount !== after.cpuCount
      || before.workload.memoryBytes !== after.workload.memoryBytes || before.workload.steps !== after.workload.steps) {
      throw new Error('Kit calibration workload changed between samples')
    }
    const left = before.all.wallMilliseconds, right = after.all.wallMilliseconds
    if (![left, right].every(value => Number.isFinite(value) && value > 0)) throw new Error('Kit calibration wall time must be positive and finite')
    const meanMilliseconds = (left + right) / 2
    const relativeDrift = Math.abs(left - right) / meanMilliseconds
    normalizedCost = busyCoreMinutes * 60 / (meanMilliseconds / 1000)
    calibration = { before, after, meanAllCpuMilliseconds: meanMilliseconds, relativeDrift }
    costStatus = relativeDrift > KIT_CALIBRATION_MAX_DRIFT ? 'unstable_vm'
      : costAnchor === null ? 'anchor_required'
        : normalizedCost > costAnchor * KIT_COST_HEADROOM ? 'over_budget' : 'within_budget'
  } else if (costRequired) {
    costStatus = 'calibration_missing'
  }
  const overCostBudget = ['over_budget', 'anchor_required', 'calibration_missing'].includes(costStatus)
  return {
    costAnchor, maxNormalizedCost: costAnchor === null ? null : costAnchor * KIT_COST_HEADROOM,
    costUnit: 'cpu_seconds_per_all_cpu_calibration_wall_second',
    costHeadroom: KIT_COST_HEADROOM, maxCalibrationDrift: KIT_CALIBRATION_MAX_DRIFT,
    normalizedCost, costStatus, calibration, wallGuardSeconds,
    overCostBudget, overWallGuard, overBudget: overCostBudget || overWallGuard,
  }
}
