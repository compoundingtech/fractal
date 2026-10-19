import { catalog, loadWorld } from '@smalltalk/st3-scenarios'
import { createReplay, realClock } from '@smalltalk/st3-scenarios/replay'

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
  return createReplay(world, { clock: realClock({ start: world.now }) })
}
