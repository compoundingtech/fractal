import { describe, expect, it } from 'vitest'
import { attentionAnswerFixtures } from '../data/attention.fixtures.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { paletteWorld } from './live.ts'

describe('the live palette world', () => {
  const agents = fixtureProjections.agents
  const world = paletteWorld({
    fleet: { hosts: [{ id: 'juniper', connected: true }], agents: agents as never },
    missions: fixtureProjections.missions as never,
    attention: fixtureProjections.attention as never,
    resources: [
      { ref: 'github/pr/compoundingtech/fractal/12', title: 'Fix it', icon: 'pull-request' },
      { ref: 'doc/notes/plan', title: 'Plan', icon: 'document' },
    ],
  })

  it('carries every agent, mission and open alert the gateway observed', () => {
    expect(world.agents.length).toBe(agents.length)
    expect(world.missions.length).toBe(fixtureProjections.missions.length)
    expect(world.alerts.length).toBe(fixtureProjections.attention.filter(item => item.state === 'open').length)
    expect(world.machines).toEqual([{ id: 'juniper', connected: true }])
  })

  it('names resources by kind and opens a document as a document', () => {
    expect(world.resources).toEqual([
      { ref: 'github/pr/compoundingtech/fractal/12', title: 'Fix it', kind: 'pull request', pane: 'resource:github/pr/compoundingtech/fractal/12' },
      { ref: 'doc/notes/plan', title: 'Plan', kind: 'document', pane: 'document:doc/notes/plan' },
    ])
  })

  it('marks an alert, and the agent it asks for, as needing you while it waits for an answer', () => {
    const asking = attentionAnswerFixtures.find(fixture => fixture.waiting)!.item
    const agent = agents[0]!
    const marked = paletteWorld({ fleet: { hosts: [], agents: [{ ...agent, ref: asking.source_id }] as never }, attention: [asking] })
    expect(marked.alerts[0]?.status?.tone).toBe('needs-you')
    expect(marked.agents[0]?.status?.tone).toBe('needs-you')
  })
})
