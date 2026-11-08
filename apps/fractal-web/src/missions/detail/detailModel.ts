import type * as Fractal from '@fractal/client/schema'
import type { DecodedAgent } from '@fractal/sdk/read'
import type { MarkdownInline } from '@fractal/client/markdown'

import type { MissionPresentation, StepWord, Who } from '../presentation.ts'

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

export const missionDetailModel = (_input: {
  readonly mission: Fractal.Mission
  readonly agents: readonly DecodedAgent[]
  /** Open attention, already the reader's. */
  readonly attention: readonly Fractal.Attention[]
  /** Usage rows since the run's start; undefined while unknown. */
  readonly usage: readonly UsageRowLike[] | undefined
  readonly now: number
}): MissionDetailModel => { throw new Error('not implemented') }

/** A step's state at time `t`, replayed from its attempts and waits. */
export const stateAt = (_step: DetailStep, _t: number, _now: number): StepState => 'pending'

/** Did any attempt of the step run inside [a, b]? */
export const ranIn = (_step: DetailStep, _a: number, _b: number, _now: number): boolean => false

/** Every step upstream and downstream of `id`, and `id`. */
export const lineage = (_steps: readonly DetailStep[], id: string): ReadonlySet<string> => new Set([id])

/** The longest chain of dependencies by actual-or-expected duration. */
export const criticalPath = (_steps: readonly DetailStep[], _now: number): ReadonlySet<string> => new Set()

export const runningMs = (_step: DetailStep, _now: number): number => 0
export const waitedMs = (_step: DetailStep, _now: number, _kind?: WaitKind): number => 0

/** Spend of one step, or of a run, from usage rows; undefined when no row names it. */
export const spendOf = (_rows: readonly UsageRowLike[] | undefined, _match: { readonly step: string } | { readonly run: string; readonly steps: ReadonlySet<string> }): Spend | undefined => undefined

/** `<1m`, `4m`, `1h 12m`, `2d 3h`. */
export const span = (_ms: number): string => ''
