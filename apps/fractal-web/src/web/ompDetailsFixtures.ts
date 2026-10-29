import * as Atom from 'effect/reactivity/Atom'
import { Agent, UsagePeriod, decodeUnknownSync } from '@fractal/client/schema'
import type * as Native from '@fractal/client/schema'
import { ompAgentWire, ompDeclarationsNotReported, ompNow, ompUsagePeriodWire } from '../../../../packages/fractal-ui/src/assistant-ui/details/omp-fixtures.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, type DataSource, type Feed } from '../data/source.ts'
import type { NativeDetailsSource } from '../data/nativeDetailsSource.ts'
import type { NativeDeclarations } from '../data/nativeDeclarations.ts'

/** The kit's omp seat over the deterministic fixture world, with its priced session as the native Details read. */
export const ompDetailsSource = (): DataSource => {
  const rawDeclarations = Atom.make<Feed<Native.AgentDeclaration>>(ompDeclarationsNotReported)
  const declarations = Atom.make<Feed<NativeDeclarations>>(ompDeclarationsNotReported)
  const usagePeriod = Atom.make<Feed<Native.UsagePeriod>>(observed({ value: decodeUnknownSync(UsagePeriod)(ompUsagePeriodWire) }))
  return {
    ...fixtureSource({ world: { now: ompNow, agents: [decodeUnknownSync(Agent)(ompAgentWire)], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
    nativeDetails: {
      usagePeriod: () => usagePeriod,
      rawDeclarations: () => rawDeclarations,
      declarations: () => declarations,
      // The fixture world is fixed: a re-read observes the same period.
      retry: () => undefined,
      refresh: () => undefined,
    } satisfies NativeDetailsSource,
  }
}
