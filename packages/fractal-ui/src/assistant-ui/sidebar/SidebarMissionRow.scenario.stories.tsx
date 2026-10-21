import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice, type WireSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { expect, waitFor, within } from 'storybook/test'
import { MilestoneAgentRow } from './MilestoneAgentRow'
import { statuses, type AgentStatus, type SidebarAgentRow as Row, type SidebarMissionContext } from './model'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { lightTheme, type Scheme } from '../composition-theme'
import { ThemePortal } from '../taste/ThemePortal'

type WireAgent = WireSlice<'roster'>['state']['agents'][number]
const agentStatus = (state: WireAgent['state']): AgentStatus => {
  switch (state) {
    case 'running': return 'working'
    case 'waiting': return 'waiting'
    case 'desired': case 'starting': return 'pending'
    case 'down': return 'offline'
    case 'stopped': case 'failed': return 'ended'
    case 'suspended': return 'suspended'
    case 'retired': return 'retired'
    default: return 'unobserved'
  }
}
const lastSegment = (id: string) => id.slice(id.lastIndexOf('/') + 1)

/** A roster carries work labels, but no hydrated run summaries or attention: those fields stay unknown. */
const scenarioRows = ({ loading, state }: WireSlice<'roster'>): readonly Row[] => {
  if (loading) return []
  const hosts = new Map(state.machines.map(machine => [machine.host_id, machine.name]))
  const rank = new Map(state.order.map((id, index) => [id, index]))
  return [...state.agents].sort((a, b) => (rank.get(a.id) ?? state.order.length) - (rank.get(b.id) ?? state.order.length)).map((agent): Row => {
    const status = agentStatus(agent.state)
    const current = (agent.current_work ?? [])[0]
    const queued = new Map<string, { readonly id: string; readonly slug: string }>()
    for (const work of agent.upcoming_work ?? []) {
      if (work.mission_id !== current?.mission_id && !queued.has(work.mission_id)) queued.set(work.mission_id, { id: work.mission_id, slug: lastSegment(work.mission_id) })
    }
    const missionContext: SidebarMissionContext = current === undefined ? { _tag: 'None' }
      : { _tag: 'Mission', id: current.mission_id, slug: lastSegment(current.mission_id), progress: { _tag: 'Unknown' }, step: { _tag: 'Step', name: current.title || current.path }, queued: [...queued.values()] }
    return {
      ref: agent.id, id: agent.id, title: agent.name, status, statusLabel: statuses[status].label,
      host: agent.host_id == null ? 'Unknown host' : hosts.get(agent.host_id) ?? agent.host_id,
      freshness: 'live', usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [],
      ...(current === undefined ? {} : { mission: current.mission_id }), missionContext,
    }
  })
}

function ScenarioRosterStory({ scheme }: { readonly scheme: Scheme }) {
  const roster = useScenarioSlice('roster', scenarioRows)
  return <main data-testid="scenario-mission-roster" {...stylex.props(styles.root, scheme === 'light' && lightTheme)}>
    <ThemePortal><section aria-label="Scenario roster" {...stylex.props(styles.column)}>
      {roster.map(item => <MilestoneAgentRow key={item.ref} row={item} />)}
    </section></ThemePortal>
  </main>
}

type Args = { readonly scheme: Scheme; readonly roster?: string }
const meta = {
  title: 'Fractal UI/Sidebar/Mission row scenario',
  component: ScenarioRosterStory,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'slices', slices: ['roster'] } },
  args: { scheme: 'dark' },
  argTypes: { ...scenarioArgTypes(['roster']), scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<Args>
export default meta

export const ScenarioRoster: StoryObj<Args> = {
  name: 'Scenario roster · current mission work',
  play: async ({ canvasElement }) => {
    const roster = await within(canvasElement).findByRole('region', { name: 'Scenario roster' })
    await waitFor(() => expect(roster.querySelectorAll('[data-milestone-row]').length).toBeGreaterThan(0))
    await expect(roster.querySelectorAll('[data-mission-pill]').length).toBeGreaterThan(0)
    for (const pill of roster.querySelectorAll('[data-mission-pill]')) {
      await expect(pill.closest('[data-row-column="line2"]')?.querySelector('[data-row-field="step"]')).not.toBeNull()
      await expect(pill.querySelector('[data-mission-progress]')).toBeNull()
    }
  },
}

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  column: { display: 'flex', flexDirection: 'column', gap: s.xs2, width: g.sidebarDefault, boxSizing: 'border-box', padding: s.md, backgroundColor: surface.sidebar },
})
