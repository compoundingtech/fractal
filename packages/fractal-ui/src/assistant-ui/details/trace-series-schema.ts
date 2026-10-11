import { Schema } from 'effect'
import type { TraceSeries } from './trace-series'
const count=Schema.Int.check(Schema.isGreaterThanOrEqualTo(0))
const amount=Schema.Number.check(Schema.isFinite(),Schema.isGreaterThanOrEqualTo(0))
const timestamp=Schema.Number.check(Schema.isFinite())
const unknown=Schema.TaggedStruct('Unknown',{reason:Schema.Literals(['no-provider','not-covered','not-attributed','out-of-retention','loading','failed']),detail:Schema.optional(Schema.String)})
const fact=<S extends Schema.Top>(value:S)=>Schema.Union([Schema.TaggedStruct('Known',{value,source:Schema.Literals(['st','series']),asOfMs:timestamp}),unknown])
const cost=Schema.Struct({uncachedInputUsd:amount,cacheReadUsd:amount,cacheWriteUsd:amount,outputUsd:amount})
const spendScope=Schema.Union([Schema.TaggedStruct('Self',{}),Schema.TaggedStruct('IncludingSubagents',{}),Schema.TaggedStruct('Unknown',{reason:Schema.Literal('not-attributed')})])
const accumulationWindow=Schema.Union([Schema.TaggedStruct('AllTime',{}),Schema.TaggedStruct('Interval',{fromMs:timestamp,toMs:timestamp})])
const subagent=Schema.Struct({sessionId:Schema.String,parentSessionId:Schema.NullOr(Schema.String),label:Schema.NullOr(Schema.String),model:Schema.String,usd:amount,requests:count,firstMs:timestamp,lastMs:timestamp})
const buckets=Schema.Struct({range:Schema.Literals(['1d','7d','30d']),bucketMs:count,points:Schema.Array(Schema.Struct({tMs:timestamp,tokens:Schema.Struct({input:count,output:count,cacheRead:count,cacheWrite:count}),cost,model:Schema.String,contextTokens:Schema.NullOr(count),contextCapacity:Schema.NullOr(count),compactionEpoch:Schema.NullOr(count),subagentUsd:amount,requests:count})),compactions:Schema.Array(Schema.Struct({tMs:timestamp,sessionId:Schema.String})),coverage:Schema.Struct({harnesses:Schema.Array(Schema.Literals(['omp','claude','codex'])),requestsIncluded:count,requestsTotal:count,oldestMs:timestamp})})
const turns=Schema.Struct({points:Schema.Array(Schema.Struct({turnIndex:count,tMs:timestamp,contextTokens:Schema.NullOr(count),estimated:Schema.Boolean})),compactions:Schema.Array(Schema.Struct({tMs:timestamp,sessionId:Schema.String}))})
/** The single portable trace schema; providers decode this at their data boundary. */
export const TraceSeriesSchema=Schema.Struct({
 agentId:Schema.String,nativeSessionId:fact(Schema.String),
 meters:Schema.Struct({context:Schema.Struct({usedTokens:fact(count),windowTokens:fact(count),compactions:fact(count),lastCompactionMs:fact(timestamp)}),spend:Schema.Struct({scope:spendScope,totalUsd:fact(amount),mainUsd:fact(amount),subagentUsd:fact(amount),tokens:fact(count),tokensWindow:fact(accumulationWindow),split:fact(cost)})}),
 series:fact(buckets),turns:fact(turns),subagents:fact(Schema.Array(subagent)),history:fact(Schema.Array(Schema.Struct({nativeSessionId:Schema.String,startedMs:timestamp,endedMs:Schema.NullOr(timestamp),usd:amount,compactions:count}))),
}).annotate({identifier:'Fractal.TraceSeries'})
type Equal<A,B>=(<T>()=>T extends A?1:2) extends (<T>()=>T extends B?1:2)?true:false
type Assert<T extends true>=T
export type TraceSeriesSchemaMatchesProjection=Assert<Equal<typeof TraceSeriesSchema.Type,TraceSeries>>
