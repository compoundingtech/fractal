import type * as Fractal from '@fractal/client/schema'
import type { DecodedAgent } from '@fractal/sdk/read'
import type { MarkdownInline } from '@fractal/client/markdown'
import { Option } from 'effect'

import { leafText } from '../../data/projections.ts'
import { millis, missionPlan } from '../missionPlan.ts'
import { missionReaderTitle, presentMission, type MissionPresentation, type StepWord, type Who } from '../presentation.ts'

// The mission detail's one model: what the overview, graph, trace, lists and inspector show, derived from what Fractal
// serves. Today that is coarse: each step's selected attempt (`started_at`, `completed_at`, `attempt`, `since`, state)
// and its current wait. A newer daemon's per-step history (`attempts`, `waits`, `eta`, `history_complete`) is read
// loosely and preferred where present. Spend sums `usage.period` rows by step and by run.

export type StepState = 'done' | 'running' | 'failed' | 'waiting-you' | 'waiting' | 'ready' | 'pending' | 'cancelled'

/** One execution of a step. */
export interface Attempt {
  readonly n: number
  readonly agent: string | undefined
  readonly start: number
  /** Absent while it runs. */
  readonly end: number | undefined
  readonly outcome: 'done' | 'failed' | 'running' | 'cancelled'
  readonly reason: string | undefined
}

export type WaitKind = 'you' | 'dependency' | 'queue' | 'paused' | 'retry' | 'other'

/** Time a step spent not running. */
export interface Wait {
  readonly kind: WaitKind
  readonly start: number
  /** Absent while it waits. */
  readonly end: number | undefined
  readonly reason: string | undefined
}

export interface Spend {
  readonly costUsd: number
  readonly tokens: number
}

/** What spend sums read of a `usage.period` row. */
export interface UsageRowLike {
  readonly step?: string | undefined
  readonly mission_run?: string | undefined
  readonly cost_microusd: number
  readonly total_tokens: number
}

export interface DetailStep {
  readonly id: string
  readonly path: string
  readonly title: string
  readonly state: StepState
  readonly word: StepWord
  /** Ids of the steps this one waits for. */
  readonly after: readonly string[]
  readonly owner: Who
  /** The seat that holds or last held it. */
  readonly agent: string | undefined
  readonly attempts: readonly Attempt[]
  /** The attempt number Fractal counts, which can exceed the attempts known. */
  readonly attemptCount: number
  readonly waits: readonly Wait[]
  /** `served`: the daemon's per-step history; `coarse`: the selected attempt and the current wait only. */
  readonly history: 'served' | 'coarse'
  /** False when earlier attempts or waits are not known. */
  readonly historyComplete: boolean
  /** Typical duration, from the daemon's per-step samples. */
  readonly expected: { readonly lowMs: number; readonly highMs: number } | undefined
  /** Undefined when no usage row names the step (unknown, not zero). */
  readonly spend: Spend | undefined
  readonly since: number | undefined
  readonly goal: MarkdownInline | undefined
  readonly progress: MarkdownInline | undefined
  readonly blockedReason: string | undefined
  readonly note: string | null
}

export interface DetailRun {
  readonly id: string
  readonly status: string
  readonly startedAt: number | undefined
  readonly settled: boolean
  /** Served remaining wall time, from `as_of`. */
  readonly eta: { readonly lowMs: number; readonly highMs: number; readonly asOf: number } | undefined
}

export type AgentLiveState = 'working' | 'idle' | 'needs-you' | 'failed' | 'stopped' | 'unknown'

export interface DetailAgent {
  readonly id: string
  readonly label: string
  readonly state: AgentLiveState
  /** The step it holds now, else its latest. */
  readonly step: string | undefined
}

export interface Problem {
  readonly at: number
  readonly kind: 'failure' | 'retry' | 'wait-you' | 'stall'
  readonly step: string
  readonly text: string
  readonly resolvedAt: number | undefined
}

export interface MissionDetailModel {
  readonly id: string
  readonly title: string
  readonly now: number
  readonly run: DetailRun | undefined
  /** The run's start, else its first step's. */
  readonly startedAt: number
  /** Live steps in plan order (layer, then declaration), cancelled last. */
  readonly steps: readonly DetailStep[]
  readonly progress: { readonly done: number; readonly total: number }
  readonly running: readonly string[]
  readonly needsYou: readonly string[]
  readonly failed: readonly string[]
  readonly blocked: readonly string[]
  readonly next: string | undefined
  /** Undefined when spend is unknown. */
  readonly spend: Spend | undefined
  readonly agents: readonly DetailAgent[]
  readonly problems: readonly Problem[]
  readonly presentation: MissionPresentation
}

