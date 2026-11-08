import { DateTime } from 'effect'
import { describe, expect, it } from 'vitest'

import { fixtureAttention, fixtureMissions, webfractalFoundation } from './fixtures.ts'
import { missionPlan } from './missionPlan.ts'
import { presentMission } from './presentation.ts'
import { groupMissionRows, missionChildOf, missionProject, missionRowData, sortMissionRows, STATE_GROUPS } from './rowData.ts'

const attention = fixtureAttention.filter(item => item.state === 'open')
const rows = fixtureMissions.map(mission => ({ mission, presentation: presentMission({ mission, agents: [], attention }) }))

describe('mission rows', () => {
  it('groups by state in reading order and drops empty groups', () => {
    const groups = groupMissionRows(rows, { group: 'state', sort: 'activity' })
    const order = groups.map(group => STATE_GROUPS.indexOf(group.key.slice('state:'.length) as never))
    expect(order).toEqual([...order].sort((a, b) => a - b))
    expect(groups.every(group => group.rows.length > 0)).toBe(true)
    expect(groups.reduce((sum, group) => sum + group.rows.length, 0)).toBe(rows.length)
    expect(groups[0]!.key).toBe('state:needs_you')
  })

  it('groups by project from the mission path, and by lead agent with no-agent last', () => {
    expect(missionProject('mission/webfractal/foundation')).toBe('webfractal')
    expect(missionProject('mission/spontaneous-work/0123456789abcdef0123')).toBe('Ad hoc work')
    const byAgent = groupMissionRows(rows, { group: 'agent', sort: 'activity' })
    expect(byAgent.at(-1)!.key).toBe('agent:')
  })

  it('sorts by activity, newest first', () => {
    const sorted = sortMissionRows(rows, 'activity').map(row => DateTime.toEpochMillis(row.mission.updated_at))
    expect(sorted).toEqual([...sorted].sort((a, b) => b - a))
  })

  it('hues only missions that need the person or failed, and never shows a hash title', () => {
    for (const row of rows) {
      const data = missionRowData(row)
      expect(data.title).not.toMatch(/[0-9a-f]{16,}/)
      expect(['needs-you', 'failed', 'working', 'waiting', 'settled']).toContain(data.tone)
    }
    const foundation = missionRowData(rows.find(row => row.mission.id === webfractalFoundation.id)!)
    expect(foundation.tone).toBe('needs-you')
    expect(missionChildOf(rows.find(row => row.mission.id === webfractalFoundation.id)!).slug).toBe('webfractal/foundation')
  })

  it('orders the plan by dependency level, not by state', () => {
    const row = rows.find(item => item.mission.id === webfractalFoundation.id)!
    const plan = missionPlan(row.mission, row.presentation)
    expect(plan.levels.length).toBeGreaterThan(1)
    for (const edge of plan.edges) {
      const from = plan.steps.find(step => step.path === edge.from)!
      const to = plan.steps.find(step => step.path === edge.to)!
      expect(from.level).toBeLessThan(to.level)
    }
  })
})
