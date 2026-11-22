/** EXPLORATION: mission detail and list-row sparkline variants over invented data (a 30-hour mission with three server attempts, a failed load test, a five-hour wait on you and two plan revisions). Not wired to live data. */
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { liveLegacyTheme } from '../../ui-compat/live-theme.stylex.ts'
import { VariantStacked, VariantTabs, VariantTraceFirst } from './DetailVariants.tsx'
import { listMissions, longMission } from './example.ts'
import { C } from './parts.tsx'
import { RowCells, RowMiniTrace, RowTimeStrip } from './RowVariants.tsx'
import { RefinedB } from './RefinedB.tsx'
import { richMission } from './exampleRich.ts'
import { GithubReaderProvider, type GithubReader } from '../../plugins/github/data.ts'
import { conversationRead, checksRead, filesRead, issueRead, pullRequestRead, runRead, commentRead } from '../../plugins/github/fixtures.ts'

const Themed = ({ children }: { readonly children: React.ReactNode }) => <main data-scheme="dark" {...stylex.props(liveLegacyTheme, assistantDarkTheme)}>{children}</main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Missions/Explore',
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Exploration over invented mission data that no daemon serves yet; see explore/model.ts for the unserved fields.' } },
} satisfies Meta
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['DetailA_Tabs:contract-fixture', 'DetailA_TraceTab:contract-fixture', 'DetailB_Stacked:contract-fixture', 'DetailC_TraceFirst:contract-fixture', 'RefinedB_Definition:contract-fixture', 'RefinedB_Execution:contract-fixture', 'RefinedB_Replay:contract-fixture', 'Rows_Compare:contract-fixture']

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

// --- Variant B, refined after the second interview -------------------------------------------------------------------

const onOpenRef = fn()
/** GitHub reads answered from the plugin's normative fixtures, so linked work opens real hover cards. */
const githubFixtures: GithubReader = {
  item: async ref => ref.kind === 'pull-request' ? pullRequestRead : ref.kind === 'issue' ? issueRead : ref.kind === 'comment' ? commentRead : runRead,
  conversation: async () => conversationRead,
  checks: async () => checksRead,
  files: async () => filesRead,
}
const B = (props: Partial<React.ComponentProps<typeof RefinedB>>) =>
  <Themed><GithubReaderProvider value={githubFixtures}><RefinedB mission={richMission} onOpen={onOpenRef} {...props} /></GithubReaderProvider></Themed>

const setRange = (input: HTMLInputElement, value: number) => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, String(value))
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

export const RefinedB_Execution: Story = {
  render: () => <B />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Linked hover: hovering a step in the graph lights its lineage and dims the rest, in the graph and the trace.
    await userEvent.hover(canvasElement.querySelector('[data-dag-node="load"]')!)
    await waitFor(() => expect(canvasElement.querySelector('[data-dag-edge="server->load"]')).toHaveAttribute('data-lit', 'true'))
    await expect(canvasElement.querySelector('[data-dag-node="docs"]')).toHaveAttribute('data-dimmed', 'true')
    await expect(canvasElement.querySelector('[data-trace-row="docs"]')).toHaveAttribute('data-dimmed', 'true')
    await expect(canvasElement.querySelector('[data-trace-row="canary"]')).not.toHaveAttribute('data-dimmed')
    // The shared step card opens on the hovered step.
    await waitFor(() => expect(canvasElement.ownerDocument.querySelector('[data-hover-card] [data-testid="step-card"]')).toHaveTextContent('Load test at 3× peak'))
    await userEvent.unhover(canvasElement.querySelector('[data-dag-node="load"]')!)
    await waitFor(() => expect(canvasElement.querySelector('[data-dag-node="docs"]')).not.toHaveAttribute('data-dimmed'))
    // Brush the first eight hours: steps that did not run then dim.
    const axis = canvasElement.querySelector<HTMLElement>('[data-trace-axis]')!
    const box = axis.getBoundingClientRect()
    await userEvent.pointer([{ keys: '[MouseLeft>]', target: axis, coords: { clientX: box.left + 2, clientY: box.top + 5 } }, { target: axis, coords: { clientX: box.left + box.width * 0.22, clientY: box.top + 5 } }, { keys: '[/MouseLeft]', target: axis, coords: { clientX: box.left + box.width * 0.22, clientY: box.top + 5 } }])
    await expect(canvas.getByText(/–/, { selector: '[data-trace-brush-label]' })).toBeVisible()
    await expect(canvasElement.querySelector('[data-trace-row="web"]')).toHaveAttribute('data-dimmed', 'true')
    await expect(canvasElement.querySelector('[data-trace-row="server"]')).not.toHaveAttribute('data-dimmed')
    await userEvent.click(canvas.getByRole('button', { name: 'Clear range' }))
    // Agents open; linked work and conversation excerpts sit with the picked step.
    await userEvent.click(canvasElement.querySelector('[data-agent-chip="agent/web-dev"]')!)
    await expect(onOpenRef).toHaveBeenCalledWith('agent/web-dev')
    await expect(canvas.getByRole('complementary', { name: 'Implement the server side details' })).toHaveTextContent('Batch the backfill')
    await expect(canvasElement.querySelector('[data-mission-spend]')).toHaveTextContent('$')
  },
}

export const RefinedB_Replay: Story = {
  render: () => <B initialCursor={richMission.startedAt + 5 * 3_600_000} initialSelected="server" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // At 13:00 on day one the server step waits out its retry backoff, the SDK is running, the web step has not started.
    await expect(canvasElement.querySelector('[data-dag-node="server"]')).toHaveAttribute('data-state', 'waiting')
    await expect(canvasElement.querySelector('[data-dag-node="sdk"]')).toHaveAttribute('data-state', 'running')
    await expect(canvasElement.querySelector('[data-dag-node="web"]')).toHaveAttribute('data-state', 'pending')
    await expect(canvasElement.querySelector('[data-replay-cursor]')).not.toBeNull()
    // Scrub to day two: the security review waits on you.
    setRange(canvas.getByRole('slider', { name: 'Replay time' }) as HTMLInputElement, richMission.startedAt + 26 * 3_600_000)
    await waitFor(() => expect(canvasElement.querySelector('[data-dag-node="security"]')).toHaveAttribute('data-state', 'waiting-you'))
    // Left scrubbed, so the story shows a replayed moment; "Now" returns to the live state.
    await expect(canvas.getByRole('button', { name: 'Back to now' })).toBeVisible()
  },
}

export const RefinedB_Definition: Story = {
  render: () => <B initialTab="definition" />,
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByLabelText('Mission KDL')).toHaveTextContent('mission "platform/sync-v2"') },
}