const FINISHED = new Set(['completed', 'failed', 'cancelled'])
const ACTIVE = new Set(['claimed', 'working', 'running', 'verifying'])

const text = (value: unknown): string | undefined => {
  if (value === null || value === undefined) return undefined
  if (Option.isOption(value)) return Option.isSome(value) ? text(value.value) : undefined
  return typeof value === 'string' && value !== '' ? value : undefined
}
/** An optional field a newer daemon serves: a raw value, or an Option once the generated schema carries it. */
const loose = (value: unknown): unknown => Option.isOption(value) ? Option.getOrUndefined(value) : value ?? undefined
const list = (value: unknown): readonly Record<string, unknown>[] | undefined => {
  const raw = loose(value)
  return Array.isArray(raw) ? raw.filter((item): item is Record<string, unknown> => typeof item === 'object' && item !== null) : undefined
}
const count = (value: unknown): number | undefined => {
  const raw = loose(value)
  return typeof raw === 'number' && Number.isFinite(raw) ? raw : undefined
}

/** The run the detail shows: the latest open run, else the latest run with steps. */
const shownRun = (mission: Fractal.Mission): Fractal.MissionRunSummary | undefined => {
  const runs = mission.run_details ?? []
  const detailed = runs.filter(run => Option.isSome(run.steps))
  return detailed.filter(run => !FINISHED.has(run.status) && Option.isNone(run.completed_at)).at(-1) ?? detailed.at(-1) ?? runs.at(-1)
}

const outcomeOf = (word: string | undefined): Attempt['outcome'] =>
  word === 'done' || word === 'failed' || word === 'cancelled' ? word : 'running'
const waitKindOf = (word: string | undefined): WaitKind =>
  word === 'you' || word === 'dependency' || word === 'queue' || word === 'paused' || word === 'retry' ? word : 'other'

type RawStep = Fractal.MissionStep

/** The selected attempt, as served today: from its first execution to its end, or to when it began to wait. */
const coarseAttempts = (step: RawStep, agent: string | undefined, held: boolean): readonly Attempt[] => {
  const since = millis(step.since)
  const start = millis(step.started_at) ?? (ACTIVE.has(step.state) ? since : undefined)
  if (start === undefined) return []
  const terminal = FINISHED.has(step.state)
  const end = terminal ? millis(step.completed_at) ?? since : ACTIVE.has(step.state) && !held ? undefined : since !== undefined && since > start ? since : start
  const outcome: Attempt['outcome'] = step.state === 'completed' ? 'done' : step.state === 'failed' ? 'failed' : step.state === 'cancelled' ? 'cancelled' : 'running'
  return [{ n: Math.max(1, step.attempt), agent, start, end, outcome, reason: undefined }]
}

/** The current wait, as served today: from `since` to now. */
const coarseWaits = (step: RawStep, state: StepState, ask: Fractal.Attention | undefined): readonly Wait[] => {
  const since = millis(step.since)
  if (since === undefined) return []
  const reason = text(step.blocked_reason)
  if (state === 'waiting-you') return [{ kind: 'you', start: millis(ask?.requested_at) ?? since, end: undefined, reason: ask?.title ?? reason }]
  if (state === 'ready') return [{ kind: 'queue', start: since, end: undefined, reason }]
  if (state === 'waiting' || state === 'pending') return [{ kind: 'dependency', start: since, end: undefined, reason }]
  return []
}

const stateOf = (step: RawStep, you: boolean): StepState =>
  step.state === 'cancelled' ? 'cancelled'
    : step.state === 'completed' ? 'done'
      : step.state === 'failed' ? 'failed'
        : you || step.state === 'waiting-person' ? 'waiting-you'
          : ACTIVE.has(step.state) ? 'running'
            : step.state === 'ready' ? 'ready'
              : step.state === 'blocked' ? 'waiting'
                : 'pending'

