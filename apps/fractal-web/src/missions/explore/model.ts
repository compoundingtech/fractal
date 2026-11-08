// EXPLORATION ONLY. The shape the richer mission views want, with every field the daemon does not serve today marked
// `NOT SERVED`. Example data lives in `example.ts`; nothing here is wired to a live feed.

export type StepState = 'done' | 'running' | 'failed' | 'waiting-you' | 'waiting' | 'ready' | 'pending' | 'cancelled'

/** One execution of a step. NOT SERVED: the daemon serves only the selected attempt's `started_at`/`completed_at`. */
export interface Attempt {
  readonly n: number
  readonly agent: string
  readonly start: number
  /** Absent while it runs. */
  readonly end?: number
  readonly outcome: 'done' | 'failed' | 'running' | 'cancelled'
  /** Why it failed or was retried. NOT SERVED (only the run outcome carries a reason). */
  readonly reason?: string
  /** Conversation turn that ran it. NOT SERVED. */
  readonly turn?: string
}

/** Time a step spent not running. NOT SERVED: only the current `blocked_reason`/`since` are. */
export interface Wait {
  readonly kind: 'you' | 'dependency' | 'queue'
  readonly start: number
  readonly end?: number
  readonly reason: string
}

export interface PlanStep {
  readonly id: string
  readonly title: string
  readonly state: StepState
  /** Paths this step waits for (served as visualization edges / step blockers). */
  readonly after: readonly string[]
  readonly attempts: readonly Attempt[]
  readonly waits: readonly Wait[]
  /** Typical duration from earlier runs. NOT SERVED per step (the run ETA aggregates it). */
  readonly expectedMs?: number
  /** Revision that introduced this step. NOT SERVED (derivable by diffing declarations). */
  readonly since: number
}

export interface PlanChange {
  readonly kind: 'added' | 'removed' | 'rewritten'
  readonly step: string
  readonly before?: string
  readonly after?: string
}

/** A published plan revision. Served: revision id and KDL per revision (missionDeclarationGet). NOT SERVED: the diff. */
export interface PlanRevision {
  readonly n: number
  readonly at: number
  readonly by: string
  readonly why: string
  readonly changes: readonly PlanChange[]
}

export interface Problem {
  readonly at: number
  readonly kind: 'failure' | 'retry' | 'wait-you' | 'stall'
  readonly step: string
  readonly text: string
  readonly agent?: string
  /** NOT SERVED: the conversation turn it links to. */
  readonly turn?: string
  readonly resolvedAt?: number
}

export interface ExploreMission {
  readonly id: string
  readonly title: string
  readonly startedAt: number
  readonly now: number
  /** Expected total, from the served run ETA plus elapsed. */
  readonly expectedMs: number
  readonly steps: readonly PlanStep[]
  readonly revisions: readonly PlanRevision[]
  readonly problems: readonly Problem[]
}

export const stepEnd = (step: PlanStep, now: number): number | undefined => {
  const ends = [...step.attempts.map(a => a.end ?? now), ...step.waits.map(w => w.end ?? now)]
  return ends.length === 0 ? undefined : Math.max(...ends)
}
export const stepStart = (step: PlanStep): number | undefined => {
  const starts = [...step.attempts.map(a => a.start), ...step.waits.map(w => w.start)]
  return starts.length === 0 ? undefined : Math.min(...starts)
}
export const runningMs = (step: PlanStep, now: number) => step.attempts.reduce((sum, a) => sum + ((a.end ?? now) - a.start), 0)

export const progressOf = (mission: ExploreMission) => {
  const live = mission.steps.filter(step => step.state !== 'cancelled')
  return { done: live.filter(step => step.state === 'done').length, total: live.length }
}

/** The longest chain of dependencies by expected-or-actual duration: what decides when the mission ends. */
export const criticalPath = (mission: ExploreMission): ReadonlySet<string> => {
  const steps = new Map(mission.steps.filter(s => s.state !== 'cancelled').map(s => [s.id, s]))
  const weight = (s: PlanStep) => Math.max(runningMs(s, mission.now), s.state === 'done' ? 0 : s.expectedMs ?? 0)
  const best = new Map<string, { total: number; prev?: string }>()
  const visit = (id: string): { total: number; prev?: string } => {
    const known = best.get(id)
    if (known !== undefined) return known
    const step = steps.get(id)!
    const deps = step.after.filter(dep => steps.has(dep)).map(dep => ({ dep, total: visit(dep).total }))
    const top = deps.sort((a, b) => b.total - a.total)[0]
    const value = top === undefined ? { total: weight(step) } : { total: top.total + weight(step), prev: top.dep }
    best.set(id, value)
    return value
  }
  let end: string | undefined
  for (const id of steps.keys()) if (end === undefined || visit(id).total > visit(end).total) end = id
  const path = new Set<string>()
  for (let at = end; at !== undefined; at = best.get(at)?.prev) path.add(at)
  return path
}

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR
/** `42m`, `3h 10m`, `1d 4h`. */
export const span = (ms: number): string => {
  if (ms < MIN) return '<1m'
  if (ms < HOUR) return `${Math.round(ms / MIN)}m`
  if (ms < DAY) { const h = Math.floor(ms / HOUR); const m = Math.round((ms - h * HOUR) / MIN); return m === 0 ? `${h}h` : `${h}h ${m}m` }
  const d = Math.floor(ms / DAY); const h = Math.round((ms - d * DAY) / HOUR); return h === 0 ? `${d}d` : `${d}d ${h}h`
}
export const clock = (at: number): string => new Date(at).toLocaleString('en-GB', { weekday: 'short', hour: '2-digit', minute: '2-digit', timeZone: 'UTC' })
