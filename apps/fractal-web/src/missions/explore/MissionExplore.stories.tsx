/** EXPLORATION: mission detail and list-row sparkline variants over invented data (a 30-hour mission with three server attempts, a failed load test, a five-hour wait on you and two plan revisions). Not wired to live data. */
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, userEvent, within } from 'storybook/test'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { liveLegacyTheme } from '../../ui-compat/live-theme.stylex.ts'
import { VariantStacked, VariantTabs, VariantTraceFirst } from './DetailVariants.tsx'
import { listMissions, longMission } from './example.ts'
import { C } from './parts.tsx'
import { RowCells, RowMiniTrace, RowTimeStrip } from './RowVariants.tsx'

const Themed = ({ children }: { readonly children: React.ReactNode }) => <main data-scheme="dark" {...stylex.props(liveLegacyTheme, assistantDarkTheme)}>{children}</main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Missions/Explore',
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Exploration over invented mission data that no daemon serves yet; see explore/model.ts for the unserved fields.' } },
} satisfies Meta
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['DetailA_Tabs:contract-fixture', 'DetailA_TraceTab:contract-fixture', 'DetailB_Stacked:contract-fixture', 'DetailC_TraceFirst:contract-fixture', 'Rows_Compare:contract-fixture']

export const DetailA_Tabs: Story = {
  render: () => <Themed><VariantTabs mission={longMission} /></Themed>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Selection is shared: pick a step in the graph, then the trace shows it selected.
    await userEvent.click(canvasElement.querySelector('[data-dag-node="server"]')!)
    await expect(canvas.getByRole('complementary', { name: 'Implement the server side details' })).toHaveTextContent('Attempt 3')
    await userEvent.click(canvas.getByRole('tab', { name: /Trace/ }))
    await expect(canvasElement.querySelector('[data-trace-row="server"]')).toHaveAttribute('aria-selected', 'true')
  },
}
export const DetailA_TraceTab: Story = { render: () => <Themed><VariantTabs mission={longMission} initialTab="trace" /></Themed> }
export const DetailB_Stacked: Story = {
  render: () => <Themed><VariantStacked mission={longMission} /></Themed>,
  play: async ({ canvasElement }) => {
    await userEvent.click(canvasElement.querySelector('[data-trace-row="load"] button')!)
    await expect(canvasElement.querySelector('[data-dag-node="load"]')).toHaveAttribute('aria-pressed', 'true')
  },
}
export const DetailC_TraceFirst: Story = { render: () => <Themed><VariantTraceFirst mission={longMission} /></Themed> }

const RowColumn = ({ title, Row }: { readonly title: string; readonly Row: (props: { readonly mission: typeof longMission }) => React.ReactNode }) =>
  <section aria-label={title} style={{ width: 256, background: 'var(--sidebar, #111)', borderRadius: 8, paddingBlock: 6 }}>
    <h2 style={{ margin: '4px 12px 8px', fontSize: 11, fontWeight: 600, color: C.muted }}>{title}</h2>
    <div role="grid" aria-label={title}>{listMissions.map(mission => <Row key={mission.id} mission={mission} />)}</div>
  </section>

export const Rows_Compare: Story = {
  render: () => <Themed><div style={{ display: 'flex', flexWrap: 'wrap', gap: 24, padding: 24, background: C.canvas, minHeight: '100vh', fontFamily: 'system-ui, sans-serif' }}>
    <RowColumn title="R1 · step cells" Row={RowCells} />
    <RowColumn title="R2 · time strip" Row={RowTimeStrip} />
    <RowColumn title="R3 · mini trace" Row={RowMiniTrace} />
  </div></Themed>,
  play: async ({ canvasElement }) => {
    for (const row of canvasElement.querySelectorAll<HTMLElement>('[role="row"]')) await expect(row.getBoundingClientRect().height).toBe(48)
  },
}