const agentState = (agent: DecodedAgent | undefined, asking: boolean): AgentLiveState => {
  if (agent === undefined) return 'unknown'
  const state = leafText(agent.state)
  if (state === 'failed' || Option.isSome(agent.fault)) return 'failed'
  if (asking || text((agent as { readonly blocked_on?: unknown }).blocked_on) === 'human') return 'needs-you'
  if (state === 'stopped') return 'stopped'
  if (ACTIVE.has(state)) return 'working'
  return 'idle'
}

export const missionDetailModel = ({ mission, agents, attention, usage, now }: {
  readonly mission: Fractal.Mission
  readonly agents: readonly DecodedAgent[]
  /** Open attention, already the reader's. */
  readonly attention: readonly Fractal.Attention[]
  /** Usage rows since the run's start; undefined while unknown. */
  readonly usage: readonly UsageRowLike[] | undefined
  readonly now: number
}): MissionDetailModel => {
  const runSummary = shownRun(mission)
  // One run at a time: the presentation and the plan read only the shown run's steps.
  const scoped = Object.assign({}, mission, { run_details: runSummary === undefined ? [] : [runSummary] }) as Fractal.Mission
  const presentation = presentMission({ mission: scoped, agents, attention })
  const plan = missionPlan(scoped, presentation)
  const raw = new Map(Option.getOrElse(runSummary?.steps ?? Option.none(), () => [] as readonly RawStep[]).map(step => [step.id as string, step]))
  const idOfPath = new Map(plan.steps.map(step => [step.path, step.id]))
  const open = attention.filter(item => item.state === 'open')
  const askOn = (id: string) => open.find(item => item.step_run_id === id || (item.attention_kind === 'human-gate' && item.source_id === id))
  const steps = plan.steps.map((planned): DetailStep => {
    const step = raw.get(planned.id)!
    const ask = askOn(planned.id)
    const state = stateOf(step, planned.word === 'needs_you' || (ask !== undefined && !FINISHED.has(step.state)))
    const agent = step.agentless === true ? undefined : planned.owner.agent ?? text(step.claimant) ?? text(step.assignee)
    const extra = step as unknown as Record<string, unknown>
    const servedAttempts = list(extra.attempts)
    const servedWaits = list(extra.waits)
    const eta = loose(extra.eta) as Record<string, unknown> | undefined
    const low = count(eta?.low_ms)
    const high = count(eta?.high_ms)
    const attempts: readonly Attempt[] = servedAttempts === undefined ? coarseAttempts(step, agent, state === 'waiting-you' || state === 'waiting' || state === 'ready')
      : servedAttempts.flatMap(attempt => {
        const start = millis(attempt.started_at)
        return start === undefined ? [] : [{ n: count(attempt.n) ?? 1, agent: text(attempt.claimant) ?? agent, start, end: millis(attempt.ended_at), outcome: outcomeOf(text(attempt.outcome)), reason: text(attempt.reason) }]
      })
    const waits: readonly Wait[] = servedWaits === undefined ? coarseWaits(step, state, ask)
      : servedWaits.flatMap(wait => {
        const start = millis(wait.start)
        return start === undefined ? [] : [{ kind: waitKindOf(text(wait.kind)), start, end: millis(wait.end), reason: text(wait.reason) }]
      })
    const served = servedAttempts !== undefined
    const complete = loose(extra.history_complete)
    return {
      id: planned.id,
      path: planned.path,
      title: planned.title,
      state,
      word: planned.word,
      after: planned.after.flatMap(path => idOfPath.get(path) ?? []),
      owner: planned.owner,
      agent,
      attempts,
      attemptCount: Math.max(step.attempt, servedAttempts?.length ?? 0),
      waits,
      history: served ? 'served' : 'coarse',
      historyComplete: served ? complete !== false : step.attempt <= 1,
      expected: low === undefined || high === undefined ? undefined : { lowMs: low, highMs: high },
      spend: spendOf(usage, { step: planned.id }),
      since: planned.since,
      goal: planned.goal,
      progress: planned.progress,
      blockedReason: planned.blockedReason,
      note: planned.note,
    }
  })
  const ordered = [...steps.filter(step => step.state !== 'cancelled'), ...steps.filter(step => step.state === 'cancelled')]
  const live = ordered.filter(step => step.state !== 'cancelled')
  const ids = (state: StepState) => live.filter(step => step.state === state).map(step => step.id)
  const byId = new Map(live.map(step => [step.id, step]))
  const unstarted = (step: DetailStep | undefined) => step !== undefined && (step.state === 'pending' || step.state === 'ready')
  const next = live.find(step => unstarted(step) && step.after.every(dep => !unstarted(byId.get(dep))))
  const runStart = millis(runSummary?.started_at)
  const firstStart = Math.min(...live.flatMap(step => [...step.attempts.map(attempt => attempt.start), ...step.waits.map(wait => wait.start)]))
  const etaRaw = runSummary === undefined ? undefined : Option.getOrUndefined(runSummary.eta)
  // Who is on it: every seat a step names, in plan order, with its live state and the step it holds.
  const seats = [...new Set(live.flatMap(step => step.agent ?? []))]
  const holding = (seat: string) => {
    const mine = live.filter(step => step.agent === seat)
    return mine.find(step => step.state === 'running' || step.state === 'waiting-you' || step.state === 'waiting' || step.state === 'failed')
      ?? [...mine].sort((a, b) => (a.attempts.at(-1)?.start ?? 0) - (b.attempts.at(-1)?.start ?? 0)).at(-1)
  }
  const roster = new Map(agents.map(agent => [agent.id as string, agent]))
  const problems: Problem[] = []
  for (const step of live) {
    const last = step.attempts.at(-1)
    const resolved = step.state === 'done' ? last?.end : undefined
    if (step.history === 'served') {
      step.attempts.forEach((attempt, index) => {
        if (attempt.outcome !== 'failed') return
        const final = index === step.attempts.length - 1 && step.state === 'failed'
        problems.push({ at: attempt.end ?? attempt.start, kind: final ? 'failure' : 'retry', step: step.id, text: `${final ? `${step.title} failed` : `Attempt ${attempt.n} failed`}${attempt.reason === undefined ? '' : `: ${attempt.reason}`}`, resolvedAt: final ? undefined : resolved })
      })
    } else if (step.state === 'failed') {
      problems.push({ at: last?.end ?? step.since ?? now, kind: 'failure', step: step.id, text: `${step.title} failed${step.attemptCount > 1 ? ` on attempt ${step.attemptCount}` : ''}`, resolvedAt: undefined })
    } else if (step.attemptCount > 1) {
      problems.push({ at: last?.start ?? step.since ?? now, kind: 'retry', step: step.id, text: `${step.title} needed ${step.attemptCount} attempts`, resolvedAt: resolved })
    }
    for (const wait of step.waits) if (wait.kind === 'you') problems.push({ at: wait.start, kind: 'wait-you', step: step.id, text: `Waiting on you${wait.reason === undefined ? '' : `: ${wait.reason}`}`, resolvedAt: wait.end })
  }
  const stuck = presentation.stuck === null ? undefined : byId.get(presentation.stuck.step)
  if (stuck !== undefined && stuck.state !== 'failed' && stuck.state !== 'waiting-you') problems.push({ at: stuck.since ?? now, kind: 'stall', step: stuck.id, text: `${stuck.title} is stuck${stuck.blockedReason !== undefined ? `: ${stuck.blockedReason}` : stuck.note !== null ? `: ${stuck.note}` : ''}`, resolvedAt: undefined })
  problems.sort((a, b) => a.at - b.at)
  return {
    id: mission.id,
    title: missionReaderTitle(scoped),
    now,
    run: runSummary === undefined ? undefined : {
      id: runSummary.id,
      status: runSummary.status,
      startedAt: runStart,
      settled: FINISHED.has(runSummary.status) || Option.isSome(runSummary.completed_at),
      eta: etaRaw === undefined ? undefined : { lowMs: etaRaw.remaining_low_ms, highMs: etaRaw.remaining_high_ms, asOf: etaRaw.as_of },
    },
    startedAt: runStart ?? (Number.isFinite(firstStart) ? firstStart : now),
    steps: ordered,
    progress: { done: live.filter(step => step.state === 'done').length, total: live.length },
    running: ids('running'),
    needsYou: ids('waiting-you'),
    failed: ids('failed'),
    blocked: ids('waiting'),
    next: next?.id,
    spend: runSummary === undefined ? undefined : spendOf(usage, { run: runSummary.id, steps: new Set(steps.map(step => step.id)) }),
    agents: seats.map(seat => {
      const known = roster.get(seat)
      const asking = open.some(item => item.requester_id === seat || item.source_id === seat)
      return { id: seat, label: known?.name ?? seat.split('/').pop() ?? seat, state: agentState(known, asking), step: holding(seat)?.id }
    }),
    problems,
    presentation,
  }
}

