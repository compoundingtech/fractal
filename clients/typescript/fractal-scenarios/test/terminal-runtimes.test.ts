import { FractalClient } from '@fractal/client'
import { afterEach, describe, expect, it } from 'vitest'

import { ANCHOR_MS, catalog, loadWorld, type World } from '../src/index.ts'
import { foldSlice } from '../src/kit/fold.ts'
import { createReplay, manualClock, type ReplayHandle } from '../src/replay/index.ts'

const replays: ReplayHandle[] = []
afterEach(() => { for (const replay of replays.splice(0)) replay.close() })

/** Every instant a roster or terminal event lands, and the state before the first. */
const instants = (world: World) => [...new Set([Number.NEGATIVE_INFINITY,
  ...world.slices.roster.timeline.map((event) => event.at_ms),
  ...world.slices.terminal.timeline.map((event) => event.at_ms)])].sort((a, b) => a - b)

// The gateway lists an agent's terminal-capable runtimes first in `runtime_ids`
// (crates/fractal-api-core/src/api/exec_only.rs), and clients find the terminal through them.
describe('agent runtime ids', () => {
  it.each(catalog.map(({ id }) => id))('%s lists each owned terminal runtime first, at every roster and terminal instant', (id) => {
    const world = loadWorld(id, { now: ANCHOR_MS })
    const missing = instants(world).flatMap((at) => {
      const terminals = foldSlice(world.slices.terminal, at).state.terminals
      return foldSlice(world.slices.roster, at).state.agents.flatMap((agent) => {
        const owned = terminals.filter((terminal) => terminal.owner === agent.id).map((terminal) => terminal.runtime.id)
        if (owned.length === 0 || agent.runtime_ids.length === 0) return []
        const first = agent.runtime_ids.slice(0, owned.length)
        return owned.every((runtime) => first.includes(runtime)) ? [] : [{ at, agent: agent.id, runtime_ids: agent.runtime_ids, owned }]
      })
    })
    expect(missing).toEqual([])
  }, 30_000)

  it('lets a client find the builder terminal through the roster, and still finds none for the migrator', async () => {
    const world = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS })
    const replay = createReplay(world, { clock: manualClock(world.now) })
    replays.push(replay)
    const client = new FractalClient({ baseUrl: 'http://scenario.invalid', fetchImpl: replay.fetch })
    const agents = world.slices.roster.state.agents
    const terminalOf = async (agent: string) => {
      for (const runtime of agents.find((row) => row.id === agent)!.runtime_ids) {
        const response = await client.runtimesGet(runtime)
        if (response.value.kind === 'runtime' && response.value.terminal_id != null) return response.value.terminal_id
      }
      return undefined
    }
    expect(await terminalOf('agent/example/atlas/builder')).toBe('terminal/example-atlas-builder')
    expect(await terminalOf('agent/example/atlas/migrator')).toBeUndefined()
  })
})
