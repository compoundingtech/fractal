/** Calibrate from an idle control bracket, never from the work being evaluated. */
export const idleFramePeriod = (intervals: readonly number[]): number => {
  const sorted = [...intervals].sort((a, b) => a - b)
  return (sorted[Math.floor((sorted.length - 1) / 2)]! + sorted[Math.floor(sorted.length / 2)]!) / 2
}

export const transcriptFrameBudget = (intervals: readonly number[], periodMs: number) => {
  const sorted = [...intervals].sort((a, b) => a - b)
  const max = sorted.at(-1)!
  const p95Ms = sorted[Math.ceil(sorted.length * 0.95) - 1]!
  // rAF timestamps quantize to vsync: 50.0/50.1ms can both be three 60Hz periods.
  // Idle calibration keeps sustained slow work from increasing its own budget.
  return {
    periodMs,
    max,
    maxPeriods: max / periodMs,
    p95Ms,
    p95Periods: Math.round(p95Ms / periodMs),
    longFrames: intervals.filter(duration => duration > 3.5 * periodMs).length,
  }
}
