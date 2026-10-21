import type { TraceAccumulationWindow, TraceCoverage, TraceFact, TraceSubagent } from './trace-series'

/**
 * An omp-driven seat whose current session is proven by its todo snapshot and priced by the usage period, so the
 * production inspector takes the OMP subagent-activity branch end to end. Plain wire rows and feed states only:
 * the app's integration test and the kit's production Details stories each decode them into their own data source.
 */
export const ompAgentRef = 'agent/omp-details'
export const ompSessionId = 'omp-session/root'
export const ompSubagent = { sessionId: 'omp-session/reviewer', label: 'Reviewer', model: 'example-model' } as const
/** The fixture world's clock. */
export const ompNow = 10_000
const at = '1970-01-01T00:00:04.000Z'

/** The seat's wire row, for strict `Agent` decoding. */
export const ompAgentWire = {
  kind: 'agent', id: ompAgentRef, name: 'OMP Details seat', host_id: 'host/build-1', runtime_ids: [],
  reachability: 'reachable', state: 'running', revision: '1', incarnation_id: 'first', updated_at: at, driver: 'omp',
  todo: { accepted_at: at, claim_id: 'claim/todo', stale: false, snapshot: {
    harness: 'omp', incarnation_id: 'first', session_id: ompSessionId, observed_at: at, source_op: 'todo/write',
    phases: [], totals: { blocked: 0, completed: 0, in_progress: 0, pending: 0 }, truncated: false,
  } },
} as const
/** The session's wire usage period, for strict `UsagePeriod` decoding. The host reports priced spend but no start bound; zero must never become an epoch interval. */
export const ompUsagePeriodWire = { since_ms: 0, until_ms: 5000, rows: [{
  agent: ompAgentRef, native_session_id: ompSessionId, model: 'example-model', input_tokens: 10, output_tokens: 20, cached_tokens: 3,
  cache_write_tokens: 4, cache_write_1h_tokens: 0, total_tokens: 37, cost_microusd: 6_190_000, reported_cost_microusd: 0, unpriced_tokens: 0,
}] } as const
/** Declarations the seat does not report. */
export const ompDeclarationsNotReported = { _tag: 'Unavailable', reason: 'unsupported', detail: 'Not reported' } as const

const reported = <A>(value: A): TraceFact<A> => ({ _tag: 'Known', value, source: 'series' })
const notCovered = { _tag: 'Unknown', reason: 'not-covered' } as const
const sevenDays = reported<TraceAccumulationWindow>({ _tag: 'Reported', label: '7d' })
const coverage: TraceCoverage = { _tag: 'Partial', attributedFraction: 0.995 }
const reviewer: TraceSubagent = {
  sessionId: ompSubagent.sessionId, parentSessionId: notCovered, label: reported(ompSubagent.label),
  model: reported(ompSubagent.model), usd: 2.86, requests: 12, firstMs: notCovered, lastMs: notCovered,
}
/** A matching report of total spend and one subagent; total spend differs from the agent's own spend. */
export const ompSupplementValue = {
  nativeSessionId: ompSessionId,
  totalUsd: reported(9.05),
  usdWindow: sevenDays,
  coverage,
  subagents: reported<readonly TraceSubagent[]>([reviewer]),
  subagentsWindow: sevenDays,
  subagentCount: reported(1),
}

/** Subagent-activity feeds for each OMP state, shaped for the composition's `UseDetailsSupplement` seam. */
export const ompSupplementFeeds = {
  loading: () => ({ _tag: 'Waiting' }) as const,
  /** The source's own detail is technical and must never reach the screen. */
  unavailable: () => ({ _tag: 'Unavailable', reason: 'failed', detail: 'Descendant ledger read failed: root not found in window-global attribution cut.' }) as const,
  reported: () => ({ _tag: 'Observed', value: ompSupplementValue, freshness: 'live' }) as const,
  retained: () => ({ _tag: 'Observed', value: { ...ompSupplementValue, receivedAtMs: 4_000 }, freshness: 'stale', error: { reason: 'failed', detail: 'Descendant ledger revalidation timed out (provenance: series).' } }) as const,
}
export type OmpSupplementState = keyof typeof ompSupplementFeeds