/** A step's state at time `t`, replayed from its attempts and waits. */
export const stateAt = (step: DetailStep, t: number, now: number): StepState => {
  if (step.state === 'cancelled') return 'cancelled'
  if (step.attempts.some(attempt => attempt.start <= t && (attempt.end ?? now) > t)) return 'running'
  const wait = step.waits.find(each => each.start <= t && (each.end ?? now) > t)
  if (wait !== undefined) return wait.kind === 'you' ? 'waiting-you' : wait.kind === 'dependency' ? 'pending' : wait.kind === 'queue' ? 'ready' : 'waiting'
  const ended = step.attempts.filter(attempt => attempt.end !== undefined && attempt.end <= t).at(-1)
  if (ended?.outcome === 'done') return 'done'
  if (ended?.outcome === 'failed') return 'failed'
  if (ended?.outcome === 'cancelled') return 'cancelled'
  return 'pending'
}

/** Did any attempt of the step run inside [a, b]? */
export const ranIn = (step: DetailStep, a: number, b: number, now: number): boolean =>
  step.attempts.some(attempt => attempt.start < b && (attempt.end ?? now) > a)

/** Every step upstream and downstream of `id`, and `id`. */
export const lineage = (steps: readonly DetailStep[], id: string): ReadonlySet<string> => {
  const out = new Set<string>([id])
  const byId = new Map(steps.map(step => [step.id, step]))
  const up = (at: string) => { for (const dep of byId.get(at)?.after ?? []) if (!out.has(dep)) { out.add(dep); up(dep) } }
  const down = (at: string) => { for (const step of steps) if (step.after.includes(at) && !out.has(step.id)) { out.add(step.id); down(step.id) } }
  up(id)
  down(id)
  return out
}

