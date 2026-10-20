import * as Atom from 'effect/reactivity/Atom'
import { Agent, UsagePeriod, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import type * as Native from '@smalltalk/st3-client/schema'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, unavailable, waiting, type DataSource, type Feed } from '../data/source.ts'
import type { NativeDetailsSource } from '../data/nativeDetailsSource.ts'
import type { NativeDeclarations } from '../data/nativeDeclarations.ts'
import type { DetailsSupplement, UseDetailsSupplement } from '../extensions/contract.ts'

/**
 * An omp-driven seat whose current session is proven by its todo snapshot and priced by the usage period, so the
 * production inspector takes the OMP subagent-activity branch end to end. Shared by the app tests and the kit's
 * production Details stories.
 */
export const ompAgentRef = 'agent/omp-details'
export const ompSessionId = 'omp-session/root'
export const ompSubagent = { sessionId: 'omp-session/reviewer', label: 'Reviewer', model: 'example-model' } as const
const at = '1970-01-01T00:00:04.000Z'

export const ompAgentRow: Native.Agent = decodeUnknownSync(Agent)({
  kind: 'agent', id: ompAgentRef, name: 'OMP Details seat', host_id: 'host/example', runtime_ids: [],
  reachability: 'reachable', state: 'running', revision: '1', incarnation_id: 'first', updated_at: at, driver: 'omp',
  todo: { accepted_at: at, claim_id: 'claim/todo', stale: false, snapshot: {
    harness: 'omp', incarnation_id: 'first', session_id: ompSessionId, observed_at: at, source_op: 'todo/write',
    phases: [], totals: { blocked: 0, completed: 0, in_progress: 0, pending: 0 }, truncated: false,
  } },
})
// The host reports priced spend but no start bound; zero must never become an epoch interval.
const ompUsagePeriod: Native.UsagePeriod = decodeUnknownSync(UsagePeriod)({ since_ms: 0, until_ms: 5000, rows: [{
  agent: ompAgentRef, native_session_id: ompSessionId, model: 'example-model', input_tokens: 10, output_tokens: 20, cached_tokens: 3,
  cache_write_tokens: 4, cache_write_1h_tokens: 0, total_tokens: 37, cost_microusd: 6_190_000, reported_cost_microusd: 0, unpriced_tokens: 0,
}] })
const declarationsUnreported = Atom.make<Feed<NativeDeclarations>>(unavailable({ reason: 'unsupported', detail: 'Not reported' }))
const rawDeclarationsUnreported = Atom.make<Feed<Native.AgentDeclaration>>(unavailable({ reason: 'unsupported', detail: 'Not reported' }))
/** The deterministic fixture world plus the omp seat's priced session. */
export const ompDetailsSource = (): DataSource => ({
  ...fixtureSource({ world: { now: 10_000, agents: [ompAgentRow], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
  nativeDetails: {
    usagePeriod: Atom.make<Feed<Native.UsagePeriod>>(observed({ value: ompUsagePeriod })),
    rawDeclarations: () => rawDeclarationsUnreported,
    declarations: () => declarationsUnreported,
    retry: () => undefined,
  } satisfies NativeDetailsSource,
})

const reported = <A>(value: A) => ({ _tag: 'Known', value, source: 'series' } as const)
/** A matching report of total spend and one subagent; total spend differs from the agent's own spend. */
export const ompSupplementValue: DetailsSupplement = {
  nativeSessionId: ompSessionId,
  totalUsd: reported(9.05),
  usdWindow: reported({ _tag: 'Reported', label: '7d' }),
  coverage: { _tag: 'Partial', attributedFraction: 0.995 },
  subagents: reported([{
    sessionId: ompSubagent.sessionId, parentSessionId: { _tag: 'Unknown', reason: 'not-covered' }, label: reported(ompSubagent.label),
    model: reported(ompSubagent.model), usd: 2.86, requests: 12, firstMs: { _tag: 'Unknown', reason: 'not-covered' }, lastMs: { _tag: 'Unknown', reason: 'not-covered' },
  }]),
  subagentsWindow: reported({ _tag: 'Reported', label: '7d' }),
  subagentCount: reported(1),
}

/** Subagent-activity feeds for each OMP state, through the composition's `UseDetailsSupplement` seam. */
export const ompSupplementFeeds = {
  loading: () => waiting,
  /** The source's own detail is technical and must never reach the screen. */
  unavailable: () => unavailable({ reason: 'failed', detail: 'Descendant ledger read failed: root not found in window-global attribution cut.' }),
  reported: () => observed({ value: ompSupplementValue }),
  retained: () => ({ _tag: 'Observed', value: { ...ompSupplementValue, receivedAtMs: 4_000 }, freshness: 'stale', error: { reason: 'failed', detail: 'Descendant ledger revalidation timed out (provenance: series).' } }),
} as const satisfies Record<string, UseDetailsSupplement>
export type OmpSupplementState = keyof typeof ompSupplementFeeds
