import { describe, expect, it, vi } from 'vitest'
import { openAgent } from '@fractal/sdk/read'
import { agentDescriptor, missionStatus } from '@fractal/ui/assistant-ui'
import { attentionAnswerFixtures } from '../data/attention.fixtures.ts'
import { fleetFromAgents } from '../data/projections.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { missionChildOf } from '../missions/rowData.ts'
import { presentMission } from '../missions/presentation.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import { opensInWeb, paletteWorld } from './live.ts'

// The kind registry brings each kind's surface, and their styles.
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const decoded = fixtureProjections.agents.map(openAgent)
const fleet = fleetFromAgents(decoded)
const open = fixtureProjections.attention.filter(item => item.state === 'open')
const missionRows = fixtureProjections.missions.map(mission => ({ mission, presentation: presentMission({ mission, agents: decoded, attention: open }) }))
const needsMe = new Set([fleet.agents[0]!.ref])

describe('the live palette world', () => {
  const world = paletteWorld({
    fleet, needsMe, missionRows,
    attention: fixtureProjections.attention,
    resources: [
      { ref: 'resource/github/compoundingtech/fractal/pull-request/12', title: 'Fix it', icon: 'pull-request' },
      { ref: 'doc/notes/plan', title: 'Plan', icon: 'document' },
    ],
  })

  it('carries every agent, mission and open alert the gateway observed', () => {
    expect(world.agents.length).toBe(fleet.agents.length)
    expect(world.missions.length).toBe(fixtureProjections.missions.length)
    expect(world.alerts.length).toBe(open.length)
  })

  it('shows each agent’s status exactly as the sidebar ring does', () => {
    for (const [index, agent] of fleet.agents.entries()) {
      expect(world.agents[index]!.status).toEqual(agentDescriptor(sidebarRow({ agent, stale: false, needsMe: needsMe.has(agent.ref) })).status)
    }
    expect(world.agents[0]!.status?.tone).toBe('needs-you')
  })

  it('shows each mission’s status exactly as its sidebar row does', () => {
    for (const [index, row] of missionRows.entries()) expect(world.missions[index]!.status).toEqual(missionStatus(missionChildOf(row)))
  })

  it('names resources by kind and opens a document as a document', () => {
    expect(world.resources).toEqual([
      { ref: 'resource/github/compoundingtech/fractal/pull-request/12', title: 'Fix it', kind: 'pull request', pane: 'resource:resource/github/compoundingtech/fractal/pull-request/12' },
      { ref: 'doc/notes/plan', title: 'Plan', kind: 'document', pane: 'document:doc/notes/plan' },
    ])
  })

  it('marks an alert that waits for an answer as needing you', () => {
    const asking = attentionAnswerFixtures.find(fixture => fixture.waiting)!.item
    expect(paletteWorld({ attention: [asking] }).alerts[0]?.status?.tone).toBe('needs-you')
  })
})

describe('what the web can open', () => {
  it('follows the kind registry: kinds without a surface stay hidden', () => {
    for (const ref of ['agent:agent/example/atlas', 'mission:mission/release', 'list:home', 'usage:', 'home:attention/1', 'resource:resource/github/o/r/pull-request/1']) expect(opensInWeb(ref)).toBe(true)
    for (const ref of ['list:fleet', 'machine:machine/juniper', 'new-mission:']) expect(opensInWeb(ref)).toBe(false)
  })
})