/** The longest chain of dependencies by actual-or-expected duration: what decides when the run ends. */
export const criticalPath = (steps: readonly DetailStep[], now: number): ReadonlySet<string> => {
  const live = new Map(steps.filter(step => step.state !== 'cancelled').map(step => [step.id, step]))
  const weight = (step: DetailStep) => Math.max(runningMs(step, now), step.state === 'done' || step.expected === undefined ? 0 : (step.expected.lowMs + step.expected.highMs) / 2)
  const best = new Map<string, { readonly total: number; readonly prev: string | undefined }>()
  const visit = (id: string, seen: ReadonlySet<string> = new Set()): { readonly total: number; readonly prev: string | undefined } => {
    const known = best.get(id)
    if (known !== undefined) return known
    const step = live.get(id)!
    const deps = seen.has(id) ? [] : step.after.filter(dep => live.has(dep)).map(dep => ({ dep, total: visit(dep, new Set([...seen, id])).total }))
    const top = deps.sort((a, b) => b.total - a.total)[0]
    const value = { total: (top?.total ?? 0) + weight(step), prev: top?.dep }
    best.set(id, value)
    return value
  }
  let end: string | undefined
  // A tie goes to the later step, so the path runs to the plan's end.
  for (const id of live.keys()) if (end === undefined || visit(id).total >= visit(end).total) end = id
  const path = new Set<string>()
  for (let at = end; at !== undefined && !path.has(at); at = best.get(at)?.prev) path.add(at)
  return path
}

export const runningMs = (step: DetailStep, now: number): number =>
  step.attempts.reduce((sum, attempt) => sum + Math.max(0, (attempt.end ?? now) - attempt.start), 0)
export const waitedMs = (step: DetailStep, now: number, kind?: WaitKind): number =>
  step.waits.filter(wait => kind === undefined || wait.kind === kind).reduce((sum, wait) => sum + Math.max(0, (wait.end ?? now) - wait.start), 0)

/** Where a step's bars start and end on the clock, if it has any. */
export const stepStart = (step: DetailStep): number | undefined => {
  const starts = [...step.attempts.map(attempt => attempt.start), ...step.waits.map(wait => wait.start)]
  return starts.length === 0 ? undefined : Math.min(...starts)
}
export const stepEnd = (step: DetailStep, now: number): number | undefined => {
  const ends = [...step.attempts.map(attempt => attempt.end ?? now), ...step.waits.map(wait => wait.end ?? now)]
  return ends.length === 0 ? undefined : Math.max(...ends)
}

