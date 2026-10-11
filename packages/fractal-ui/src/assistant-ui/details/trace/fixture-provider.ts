import type { TraceBucket, TraceCostSplit, TraceFact, TraceSeries } from '../trace-series.ts'
import { sessionTraceFixture } from './fixture.ts'
import { projectAgentTrace } from './projection.ts'

const asOfMs = sessionTraceFixture.windowEnd
const known = <A>(value: A, source: 'st' | 'series' = 'series'): TraceFact<A> => ({ _tag: 'Known', value, source, asOfMs })
const unknown = { _tag: 'Unknown', reason: 'no-provider' } as const
const unattributedScope={_tag:'Unknown',reason:'not-attributed'} as const
const scene = projectAgentTrace(sessionTraceFixture)
const main = scene.lanes.find((lane) => lane.session.id === 'session-demo')!
const children = scene.lanes.filter((lane) => lane.session.id !== main.session.id)
const mainRequests = sessionTraceFixture.requests.filter((request) => request.sessionId === main.session.id)
const split: TraceCostSplit = mainRequests.reduce((sum, request) => ({ uncachedInputUsd: sum.uncachedInputUsd + request.usdInput, cacheReadUsd: sum.cacheReadUsd + request.usdCacheRead, cacheWriteUsd: sum.cacheWriteUsd + request.usdCacheWrite, outputUsd: sum.outputUsd + request.usdOutput }), { uncachedInputUsd: 0, cacheReadUsd: 0, cacheWriteUsd: 0, outputUsd: 0 })
const bucketMs = 60_000
const buckets = new Map<number, TraceBucket>()
for (const request of sessionTraceFixture.requests) {
  const tMs = Math.floor(request.ts / bucketMs) * bucketMs
  const previous = buckets.get(tMs)
  const isMain = request.sessionId === main.session.id
  // Additive zeros are empty main-session contributions, not invented observations.
  buckets.set(tMs, {
    tMs, model: isMain ? request.model : previous?.model ?? request.model,
    tokens: { input: (previous?.tokens.input ?? 0) + (isMain ? request.inputTokens : 0), output: (previous?.tokens.output ?? 0) + (isMain ? request.outputTokens : 0), cacheRead: (previous?.tokens.cacheRead ?? 0) + (isMain ? request.cacheReadTokens : 0), cacheWrite: (previous?.tokens.cacheWrite ?? 0) + (isMain ? request.cacheWriteTokens : 0) },
    cost: { uncachedInputUsd: (previous?.cost.uncachedInputUsd ?? 0) + (isMain ? request.usdInput : 0), cacheReadUsd: (previous?.cost.cacheReadUsd ?? 0) + (isMain ? request.usdCacheRead : 0), cacheWriteUsd: (previous?.cost.cacheWriteUsd ?? 0) + (isMain ? request.usdCacheWrite : 0), outputUsd: (previous?.cost.outputUsd ?? 0) + (isMain ? request.usdOutput : 0) },
    contextTokens: isMain && request.contextTokens !== null ? Math.max(previous?.contextTokens ?? 0, request.contextTokens) : previous?.contextTokens ?? null,
    contextCapacity: isMain ? request.contextCapacity : previous?.contextCapacity ?? null,
    compactionEpoch: isMain ? request.compactionEpoch : previous?.compactionEpoch ?? null,
    subagentUsd: (previous?.subagentUsd ?? 0) + (isMain ? 0 : request.usd), requests: (previous?.requests ?? 0) + request.requests,
  })
}
const compactions = sessionTraceFixture.events.flatMap((event) => event._tag === 'compaction' ? [{ tMs: event.ts, sessionId: event.sessionId }] : [])

const stContextMeter={usedTokens:known(61206,'st'),windowTokens:known(1000000,'st'),compactions:known(6,'st'),lastCompactionMs:known(asOfMs-644758,'st')}
/** Sanitized captured numeric samples; not a claim of complete live-session coverage. */
export const fullTraceFixture: TraceSeries = {
  agentId: 'agent/worker-1', nativeSessionId: known(main.session.id),
  meters: {
    context: stContextMeter,
    spend: { scope:{_tag:'IncludingSubagents'}, totalUsd: known(scene.totals.usd), mainUsd: known(main.totalUsd), subagentUsd: known(children.reduce((sum, lane) => sum + lane.totalUsd, 0)), tokens: known(scene.lanes.reduce((sum, lane) => sum + lane.tokens.total, 0)), tokensWindow: known({_tag:'Interval',fromMs:sessionTraceFixture.windowStart,toMs:sessionTraceFixture.windowEnd}), split: known(split) },
  },
  series: known({ range: '1d', bucketMs, points: [...buckets.values()].toSorted((a, b) => a.tMs - b.tMs), compactions, coverage: { harnesses: ['omp'], requestsIncluded: sessionTraceFixture.coverage.includedRequests, requestsTotal: sessionTraceFixture.coverage.totalRequests, oldestMs: sessionTraceFixture.windowStart } }),
  turns: known({ points: main.samples.map((sample, index) => ({ turnIndex: index + 1, tMs: sample.ts, contextTokens: sample.contextTokens, estimated: sample.isContextEstimated })), compactions }),
  subagents: known(children.map((lane) => ({ sessionId: lane.session.id, parentSessionId: lane.session.parentId, label: lane.session.name ?? lane.session.title, model: lane.currentModel, usd: lane.totalUsd, requests: lane.requests, firstMs: lane.session.startedAt, lastMs: lane.endTs }))),
  history: known([{ nativeSessionId: main.session.id, startedMs: main.session.startedAt, endedMs: main.session.endedAt, usd: main.totalUsd, compactions: main.compactionCount }]),
}

/** Captured st totals have no child dimension; neither self nor inclusive scope is proven. */
export const partialTraceFixture: TraceSeries = {
  agentId: 'agent/worker-1', nativeSessionId: known('native-session-example', 'st'),
  meters: {
    context: stContextMeter,
    spend: { scope:unattributedScope, totalUsd: unknown, mainUsd: unknown, subagentUsd: unknown, tokens: known(480035214, 'st'), tokensWindow:unknown, split: unknown },
  },
  series: unknown, turns: unknown, subagents: unknown, history: unknown,
}

/** Missing provider facts are omitted; unavailable sections explain the missing evidence. */
export const unknownTraceFixture: TraceSeries = {
  agentId: 'agent/worker-1', nativeSessionId: unknown,
  meters: {
    context: { usedTokens: unknown, windowTokens: unknown, compactions: unknown, lastCompactionMs: unknown },
    spend: { scope:unattributedScope, totalUsd: unknown, mainUsd: unknown, subagentUsd: unknown, tokens: unknown, tokensWindow:unknown, split: unknown },
  },
  series: unknown, turns: unknown, subagents: unknown, history: unknown,
}
