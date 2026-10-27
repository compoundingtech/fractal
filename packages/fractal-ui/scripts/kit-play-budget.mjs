// Frozen hosted 162-case measurement; lower once after #3982, never raise.
export const KIT_CPU_ANCHOR_SECONDS = 413.9
const CPU_HEADROOM = 1.1

export function kitPlayBudget(env) {
  const raw = env.KIT_PLAY_WALL_GUARD_SECONDS
  const wallGuardSeconds = raw === undefined || raw === '' ? null : Number(raw)
  if (wallGuardSeconds !== null && !(Number.isFinite(wallGuardSeconds) && wallGuardSeconds > 0)) {
    throw new Error('KIT_PLAY_WALL_GUARD_SECONDS must be a positive finite number')
  }
  return { wallGuardSeconds }
}

export function assessKitPlayCost({ wallGuardSeconds }, { elapsedSeconds, busyCoreMinutes, costRequired }) {
  const cpuSeconds = busyCoreMinutes * 60
  const cpuAnchorSeconds = costRequired ? KIT_CPU_ANCHOR_SECONDS : null
  const maxCpuSeconds = cpuAnchorSeconds === null ? null : cpuAnchorSeconds * CPU_HEADROOM
  const overCpuBudget = maxCpuSeconds !== null && cpuSeconds > maxCpuSeconds
  const overWallGuard = wallGuardSeconds !== null && elapsedSeconds > wallGuardSeconds
  return {
    cpuSeconds, cpuAnchorSeconds, maxCpuSeconds, wallGuardSeconds, maxSeconds: wallGuardSeconds,
    overCpuBudget, overWallGuard, overBudget: overCpuBudget || overWallGuard,
  }
}
