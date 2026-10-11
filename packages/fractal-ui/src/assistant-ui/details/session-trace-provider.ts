import { Context, Effect, Layer, Schema } from 'effect'
import type { TraceSeries } from './trace-series'
import { TraceSeriesSchema } from './trace-series-schema'
import { fullTraceFixture, partialTraceFixture, unknownTraceFixture } from './trace/fixture-provider'
export interface SessionTraceQuery {readonly agentId:string;readonly range:'1d'|'7d'|'30d'}
import type { DetailsState } from './fixtures'
export interface SessionTraceProviderService {
 readonly capabilities:{readonly series:boolean;readonly subagents:boolean;readonly history:boolean;readonly harnesses:readonly ('omp'|'claude'|'codex')[]}
 readonly load:(query:SessionTraceQuery)=>Effect.Effect<TraceSeries>
}
/** Portable SessionTraceProvider service, using the host package's Effect runtime. */
export class SessionTraceProvider extends Context.Service<SessionTraceProvider,SessionTraceProviderService>()('Fractal/SessionTraceProvider') {}
const fixtureLayer=(fixture:TraceSeries,capabilities:SessionTraceProviderService['capabilities'])=>Layer.effect(SessionTraceProvider,Schema.decodeUnknownEffect(TraceSeriesSchema)(fixture).pipe(Effect.orDie,Effect.map(value=>({capabilities,load:Effect.fn('SessionTraceProvider.fixture.load')((_query:SessionTraceQuery)=>Effect.succeed(value))}))))
/** Story fixtures only. The application owns its real st-only default provider. */
export const fullTraceFixtureLayer=fixtureLayer(fullTraceFixture,{series:true,subagents:true,history:true,harnesses:['omp']})
/** Captured minute buckets remain intact when the provider does not cover turns. */
export const bucketOnlyTraceFixtureLayer=fixtureLayer({...fullTraceFixture,turns:{_tag:'Unknown',reason:'not-covered'}},{series:true,subagents:true,history:true,harnesses:['omp']})
export const partialTraceFixtureLayer=fixtureLayer(partialTraceFixture,{series:false,subagents:false,history:false,harnesses:['omp']})
export const unknownTraceFixtureLayer=fixtureLayer(unknownTraceFixture,{series:false,subagents:false,history:false,harnesses:[]})
const load=(layer:Layer.Layer<SessionTraceProvider>)=>Effect.runSync(Effect.gen(function*(){const provider=yield* SessionTraceProvider;return yield* provider.load({agentId:'agent/worker-1',range:'1d'})}).pipe(Effect.provide(layer)))
/** Provider-backed immutable story projections; no React data-fetching effects. */
export const bucketOnlyTraceFixture=load(bucketOnlyTraceFixtureLayer)
export const detailsTraceFixtures={full:load(fullTraceFixtureLayer),bucketOnly:bucketOnlyTraceFixture,partial:load(partialTraceFixtureLayer),unknown:load(unknownTraceFixtureLayer)} as const
export const traceForDetailsState=(state:DetailsState):TraceSeries=>state==='busy'||state==='idle'?detailsTraceFixtures.full:state==='partial'||state==='absent'?detailsTraceFixtures.partial:detailsTraceFixtures.unknown
