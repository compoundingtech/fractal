import { DEFAULT_WORLD, catalog, loadWorld } from '@smalltalk/st3-scenarios'
import { createReplay, realClock } from '@smalltalk/st3-scenarios/replay'

import { liveSource } from '../data/liveSource.ts'

/**
 * The scenario page's transport: one replayed scenario world behind the real data layer. World
 * selection belongs to this entry (a catalog id in the page URL, else the default world);
 * production code never reads a parameter or flag to pick scenario data. The replay's socket and
 * fetch are the only transports, so nothing in this mode reaches a real gateway.
 */
const worldOf = () => {
  const requested = new URLSearchParams(window.location.search).get('world')
  const known = catalog.find((entry) => entry.id === requested)
  return loadWorld(known === undefined ? DEFAULT_WORLD : known.id)
}

const world = worldOf()
const replay = createReplay(world, { clock: realClock({ start: world.now }) })

/** The workbench source for the scenario page: replay seams in, replay only. */
export const scenario = liveSource({
  options: {
    baseUrl: window.location.origin,
    maxFollows: 8,
    conversationSlots: 'advertised',
    adoptEarlyCollections: false,
    socket: replay.socket,
    fetch: replay.fetch,
  },
})
