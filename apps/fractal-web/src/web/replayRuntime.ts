import { FractalClient } from '@fractal/client'
import { arrangementActions } from '@fractal/sdk/mutations'
import { catalog, loadWorld } from '@fractal/scenarios'
import { createReplay, realClock } from '@fractal/scenarios/replay'
import { fractalInventoryGateway } from '../data/arrangements.ts'
import type { AlertActionPort } from '../conversation/alerts/alert-action-port.ts'
import type { PromptAnswerPort } from '../conversation/alerts/prompt-answer-port.ts'
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
  const attention = new URLSearchParams(search).get('attention')
  const world = loadWorld(id, { now: new Date().toISOString(), ...(attention === null ? {} : { overrides: { attention } }) })
  const replay = createReplay(world, { clock: realClock({ start: world.now }) })
  installPageFolders(sidebarFolders({
    gateway: () => {
      const options = { baseUrl: globalThis.location.origin, fetchImpl: replay.fetch }
      return { ...fractalInventoryGateway(options), actions: arrangementActions(new FractalClient(options)) }
    },
    socket: replay.socket,
  }))
  return replay
}

const REPLY_MS = 700

/**
 * Example data's answer ports: the replay is read-only and never drops an answered alert, so these answer after a
 * moment and settle it themselves, except an alert whose id ends `-refused`, which shows a refusal rolling back.
 * Selected with `?attention=answerable` only; any other scenario keeps its plain replay.
 */
export const replayAnswerPorts = (search: string): { readonly alertActions: AlertActionPort; readonly prompts: PromptAnswerPort } | undefined => {
  if (new URLSearchParams(search).get('attention') !== 'answerable') return undefined
  const reply = (id: string) => new Promise<{ readonly _tag: 'Sent'; readonly settled: true } | { readonly _tag: 'Refused'; readonly detail: string }>((resolve) => {
    setTimeout(() => resolve(id.endsWith('-refused') ? { _tag: 'Refused', detail: 'Fractal did not take this answer.' } : { _tag: 'Sent', settled: true }), REPLY_MS)
  })
  return {
    alertActions: { act: ({ alert }) => reply(alert.id) },
    prompts: { respond: ({ alert }) => reply(alert.id) },
  }
}
