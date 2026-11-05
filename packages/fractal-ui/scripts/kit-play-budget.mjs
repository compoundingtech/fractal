// Frozen hosted 171-play anchor, normalized so inventory growth needs no re-anchor.
export const KIT_CPU_ANCHOR_SECONDS_PER_PLAY = 413.1 / 171
// Twenty successful commit receipts: max 2.475682 CPU s/play; 15% covers it
// with 12.2% headroom while still rejecting twice the fastest observed cost.
export const KIT_CPU_MARGIN = 0.15

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

export function assessKitPlayCost({ wallGuardSeconds }, { elapsedSeconds, busyCoreMinutes, costRequired, plays }) {
  const cpuSeconds = roundSecondsToMilliseconds(busyCoreMinutes * 60)
  const cpuSecondsPerPlay = plays > 0 ? cpuSeconds / plays : null
  const cpuAnchorSecondsPerPlay = costRequired ? KIT_CPU_ANCHOR_SECONDS_PER_PLAY : null
  const maxCpuSecondsPerPlay = costRequired ? KIT_CPU_ANCHOR_SECONDS_PER_PLAY * (1 + KIT_CPU_MARGIN) : null
  const cpuAnchorSeconds = costRequired ? roundSecondsToMilliseconds(cpuAnchorSecondsPerPlay * plays) : null
  const maxCpuSeconds = costRequired ? roundSecondsToMilliseconds(maxCpuSecondsPerPlay * plays) : null
  const overCpuBudget = costRequired && cpuSecondsPerPlay !== null && cpuSecondsPerPlay > maxCpuSecondsPerPlay
  const overWallGuard = wallGuardSeconds !== null && elapsedSeconds > wallGuardSeconds
  return {
    cpuSeconds, cpuAnchorSeconds, maxCpuSeconds, wallGuardSeconds, maxSeconds: wallGuardSeconds,
    cpuSecondsPerPlay, cpuAnchorSecondsPerPlay, maxCpuSecondsPerPlay,
    cpuMargin: costRequired ? KIT_CPU_MARGIN : null,
    overCpuBudget, overWallGuard, overBudget: overCpuBudget || overWallGuard,
  }
}
