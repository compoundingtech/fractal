/** The sidebar's Missions tab in a fixed 256px column: grouped by state, project and agent; stale, truncated, loading, refused and empty. Synthetic fixture missions only. */
import type * as Fractal from '@fractal/client/schema'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, fn, userEvent, within } from 'storybook/test'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { surfaceVars as surface, textVars as ink } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { decodedRoster, observed, unavailable, waiting, type Feed, type RosterDecode } from '../data/source.ts'
import { defaultMissionsListView, type MissionsListView } from '../state/cells/lists.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { casMigration, commandPalette, dailyHealth, fixtureAttention, fixtureMissions, fixtureNow, flakeTriage, gatewaySchemaV1, rateLimits, runbooksRefresh, sdkInterop, terminalRendererBakeoff, vistaEmbed, webfractalFoundation, weeklyCycle } from './fixtures.ts'
import { sortMissions } from './missionWords.ts'
import { presentMission } from './presentation.ts'
import { MissionsList } from './SidebarMissionsList.tsx'

const fixtures: readonly Fractal.Mission[] = [webfractalFoundation, commandPalette, terminalRendererBakeoff, vistaEmbed, gatewaySchemaV1, sdkInterop, rateLimits, weeklyCycle, dailyHealth, casMigration, flakeTriage, runbooksRefresh]
const onOpen = fn()

const Surface = ({ feed, decode = decodedRoster, view = defaultMissionsListView, total }: { readonly feed: Feed<readonly Fractal.Mission[]>; readonly decode?: RosterDecode; readonly view?: MissionsListView; readonly total?: number }) => {
  const [current, setCurrent] = React.useState<string>()
  const [state, setState] = React.useState(view)
  const open = fixtureAttention.filter(item => item.state === 'open')
  const rows = feed._tag === 'Observed' ? sortMissions(feed.value.map(mission => ({ mission, presentation: presentMission({ mission, agents: [], attention: open }) }))) : []
  return <main data-scheme="dark" {...stylex.props(styles.canvas, liveLegacyTheme, assistantDarkTheme)}>
    <aside aria-label="Missions sidebar" {...stylex.props(styles.sidebar)}>
      <MissionsList missions={feed} rows={rows} observedIds={new Set(rows.map(row => row.mission.id))} decode={decode} now={fixtureNow} view={state} onViewChange={setState} {...(total === undefined ? {} : { total })} {...(current === undefined ? {} : { current })} onOpen={ref => { onOpen(ref); setCurrent(ref) }} />
    </aside>
  </main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Sidebar/Missions',
  component: Surface,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Fixture missions with exact words, steps and feed states pin the list rows, grouping, folding and every feed state.' } },
} satisfies Meta<typeof Surface>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['ByAgent:contract-fixture', 'ByProject:contract-fixture', 'Empty:contract-fixture', 'Loading:contract-fixture', 'Populated:contract-fixture', 'Stale:contract-fixture', 'Truncated:contract-fixture', 'TruncatedWithTotal:contract-fixture', 'Unavailable:contract-fixture']

const rowsOf = (canvas: HTMLElement) => [...canvas.querySelectorAll<HTMLElement>('[data-mission-row]')]
const groupKeys = (canvas: HTMLElement) => [...canvas.querySelectorAll<HTMLElement>('[data-mission-group-key]')].map(group => group.dataset.missionGroupKey)

export const Populated: Story = {
  args: { feed: observed({ value: fixtures }) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // State groups in reading order.
    const groups = groupKeys(canvasElement)
    await expect(groups[0]).toBe('state:needs_you')
    await expect(groups.indexOf('state:needs_you')).toBeLessThan(groups.indexOf('state:working'))
    const rows = rowsOf(canvasElement)
    await expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) await expect(row.getBoundingClientRect().height).toBe(48)
    // Titles are reader text, never machine hashes; only acting states are hued.
    for (const row of rows) await expect(row.querySelector('[data-row-field="title"]')!.textContent).not.toMatch(/[0-9a-f]{16,}/)
    await expect(canvasElement.querySelector('[data-mission-tone="needs-you"]')).not.toBeNull()
    await userEvent.click(rows[0]!)
    await expect(onOpen).toHaveBeenCalledWith(rows[0]!.dataset.missionRow)
    await expect(rowsOf(canvasElement)[0]).toHaveAttribute('data-current', 'true')
    // Collapsing a group hides its rows and keeps its header.
    await userEvent.click(canvas.getByRole('button', { name: /^Needs you/ }))
    await expect(canvas.getByRole('button', { name: /^Needs you/ })).toHaveAttribute('aria-expanded', 'false')
    await expect(canvasElement.querySelector('[data-mission-tone="needs-you"][data-mission-row]')).toBeNull()
    // Group and sort live in one menu.
    await userEvent.click(canvas.getByRole('button', { name: /^Group by state/ }))
    await userEvent.click(within(document.body).getByRole('menuitemradio', { name: 'Project' }))
    await expect(groupKeys(canvasElement)).toContain('project:webfractal')
  },
}
export const ByProject: Story = {
  args: { feed: observed({ value: fixtures }), view: { ...defaultMissionsListView, group: 'project', collapsed: [] } },
  play: async ({ canvasElement }) => {
    const keys = groupKeys(canvasElement)
    await expect(keys).toContain('project:webfractal')
    await expect(keys).toContain('project:platform')
  },
}
export const ByAgent: Story = {
  args: { feed: observed({ value: fixtures }), view: { group: 'agent', sort: 'progress', collapsed: [] } },
  play: async ({ canvasElement }) => { await expect(canvasElement.querySelectorAll('[data-mission-group-key^="agent:"]').length).toBeGreaterThan(1) },
}
export const Stale: Story = {
  args: { feed: observed({ value: fixtures, freshness: 'stale' }) },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector('[data-missions-stale]')).toHaveTextContent('last observed')
    await expect(canvasElement.querySelector('[data-mission-row][data-frozen]')).not.toBeNull()
  },
}
export const Truncated: Story = {
  args: { feed: observed({ value: fixtures }), decode: { ...decodedRoster, hasMore: true } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByRole('status')).toHaveTextContent(`First ${fixtures.length} missions`) },
}
export const TruncatedWithTotal: Story = {
  args: { feed: observed({ value: fixtures }), decode: { ...decodedRoster, hasMore: true }, total: fixtureMissions.length + 40 },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole('status')).toHaveTextContent(`Showing ${fixtures.length} of ${fixtureMissions.length + 40} missions`)
    await expect(canvasElement.querySelector('[data-missions-count]')).toHaveTextContent(`of ${fixtureMissions.length + 40}`)
  },
}
export const Loading: Story = { args: { feed: waiting }, play: async ({ canvasElement }) => { await expect(canvasElement.querySelector('[data-system-state="loading"]')).toHaveTextContent('Loading missions') } }
export const Unavailable: Story = { args: { feed: unavailable({ reason: 'ungranted', detail: 'Read access has not been granted.' }) }, play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText(/^No access to missions/)).toBeVisible() } }
export const Empty: Story = { args: { feed: observed({ value: [] }) }, play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText('No missions')).toBeVisible() } }

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg },
  sidebar: { display: 'flex', flexDirection: 'column', width: '256px', height: '720px', backgroundColor: surface.sidebar },
})
