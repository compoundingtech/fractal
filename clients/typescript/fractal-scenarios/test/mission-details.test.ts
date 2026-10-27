import { expect, it } from 'vitest'
import { ANCHOR_MS, catalog, foldSlice, loadWorld, type World } from '../src/index.ts'
import { decodeSlice } from '../scripts/decode.ts'

const checkDetails = (world: World): void => {
  const slice = world.slices.details
  for (const offset of [-1, ...slice.timeline.map((event) => event.at_ms)]) {
    const snapshot = foldSlice(slice, offset)
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
        for (const step of run.steps ?? []) {
          if (step.claimant != null) expect(world.cast.agents.some((agent) => agent.id === step.claimant)).toBe(true)
          expect(step.assignee).toBe(step.claimant)
        }
        expect(run.current_steps).toEqual((run.steps ?? [])
          .filter((step) => ['ready', 'claimed', 'verifying', 'blocked'].includes(step.state))
          .map((step) => ({ id: step.id, title: step.title, state: step.state, since: step.since,
            assignee: step.assignee, claimant: step.claimant })))
      }
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
})
