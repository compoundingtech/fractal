import { scenarioDomainFixture } from '../domain-fixture.ts'
import type { WorldDefinition } from '../kit/world.ts'
import { liveSync } from '../kit/variants.ts'
import { syntheticFleetMidRefactor } from './fleetMidRefactor.ts'

/** Discovery is available, but the fleet has not published any resources. */
export const empty: WorldDefinition = {
  id: 'empty',
  title: 'Empty fleet',
  narrative: 'A newly connected fleet has no agents, missions, messages, conversations or terminals yet.',
  seed: 21,
  cast: syntheticFleetMidRefactor.cast,
  slices: (ctx) => {
    const metadata = { variant: 'default', source: { _tag: 'synthetic' as const, seed: 21 }, decode: 'strict' as const, loading: false, timeline: [] }
    const conversation = { ...metadata, kind: 'conversation' as const, state: { threads: [] } }
    return {
      roster: { ...metadata, kind: 'roster', state: { sessions: [], devices: [], agents: [], runtimes: [], machines: [], order: [] } },
      details: { ...metadata, kind: 'details', state: { launches: [], launch_variants: [], missions: [], work: [] } },
      attention: { ...metadata, kind: 'attention', state: { attention: [], messages: [] } },
      conversation,
      terminal: { ...metadata, kind: 'terminal', state: { terminals: [] } },
      sync: { ...liveSync(ctx, { conversation }), source: metadata.source },
    }
  },
}

export const scenarioDomainFixtureMarker: typeof scenarioDomainFixture = scenarioDomainFixture
