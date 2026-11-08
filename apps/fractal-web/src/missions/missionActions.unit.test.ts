import { ClientError } from '@fractal/client'
import { describe, expect, it } from 'vitest'

import { missionActionLabel, runMissionAction, type MissionActionClient, type MissionActionOffer } from './missionActions.ts'

const mission = { id: 'mission/release', run_generations: { 'mission-run/release/3': 'run-generation/release/3/g2' } }
const ok = { value: { status: 'accepted' } }

/** Records every call; reads answer from the given rows. */
const fakeClient = (overrides: Record<string, unknown> = {}) => {
  const calls: Array<{ readonly method: string; readonly input: unknown }> = []
  const record = (method: string) => async (input: unknown) => { calls.push({ method, input }); return ok }
  const client = {
    capabilities: async () => ({ snapshot: { id: 'snapshot/7' } }),
    workGet: async () => ({ value: { id: 'step-run/release/3/g2/deploy', generation_id: 'run-generation/release/3/g2', definition_id: 'sha256:def', attempt: 2, readiness_epoch: 5 } }),
    subjectDefinition: async () => ({ snapshot: { id: 'snapshot/8' }, value: { desired_token: 'desired/42' } }),
    agentsGet: async () => ({ value: { kind: 'agent', runtime_ids: ['runtime/y'] } }),
    runtimesGet: async () => ({ value: { kind: 'runtime', id: 'runtime/y', incarnation_id: 'inc/9', desired_revision: 'rev/3' } }),
    missionCancel: record('missionCancel'),
    workRetry: record('workRetry'),
    agentStart: record('agentStart'),
    runtimeRestart: record('runtimeRestart'),
    ...overrides,
  } as unknown as MissionActionClient
  return { client, calls }
}

const offer = (fields: Partial<MissionActionOffer> & Pick<MissionActionOffer, 'id' | 'action' | 'target'>): MissionActionOffer => ({ confirm: false, ...fields })

describe('mission actions', () => {
  it('cancels a run fenced to its served generation', async () => {
    const { client, calls } = fakeClient()
    const result = await runMissionAction({ client, mission, offer: offer({ id: 'cancel-run', action: 'mission.cancel', target: 'mission-run/release/3', confirm: true }) })
    expect(result).toEqual({ _tag: 'Done' })
    expect(calls).toHaveLength(1)
    expect(calls[0]).toMatchObject({ method: 'missionCancel', input: { fence: { snapshot_id: 'snapshot/7', mission_generation: 'run-generation/release/3/g2', subject_revisions: {} }, parameters: { target_id: 'mission-run/release/3' } } })
  })

  it('retries a step fenced to the work it reads first', async () => {
    const { client, calls } = fakeClient()
    await runMissionAction({ client, mission, offer: offer({ id: 'retry-step', action: 'work.retry', target: 'step-run/release/3/g2/deploy' }) })
    expect(calls[0]).toMatchObject({ method: 'workRetry', input: {
      fence: { snapshot_id: 'snapshot/7', mission_generation: 'run-generation/release/3/g2', step_definition: 'sha256:def', attempt: 2, readiness_epoch: 5 },
      parameters: { target_id: 'step-run/release/3/g2/deploy' },
    } })
    expect((calls[0]!.input as { parameters: { reason: string } }).parameters.reason).not.toBe('')
  })

  it('starts a stopped seat fenced to its current declaration', async () => {
    const { client, calls } = fakeClient()
    await runMissionAction({ client, mission, offer: offer({ id: 'restart-agent', action: 'agent.start', target: 'agent/builder', confirm: true }) })
    expect(calls[0]).toMatchObject({ method: 'agentStart', input: { fence: { snapshot_id: 'snapshot/8', runtime_desired_revision: 'desired/42' }, parameters: { agent: 'agent/builder' } } })
  })

  it('restarts a faulted runtime fenced to the seat runtime it finds', async () => {
    const { client, calls } = fakeClient()
    await runMissionAction({ client, mission, offer: offer({ id: 'restart-agent', action: 'runtime.restart', target: 'agent/builder', confirm: true }) })
    expect(calls[0]).toMatchObject({ method: 'runtimeRestart', input: { fence: { snapshot_id: 'snapshot/7', runtime_incarnation: 'inc/9', runtime_desired_revision: 'rev/3' }, parameters: { target_id: 'agent/builder' } } })
  })

  it('gives every submit its own action id and idempotency key', async () => {
    const { client, calls } = fakeClient()
    const cancel = offer({ id: 'cancel-run', action: 'mission.cancel', target: 'mission-run/release/3' })
    await runMissionAction({ client, mission, offer: cancel })
    await runMissionAction({ client, mission, offer: cancel })
    const [a, b] = calls.map(call => call.input as { id: string; idempotency_key: string })
    expect(a!.id).toMatch(/^action\//)
    expect(a!.id).not.toBe(b!.id)
    expect(a!.idempotency_key).not.toBe(b!.idempotency_key)
  })

  it('says why in plain words when Fractal refuses', async () => {
    const { client } = fakeClient({ missionCancel: async () => { throw new ClientError({ code: 'stale-fence', message: 'the mission generation fence is stale' } as never, 409) } })
    const result = await runMissionAction({ client, mission, offer: offer({ id: 'cancel-run', action: 'mission.cancel', target: 'mission-run/release/3' }) })
    expect(result._tag).toBe('Failed')
    expect(result._tag === 'Failed' ? result.message : '').toMatch(/changed while this was on its way/)
  })

  it('refuses a cancel whose run has no served generation, without sending', async () => {
    const { client, calls } = fakeClient()
    const result = await runMissionAction({ client, mission, offer: offer({ id: 'cancel-run', action: 'mission.cancel', target: 'mission-run/release/9' }) })
    expect(result._tag).toBe('Failed')
    expect(calls).toHaveLength(0)
  })

  it('names actions for people, and chat is an opening, not a submit', async () => {
    expect(missionActionLabel(offer({ id: 'retry-step', action: 'work.retry', target: 's' }))).toBe('Retry step')
    expect(missionActionLabel(offer({ id: 'restart-agent', action: 'agent.start', target: 'a' }))).toBe('Start agent')
    expect(missionActionLabel(offer({ id: 'restart-agent', action: 'runtime.restart', target: 'a' }))).toBe('Restart agent')
    expect(missionActionLabel(offer({ id: 'cancel-run', action: 'mission.cancel', target: 'r' }))).toBe('Cancel run')
    const { client, calls } = fakeClient()
    expect(await runMissionAction({ client, mission, offer: offer({ id: 'chat', action: 'message.send', target: 'agent/builder' }) })).toEqual({ _tag: 'Open', ref: 'agent/builder' })
    expect(calls).toHaveLength(0)
  })
})
