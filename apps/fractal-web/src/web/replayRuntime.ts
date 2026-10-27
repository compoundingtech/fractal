import { FractalClient } from '@fractal/client'
import { arrangementActions } from '@fractal/sdk/effect'
import { catalog, loadWorld } from '@fractal/scenarios'
import { createReplay, realClock } from '@fractal/scenarios/replay'
import { st3InventoryGateway } from '../data/arrangements.ts'
import { installPageFolders, sidebarFolders } from '../folders/client.ts'

/** Only imported behind the page entry's compile-time DEV guard. */
export const selectReplayWorld = (search: string): string | undefined => {
  const id = new URLSearchParams(search).get('scenario')
  if (id === null) return undefined
  if (!catalog.some((world) => world.id === id)) {
    throw new Error(`Unknown Fractal replay scenario ${JSON.stringify(id)}. Choose one of: ${catalog.map((world) => world.id).join(', ')}`)
  }
  return id
}

export const createDevReplay = (search: string) => {
  const id = selectReplayWorld(search)
  if (id === undefined) return undefined
  const world = loadWorld(id, { now: new Date().toISOString() })
  const replay = createReplay(world, { clock: realClock({ start: world.now }) })
  installPageFolders(sidebarFolders({
    gateway: () => {
      const options = { baseUrl: globalThis.location.origin, fetchImpl: replay.fetch }
      return { ...st3InventoryGateway(options), actions: arrangementActions(new FractalClient(options)) }
    },
    socket: replay.socket,
  }))
  return replay
}
