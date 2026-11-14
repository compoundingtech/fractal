/** The sidebar's Missions tab in a fixed 256px column: populated, truncated, loading, refused and empty. Synthetic fixture missions only. */
import type * as Fractal from '@fractal/client/schema'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, fn, userEvent, within } from 'storybook/test'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { surfaceVars as surface, textVars as ink } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { decodedRoster, observed, unavailable, waiting, type Feed, type RosterDecode } from '../data/source.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { commandPalette, dailyHealth, fixtureNow, gatewaySchemaV1, rateLimits, sdkInterop, terminalRendererBakeoff, vistaEmbed, webfractalFoundation, weeklyCycle } from './fixtures.ts'
import { sortMissions } from './missionWords.ts'
import { presentMission } from './presentation.ts'
import { MissionsList } from './SidebarMissionsList.tsx'

const fixtures: readonly Fractal.Mission[] = [webfractalFoundation, commandPalette, terminalRendererBakeoff, vistaEmbed, gatewaySchemaV1, sdkInterop, rateLimits, weeklyCycle, dailyHealth]
const onOpen = fn()

const Surface = ({ feed, decode = decodedRoster }: { readonly feed: Feed<readonly Fractal.Mission[]>; readonly decode?: RosterDecode }) => {
  const [current, setCurrent] = React.useState<string>()
  const rows = feed._tag === 'Observed' ? sortMissions(feed.value.map(mission => ({ mission, presentation: presentMission({ mission, agents: [], attention: [] }) }))) : []
  return <main data-scheme="dark" {...stylex.props(styles.canvas, liveLegacyTheme, assistantDarkTheme)}>
    <aside aria-label="Missions sidebar" {...stylex.props(styles.sidebar)}>
      <MissionsList missions={feed} rows={rows} observedIds={new Set(rows.map(row => row.mission.id))} decode={decode} now={fixtureNow} {...(current === undefined ? {} : { current })} onOpen={ref => { onOpen(ref); setCurrent(ref) }} />
    </aside>
  </main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Sidebar/Missions',
  component: Surface,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Fixture missions with exact words, steps and feed states pin the list row, its folding and every feed state.' } },
} satisfies Meta<typeof Surface>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Empty:contract-fixture', 'Loading:contract-fixture', 'Populated:contract-fixture', 'Truncated:contract-fixture', 'Unavailable:contract-fixture']

export const Populated: Story = {
  args: { feed: observed({ value: fixtures }) },
  play: async ({ canvasElement }) => {
    const list = within(canvasElement).getByRole('grid', { name: 'Missions' })
    const rows = await within(list).findAllByRole('row')
    await expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) await expect(row.getBoundingClientRect().height).toBe(48)
    await userEvent.click(rows[0]!)
    await expect(onOpen).toHaveBeenCalled()
    await expect(rows[0]).toHaveAttribute('data-current', 'true')
  },
}
export const Truncated: Story = {
  args: { feed: observed({ value: fixtures }), decode: { ...decodedRoster, hasMore: true } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByRole('status')).toHaveTextContent(`First ${fixtures.length} missions`) },
}
export const Loading: Story = { args: { feed: waiting }, play: async ({ canvasElement }) => { await expect(canvasElement.querySelector('[data-system-state="loading"]')).toHaveTextContent('Loading missions') } }
export const Unavailable: Story = { args: { feed: unavailable({ reason: 'ungranted', detail: 'Read access has not been granted.' }) }, play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText(/^No access to missions/)).toBeVisible() } }
export const Empty: Story = { args: { feed: observed({ value: [] }) }, play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText('No missions')).toBeVisible() } }

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg },
  sidebar: { display: 'flex', flexDirection: 'column', width: '256px', height: '560px', backgroundColor: surface.sidebar },
})
