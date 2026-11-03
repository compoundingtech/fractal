import { expect, it } from 'vitest'
import { ANCHOR_MS, catalog, foldSlice, loadWorld, type World } from '../src/index.ts'
import { decodeSlice } from '../scripts/decode.ts'

const checkDetails = (world: World): void => {
  const slice = world.slices.details
  for (const offset of [-1, ...slice.timeline.map((event) => event.at_ms)]) {
    const snapshot = foldSlice(slice, offset)
    const agents = foldSlice(world.slices.roster, offset).state.agents
    const alerts = foldSlice(world.slices.attention, offset).state.attention
    expect(decodeSlice(world.id, snapshot)).toEqual([])
    for (const mission of snapshot.state.missions) {
      const definition = world.cast.missions.find((value) => value.id === mission.id)!
      expect(mission.run_details?.map((run) => run.id)).toEqual(mission.runs)
      for (const run of mission.run_details ?? []) {
        const work = snapshot.state.work.filter((item) => item.mission_id === mission.id && item.mission_run_id === run.id)
        expect(run.steps).toHaveLength(work.length)
        expect(run.progress).toEqual({ done: work.filter((item) => item.state === 'completed').length, total: work.length })
        expect(run.steps?.map((step) => ({ id: step.id, path: step.path, state: step.state, claimant: step.claimant }))).toEqual(
          work.map((item) => ({ id: definition.steps.find((step) => step.work === item.id)!.stepRun,
            path: item.path, state: item.state, claimant: item.claimant })),
        )
        expect((run.steps ?? []).filter((step) =>
          step.assignee !== step.claimant || (step.claimant != null && !world.cast.agents.some((agent) => agent.id === step.claimant)),
        )).toEqual([])
        expect(run.current_steps).toEqual((run.steps ?? [])
          .filter((step) => ['ready', 'claimed', 'verifying', 'blocked'].includes(step.state))
          .map((step) => ({ id: step.id, title: step.title, state: step.state, since: step.since,
            assignee: step.assignee, claimant: step.claimant })))
      }
      const steps = (mission.run_details ?? []).flatMap((run) => run.steps ?? [])
      expect(mission.presentation?.progress).toEqual({
        done: steps.filter((step) => step.state === 'completed').length, total: steps.length,
      })
      const byId = <T extends { id: string }>(items: readonly T[]): T[] => [...items].sort((a, b) => a.id.localeCompare(b.id))
      expect(byId(mission.presentation?.steps ?? []).map((step) => ({
        id: step.id, path: step.path, title: step.title, word: step.word, owner: step.owner, attempts: step.attempts,
      }))).toEqual(byId(steps).map((step) => {
        const gate = alerts.find((item) => item.state === 'open' && item.attention_kind === 'human-gate' &&
          item.mission_id === mission.id && item.step_run_id === step.id)
        const queued = step.state === 'ready' && step.claimant == null
          ? agents.find((agent) => agent.next_work_id === step.id || agent.upcoming_work_ids?.includes(step.id)) : undefined
        const ownerId = queued?.id ?? step.claimant ?? step.assignee
        const agent = agents.find((agent) => agent.id === ownerId)
        return {
          id: step.id, path: step.path, title: step.title ?? null, attempts: step.attempt,
          word: gate !== undefined ? 'needs_you' : step.state === 'completed' ? 'done'
            : ['claimed', 'verifying'].includes(step.state) ? 'working'
              : ['waiting', 'blocked'].includes(step.state) ? 'waiting'
                : ['ready', 'failed', 'cancelled'].includes(step.state) ? step.state : 'pending',
          owner: gate !== undefined ? { kind: 'person', person: gate.person_id, label: gate.person_id.replace(/^person\//u, '') }
            : ownerId == null ? { kind: 'nobody' } : { kind: 'agent', agent: ownerId, label: agent?.name ?? ownerId.replace(/^agent\//u, '') },
        }
      }))
    }
  }
}

for (const entry of catalog) {
  it(`${entry.id} joins mission run details to work and cast at every detail change`, () => {
    const world = loadWorld(entry.id, { now: ANCHOR_MS })
    checkDetails(world)
    for (const details of world.available.details.filter((name) => name !== 'default')) checkDetails(world.with({ details }))
  })
}

it('fleet-mid-refactor exposes all five steps and their actual agents', () => {
  const world = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS })
  const run = world.slices.details.state.missions[0]!.run_details![0]!
  expect(run.steps).toHaveLength(5)
  expect(new Set(run.steps?.flatMap((step) => step.claimant == null ? [] : [step.claimant])).size).toBe(3)
  expect(world.slices.details.state.missions[0]!.presentation).toMatchObject({
    word: 'stalled', progress: { done: 1, total: 5 },
    steps: expect.arrayContaining([{ id: run.steps![0]!.id, path: run.steps![0]!.path, title: run.steps![0]!.title,
      word: 'done', owner: { kind: 'agent', agent: run.steps![0]!.claimant, label: 'Atlas Builder' },
      note: null, attempts: 1, blockers: [], keep_open: false }]),
  })
})
