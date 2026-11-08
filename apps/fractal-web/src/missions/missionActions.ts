import { ClientError, plainError, type FractalClient } from '@fractal/client'

// Running what Fractal offers on a mission (`presentation.actions`, fractal-ui-model::missions::presentation): each
// offer names the client-v0 action that does it and its target. The web fences each one the way the Fractal terminal UI
// does: a run cancel on the run's served generation, a step retry on the work it reads first, a seat start on the
// seat's current declaration, and a runtime restart on the seat's running incarnation. Nothing is retried on failure.

/** One served offer (client-v0 `MissionActionOffer`). */
export interface MissionActionOffer {
  readonly id: 'retry-step' | 'restart-agent' | 'cancel-run' | 'chat'
  /** The client-v0 action type that does it, e.g. `mission.cancel`. */
  readonly action: string
  readonly target: string
  /** Binding: ask for a second tap first. */
  readonly confirm: boolean
}

export type MissionActionResult =
  | { readonly _tag: 'Done' }
  | { readonly _tag: 'Open'; readonly ref: string }
  | { readonly _tag: 'Failed'; readonly message: string }

/** The few client calls these actions need; the live source passes its FractalClient. */
export type MissionActionClient = Pick<FractalClient, 'capabilities' | 'workGet' | 'subjectDefinition' | 'agentsGet' | 'runtimesGet' | 'missionCancel' | 'workRetry' | 'agentStart' | 'runtimeRestart'>

/** The served offers on a mission row, or none from an older daemon. */
export const missionActionOffers = (mission: object): readonly MissionActionOffer[] =>
  ((mission as { readonly presentation?: { readonly actions?: readonly MissionActionOffer[] } }).presentation?.actions ?? [])

/** A person's name for an offer: what pressing it does. */
export const missionActionLabel = (offer: MissionActionOffer): string =>
  offer.id === 'retry-step' ? 'Retry step'
    : offer.id === 'cancel-run' ? 'Cancel run'
      : offer.id === 'chat' ? 'Message agent'
        : offer.action === 'agent.start' ? 'Start agent' : 'Restart agent'

/** What the confirmation says will happen, for offers Fractal marks `confirm`. */
export const missionActionConsequence = (offer: MissionActionOffer): string =>
  offer.id === 'cancel-run' ? 'This stops the run and its open steps. It cannot be resumed; a new run starts over.'
    : offer.action === 'agent.start' ? 'This starts the stopped agent on its current declaration.'
      : offer.id === 'restart-agent' ? 'This restarts the agent’s process. Its current turn is lost.'
        : 'This retries the step from its start.'

const REASON = "from the web app's mission actions"
const failed = (message: string): MissionActionResult => ({ _tag: 'Failed', message })
const identity = () => {
  const id = `action/${crypto.randomUUID()}`
  return { id, idempotency_key: id }
}
const accepted = (response: { readonly value: unknown }): MissionActionResult =>
  (response.value as { readonly status?: unknown } | null)?.status === 'rejected' ? failed('Fractal did not take this.') : { _tag: 'Done' }

/** Submit one offer. Chat opens the seat's conversation instead of sending anything. */
export const runMissionAction = async ({ client, mission, offer }: {
  readonly client: MissionActionClient
  readonly mission: { readonly run_generations: Readonly<Record<string, string>> }
  readonly offer: MissionActionOffer
}): Promise<MissionActionResult> => {
  try {
    switch (offer.action) {
      case 'message.send':
        return { _tag: 'Open', ref: offer.target }
      case 'mission.cancel': {
        const generation = mission.run_generations[offer.target]
        if (generation === undefined) return failed('Fractal no longer lists this run, so it was not cancelled. Refresh and try again.')
        const snapshot = (await client.capabilities()).snapshot.id
        return accepted(await client.missionCancel({ ...identity(), fence: { snapshot_id: snapshot, subject_revisions: {}, mission_generation: generation }, parameters: { target_id: offer.target, reason: REASON } }))
      }
      case 'work.retry': {
        const work = (await client.workGet(offer.target)).value as { readonly generation_id?: string; readonly definition_id?: string; readonly attempt?: number; readonly readiness_epoch?: number }
        if (work.generation_id === undefined || work.definition_id === undefined || work.attempt === undefined || work.readiness_epoch === undefined) return failed('Fractal did not describe this step fully, so it was not retried.')
        const snapshot = (await client.capabilities()).snapshot.id
        return accepted(await client.workRetry({
          ...identity(),
          fence: { snapshot_id: snapshot, subject_revisions: {}, mission_generation: work.generation_id, step_definition: work.definition_id, attempt: work.attempt, readiness_epoch: work.readiness_epoch },
          parameters: { target_id: offer.target, reason: REASON },
        }))
      }
      case 'agent.start': {
        const definition = await client.subjectDefinition(offer.target)
        return accepted(await client.agentStart({ ...identity(), fence: { snapshot_id: definition.snapshot.id, subject_revisions: {}, runtime_desired_revision: definition.value.desired_token }, parameters: { agent: offer.target } }))
      }
      case 'runtime.restart': {
        const agent = (await client.agentsGet(offer.target)).value as { readonly kind?: unknown; readonly runtime_ids?: readonly string[] }
        const runtimeId = agent.kind === 'agent' ? agent.runtime_ids?.[0] : undefined
        if (runtimeId === undefined) return failed('That agent is not running, so there is nothing to restart.')
        const runtime = (await client.runtimesGet(runtimeId)).value as { readonly kind?: unknown; readonly incarnation_id?: string | null; readonly desired_revision?: string | null }
        if (runtime.kind !== 'runtime' || !runtime.incarnation_id || !runtime.desired_revision) return failed('Fractal does not say which process is running, so it was not restarted.')
        const snapshot = (await client.capabilities()).snapshot.id
        return accepted(await client.runtimeRestart({ ...identity(), fence: { snapshot_id: snapshot, subject_revisions: {}, runtime_incarnation: runtime.incarnation_id, runtime_desired_revision: runtime.desired_revision }, parameters: { target_id: offer.target, reason: REASON } }))
      }
      default:
        return failed(`This web client cannot do “${offer.action}” yet.`)
    }
  } catch (error) {
    return failed(error instanceof ClientError ? `${plainError(error)}.` : 'Fractal did not confirm it; check the mission before trying again. It was not sent again.')
  }
}

/** The port a data source offers views: one submit per offer on a mission row. */
export interface MissionActionPort {
  readonly run: (request: { readonly mission: { readonly run_generations: Readonly<Record<string, string>> }; readonly offer: MissionActionOffer }) => Promise<MissionActionResult>
}

export const gatewayMissionActions = (client: MissionActionClient): MissionActionPort => ({
  run: ({ mission, offer }) => runMissionAction({ client, mission, offer }),
})
