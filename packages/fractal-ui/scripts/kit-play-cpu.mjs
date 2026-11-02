import { readFile } from 'node:fs/promises'
import { cpus } from 'node:os'

const PROC_STAT_FIELDS = ['user', 'nice', 'system', 'idle', 'iowait', 'irq', 'softirq', 'steal']

function counter(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${label} must be a nonnegative safe integer`)
  }
  return value
}

// /proc/stat counters are raw USER_HZ ticks, not milliseconds. Guest time is
// already included in user/nice, so never add guest columns to the totals.
export function parseProcStat(text) {
  if (typeof text !== 'string') throw new Error('/proc/stat must be text')
  const line = text.split('\n').find((line) => /^cpu[\t ]/.test(line))
  if (!line) throw new Error('/proc/stat is missing the aggregate cpu line')
  const columns = line.trim().split(/\s+/).slice(1)
  if (columns.length < 8 || columns.length > 10) {
    throw new Error('/proc/stat aggregate cpu line must contain 8 to 10 counters')
  }
  const values = columns.map((column, index) => {
    if (!/^\d+$/.test(column)) throw new Error(`/proc/stat cpu counter ${index} is not an unsigned integer`)
    return counter(Number(column), `/proc/stat cpu counter ${index}`)
  })
  return Object.fromEntries(PROC_STAT_FIELDS.map((field, index) => [field, values[index]]))
}

export async function captureKitCpu() {
  const busyMilliseconds = cpus().reduce((sum, { times }) => {
    for (const field of ['user', 'nice', 'sys', 'irq']) {
      sum = counter(sum + counter(times[field], `os.cpus ${field} milliseconds`), 'CPU busy milliseconds')
    }
    return sum
  }, 0)
  // Only non-Linux hosts may omit procfs evidence. Linux I/O and parse errors
  // propagate instead of being reported as zero steal or unavailable data.
  const procStat = process.platform === 'linux'
    ? parseProcStat(await readFile('/proc/stat', 'utf8'))
    : null
  return { busyMilliseconds, procStat }
}

export function kitCpuDelta(before, after) {
  const busyMilliseconds = counter(after.busyMilliseconds, 'Ending CPU busy milliseconds')
    - counter(before.busyMilliseconds, 'Starting CPU busy milliseconds')
  counter(busyMilliseconds, 'CPU busy milliseconds delta')

  // Validate every present snapshot, even if the other endpoint is unavailable.
  for (const snapshot of [before.procStat, after.procStat]) {
    if (snapshot === null) continue
    for (const field of PROC_STAT_FIELDS) counter(snapshot?.[field], `/proc/stat ${field} ticks`)
  }
  if (before.procStat === null || after.procStat === null) {
    return { busyCoreMinutes: busyMilliseconds / 60000, procStat: { available: false } }
  }
  const ticks = {}
  let totalTicks = 0
  for (const field of PROC_STAT_FIELDS) {
    ticks[field] = counter(after.procStat[field] - before.procStat[field], `/proc/stat ${field} ticks delta`)
    totalTicks = counter(totalTicks + ticks[field], '/proc/stat total ticks delta')
  }
  return {
    busyCoreMinutes: busyMilliseconds / 60000,
    procStat: {
      available: true,
      ticks,
      stealTicks: ticks.steal,
      stealFraction: totalTicks === 0 ? 0 : ticks.steal / totalTicks,
    },
  }
}
