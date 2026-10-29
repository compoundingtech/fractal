// Frozen after #3982: six hosted 171-case commit samples, runs 40477033419–40482012045,
// measured 2026-10-27; maximum rounded up to 0.1 seconds. Never raise.
export const KIT_CPU_ANCHOR_SECONDS = 413.1
const CPU_HEADROOM = 1.1

function roundSecondsToMilliseconds(seconds) {
  return Math.round(seconds * 1000) / 1000
}

export function kitPlayBudget(env) {
  const raw = env.KIT_PLAY_WALL_GUARD_SECONDS
  const wallGuardSeconds = raw === undefined || raw === '' ? null : Number(raw)
  if (wallGuardSeconds !== null && !(Number.isFinite(wallGuardSeconds) && wallGuardSeconds > 0)) {
    throw new Error('KIT_PLAY_WALL_GUARD_SECONDS must be a positive finite number')
  }
  return { wallGuardSeconds }
}

export function assessKitPlayCost({ wallGuardSeconds }, { elapsedSeconds, busyCoreMinutes, costRequired }) {
  const cpuSeconds = roundSecondsToMilliseconds(busyCoreMinutes * 60)
  const cpuAnchorSeconds = costRequired ? KIT_CPU_ANCHOR_SECONDS : null
  const maxCpuSeconds = cpuAnchorSeconds === null ? null : roundSecondsToMilliseconds(cpuAnchorSeconds * CPU_HEADROOM)
  const overCpuBudget = maxCpuSeconds !== null && cpuSeconds > maxCpuSeconds
  const overWallGuard = wallGuardSeconds !== null && elapsedSeconds > wallGuardSeconds
  return {
    cpuSeconds, cpuAnchorSeconds, maxCpuSeconds, wallGuardSeconds, maxSeconds: wallGuardSeconds,
    overCpuBudget, overWallGuard, overBudget: overCpuBudget || overWallGuard,
  }
}