/** Spend of one step, or of a run, from usage rows; undefined when no row names it. */
export const spendOf = (rows: readonly UsageRowLike[] | undefined, match: { readonly step: string } | { readonly run: string; readonly steps: ReadonlySet<string> }): Spend | undefined => {
  if (rows === undefined) return undefined
  const mine = rows.filter(row => 'step' in match ? row.step === match.step : row.mission_run === match.run || (row.step !== undefined && match.steps.has(row.step)))
  if (mine.length === 0) return undefined
  const micro = mine.reduce((sum, row) => sum + row.cost_microusd, 0)
  return { costUsd: micro / 1_000_000, tokens: mine.reduce((sum, row) => sum + row.total_tokens, 0) }
}

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR
/** `<1m`, `4m`, `1h 12m`, `2d 3h`. */
export const span = (ms: number): string => {
  if (ms < MIN) return '<1m'
  if (ms < HOUR) return `${Math.round(ms / MIN)}m`
  if (ms < DAY) {
    const h = Math.floor(ms / HOUR)
    const m = Math.round((ms - h * HOUR) / MIN)
    return m === 0 ? `${h}h` : m === 60 ? `${h + 1}h` : `${h}h ${m}m`
  }
  const d = Math.floor(ms / DAY)
  const h = Math.round((ms - d * DAY) / HOUR)
  return h === 0 ? `${d}d` : h === 24 ? `${d + 1}d` : `${d}d ${h}h`
}

/** `$0.84`, `$11`, `$1,204`. */
export const usd = (n: number): string => n < 10 ? `$${n.toFixed(2)}` : `$${Math.round(n).toLocaleString('en-US')}`
/** `920`, `61k`, `2.2M`. */
export const tokenCount = (n: number): string => n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${Math.round(n / 1000)}k` : `${n}`

/** When the shown run started: what a spend read starts from. */
export const missionRunStart = (mission: Fractal.Mission): number | undefined => {
  const run = shownRun(mission)
  if (run === undefined) return undefined
  const starts = Option.getOrElse(run.steps, () => [] as readonly RawStep[]).flatMap(step => millis(step.started_at) ?? [])
  return millis(run.started_at) ?? (starts.length === 0 ? undefined : Math.min(...starts))
}

export interface HistoryEntry {
  readonly at: number
  readonly kind: 'started' | 'done' | 'failed' | 'asked' | 'answered' | 'run'
  readonly text: string
  readonly step: string | undefined
}

/** What happened, newest first, from the known attempts and waits: nothing without a time is invented. */
export const missionHistory = (model: MissionDetailModel): readonly HistoryEntry[] => {
  const entries: HistoryEntry[] = []
  const label = (agent: string | undefined) => agent === undefined ? undefined : model.agents.find(each => each.id === agent)?.label ?? agent.split('/').pop()
  for (const step of model.steps) {
    for (const attempt of step.attempts) {
      const who = label(attempt.agent)
      entries.push({ at: attempt.start, kind: 'started', step: step.id, text: `${who === undefined ? 'Started' : `${who} took`} ${step.title}${attempt.n > 1 ? ` (attempt ${attempt.n})` : ''}` })
      if (attempt.end !== undefined && attempt.outcome === 'done') entries.push({ at: attempt.end, kind: 'done', step: step.id, text: `${step.title} done` })
      if (attempt.end !== undefined && attempt.outcome === 'failed') entries.push({ at: attempt.end, kind: 'failed', step: step.id, text: `${step.title} failed${attempt.n > 1 ? ` on attempt ${attempt.n}` : ''}${attempt.reason === undefined ? '' : `: ${attempt.reason}`}` })
    }
    for (const wait of step.waits) {
      if (wait.kind !== 'you') continue
      entries.push({ at: wait.start, kind: 'asked', step: step.id, text: `Asked you${wait.reason === undefined ? ` on ${step.title}` : `: ${wait.reason}`}` })
      if (wait.end !== undefined) entries.push({ at: wait.end, kind: 'answered', step: step.id, text: `You answered on ${step.title}` })
    }
  }
  if (model.run?.startedAt !== undefined) entries.push({ at: model.run.startedAt, kind: 'run', step: undefined, text: `Run ${model.run.id.split('/').pop()} started` })
  return entries.sort((a, b) => b.at - a.at)
}
