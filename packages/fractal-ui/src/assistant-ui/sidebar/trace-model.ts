/**
 * Trace-bucket seam for the sidebar trace card. Hosts fill each series as their
 * sources report it; a chart renders only for a Known series. Absence is never
 * zero: Unknown carries the reason the card shows instead of the chart.
 */

/** Why a series is absent. `not-retained`/`no-provider`: no history store; `not-attributed`: totals exist but are not split per agent. */
export type TraceAbsence = 'no-provider' | 'not-retained' | 'not-attributed'
export type TraceSeries<A> =
  | { readonly _tag: 'Known'; readonly value: A }
  | { readonly _tag: 'Pending' }
  | { readonly _tag: 'Unknown'; readonly reason: TraceAbsence }

/** One fixed-width time bucket. `usedTokens` absent means the agent was idle (a gap), never 0 % context. */
export interface ContextBucket {
  readonly start: number
  readonly end: number
  readonly usedTokens?: number
  readonly windowTokens: number
}
export interface ContextMark {
  readonly at: number
  readonly kind: 'compaction' | 'model-change'
  /** For model changes: the model in use after the mark. */
  readonly model?: string
}
export interface ContextHistory {
  readonly buckets: readonly ContextBucket[]
  readonly marks: readonly ContextMark[]
}
/** Latest scalar reading (option A row); available without history. */
export interface ContextReading {
  readonly usedTokens: number
  readonly windowTokens: number
  readonly compactions?: number
}

/** Cumulative USD at a bucket boundary, split by who spent it. */
export interface SpendPoint {
  readonly at: number
  readonly sessionUsd: number
  readonly subagentsUsd: number
}
export interface SpendTotals {
  readonly sessionUsd: number
  readonly subagentsUsd: number
}

/** API-equivalent token counts, in the usage-panel category order. */
export interface TokenSplit {
  readonly input: number
  readonly cacheRead: number
  readonly cacheWrite: number
  readonly output: number
}

export interface TraceActivity {
  readonly turns?: number
  readonly toolCalls?: number
  readonly lastTool?: string
}

export type TraceSubagentState = 'running' | 'waiting' | 'needs-you' | 'failed' | 'done'
export interface TraceSubagent {
  readonly ref: string
  readonly name: string
  readonly state: TraceSubagentState
  readonly startedAt: number
  readonly spendUsd?: number
}

export interface AgentTrace {
  /** Window the histories cover, e.g. the last 24 h ending at the host clock. */
  readonly range: { readonly start: number; readonly end: number }
  readonly context: TraceSeries<ContextHistory>
  readonly contextReading?: ContextReading
  readonly spend: TraceSeries<readonly SpendPoint[]>
  readonly spendTotals: TraceSeries<SpendTotals>
  readonly tokens: TraceSeries<TokenSplit>
  readonly activity: TraceActivity
  readonly subagents: TraceSeries<readonly TraceSubagent[]>
}

export const absenceNote = (reason: TraceAbsence): string =>
  reason === 'not-attributed' ? 'Spend not attributed to this agent' : 'History not retained'
