import { encodeKdl } from '@fractal/declarations'
import type { Agent, Mission, Schedule } from '@fractal/declarations'

export const editor = {
  id: 'docs/editor', workspace: '.', launch: { command: 'true' },
} as const satisfies Agent

export const writer = {
  id: 'docs/writer', workspace: '.', launch: { command: 'true' },
  under: { agent: editor, reason: "The editor reviews the writer's work." },
} as const satisfies Agent

// Areas are project metadata, not a declaration kind supported by the SDK.
export const area = { id: 'docs', name: 'Documentation', agents: [editor, writer] } as const

export const mission = {
  id: 'docs/daily-review', state: 'ready', reportTo: editor,
  goals: ['Review the documentation each day.'], completion: 'all-steps-exhausted',
  steps: [{ id: 'review', assignedTo: writer, goals: ['Review the documentation and report corrections.'] }],
} as const satisfies Mission

export const schedule = {
  id: mission.id, host: 'local', every: '1d', anchor: '2026-01-01T09:00:00Z', catchUp: 'latest',
  work: {
    mission,
    // Obtained from `fractal validate`; checked against the Rust parser's golden fixture.
    revision: 'b283ff12e887f06fbeb2da3094b10f8fd2b9b7840f7af8031445b124829f25f7',
    workspace: '.',
  },
} as const satisfies Schedule

export const render = (missionOnly = false): string => encodeKdl({
  agents: area.agents, missions: [mission], schedules: missionOnly ? [] : [schedule],
})
