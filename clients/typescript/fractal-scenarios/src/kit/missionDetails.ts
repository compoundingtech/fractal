import type { Mission, MissionRunSummary, MissionStep, Work } from '@fractal/client'

import type { FactoryContext } from './context.ts'
import { foldSlice } from './fold.ts'
import { missionPresentation } from './missionPresentation.ts'
import type { DetailsState, Slice } from './slice.ts'

const finished = (state: string): boolean => ['completed', 'failed', 'cancelled'].includes(state)

/** Project the authored work inventory, rather than maintaining a second set of step states. */
const withRunDetails = (ctx: FactoryContext, mission: Mission, work: readonly Work[]): Mission => {
  const definition = ctx.cast.missions.find((value) => value.id === mission.id)
  if (!definition) throw new Error(`Missing cast mission: ${mission.id}`)
  const runDetails = mission.runs.map((id): MissionRunSummary => {
    const steps = work.filter((item) => item.mission_id === mission.id && item.mission_run_id === id).map((item): MissionStep => {
      const step = definition.steps.find((value) => value.work === item.id)
      if (!step) throw new Error(`Missing cast step: ${item.id}`)
      return {
        id: step.stepRun, path: item.path, title: item.title, state: item.state,
        attempt: item.attempt, since: item.updated_at, claimant: item.claimant,
        assignee: item.claimant, blocked_reason: item.blocked_reason,
        blockers: item.blockers, goals: item.goals, constraints: item.constraints,
      }
    })
    const mustAct = finished(mission.state) ? 'nobody'
      : steps.some((step) => step.state === 'waiting-person') ? 'you'
        : steps.some((step) => ['ready', 'claimed', 'verifying'].includes(step.state) && step.claimant !== null) ? 'agent'
          : steps.some((step) => step.state === 'blocked') ? 'blocked' : 'system'
    return {
      id, generation_id: mission.run_generations[id],
      requester: ctx.cast.people[0]?.id ?? ctx.cast.agents[0]!.id,
      status: mission.state, phase: finished(mission.state) ? 'terminal' : 'normal',
      progress: { done: steps.filter((step) => step.state === 'completed').length, total: steps.length },
      current_steps: steps.filter((step) => ['ready', 'claimed', 'verifying', 'blocked'].includes(step.state)).map((step) => ({
        id: step.id, title: step.title ?? null, state: step.state, since: step.since,
        assignee: step.assignee ?? null, claimant: step.claimant ?? null,
      })),
      must_act: mustAct, state_since: mission.updated_at, steps,
    }
  })
  return { ...mission, run_details: runDetails }
}

/** Recorded slices remain exact captures; synthetic changes publish their joined mission row too. */
export const missionDetails = (ctx: FactoryContext, slice: Slice<'details'>, roster: Slice<'roster'>, attention: Slice<'attention'>): Slice<'details'> => {
  if (slice.source._tag === 'recorded') return slice
  const project = (state: DetailsState, atMs: number): DetailsState => {
    const agents = foldSlice(roster, atMs).state.agents
    const alerts = foldSlice(attention, atMs).state.attention
    return { ...state, missions: state.missions.map((mission) => {
      const joined = withRunDetails(ctx, mission, state.work)
      return { ...joined, presentation: missionPresentation(joined, agents, alerts) }
    }) }
  }
  const state = project(slice.state, 0)
  let current: Slice<'details'> = { ...slice, state, timeline: [] }
  const timeline = slice.timeline.map((event) => {
    const next = foldSlice({ ...current, timeline: [event] }, event.at_ms)
    const projected = project(next.state, event.at_ms)
    const changedIds = new Set([...event.upserts.map((item) => item.id), ...event.removes])
    const affected = new Set([...current.state.work, ...next.state.work]
      .filter((item) => changedIds.has(item.id)).map((item) => item.mission_id))
    const changed = projected.missions.filter((mission) => changedIds.has(mission.id) || affected.has(mission.id))
    current = { ...next, state: projected, timeline: [] }
    return { ...event, upserts: [...event.upserts.filter((item) => item.kind !== 'mission'), ...changed] }
  })
  return { ...slice, state, timeline }
}
