/** Portable projection; the schema and Effect provider live in separate modules. */
export type TraceUnknownReason = 'no-provider'|'not-covered'|'not-attributed'|'out-of-retention'|'loading'|'failed'
export type TraceFact<A> = {readonly _tag:'Known';readonly value:A;readonly source:'st'|'series';readonly asOfMs:number}|{readonly _tag:'Unknown';readonly reason:TraceUnknownReason;readonly detail?:string}
export type TraceSpendScope = {readonly _tag:'Self'}|{readonly _tag:'IncludingSubagents'}|{readonly _tag:'Unknown';readonly reason:'not-attributed'}
/** Reported accumulation window for the token total, independent of chart retention. */
export type TraceAccumulationWindow = {readonly _tag:'AllTime'}|{readonly _tag:'Interval';readonly fromMs:number;readonly toMs:number}
export interface TraceCostSplit {readonly uncachedInputUsd:number;readonly cacheReadUsd:number;readonly cacheWriteUsd:number;readonly outputUsd:number}
export interface TraceBucket {readonly tMs:number;readonly tokens:{readonly input:number;readonly output:number;readonly cacheRead:number;readonly cacheWrite:number};readonly cost:TraceCostSplit;readonly model:string;readonly contextTokens:number|null;readonly contextCapacity:number|null;readonly compactionEpoch:number|null;readonly subagentUsd:number;readonly requests:number}
export interface TraceBuckets {readonly range:'1d'|'7d'|'30d';readonly bucketMs:number;readonly points:readonly TraceBucket[];readonly compactions:readonly {readonly tMs:number;readonly sessionId:string}[];readonly coverage:{readonly harnesses:readonly ('omp'|'claude'|'codex')[];readonly requestsIncluded:number;readonly requestsTotal:number;readonly oldestMs:number}}
/** Retained main-session request observations, never expanded from bucket counts. */
export interface TraceTurns {readonly points:readonly {readonly turnIndex:number;readonly tMs:number;readonly contextTokens:number|null;readonly estimated:boolean}[];readonly compactions:readonly {readonly tMs:number;readonly sessionId:string}[]}
export interface TraceSubagent {readonly sessionId:string;readonly parentSessionId:string|null;readonly label:string|null;readonly model:string;readonly usd:number;readonly requests:number;readonly firstMs:number;readonly lastMs:number}
export interface TraceIncarnation {readonly nativeSessionId:string;readonly startedMs:number;readonly endedMs:number|null;readonly usd:number;readonly compactions:number}
export interface TraceSeries {
 readonly agentId:string
 readonly nativeSessionId:TraceFact<string>
 readonly meters:{readonly context:{readonly usedTokens:TraceFact<number>;readonly windowTokens:TraceFact<number>;readonly compactions:TraceFact<number>;readonly lastCompactionMs:TraceFact<number>};readonly spend:{readonly scope:TraceSpendScope;readonly totalUsd:TraceFact<number>;readonly mainUsd:TraceFact<number>;readonly subagentUsd:TraceFact<number>;readonly tokens:TraceFact<number>;readonly tokensWindow:TraceFact<TraceAccumulationWindow>;readonly split:TraceFact<TraceCostSplit>}}
 readonly series:TraceFact<TraceBuckets>
 readonly turns:TraceFact<TraceTurns>
 readonly subagents:TraceFact<readonly TraceSubagent[]>
 readonly history:TraceFact<readonly TraceIncarnation[]>
}
