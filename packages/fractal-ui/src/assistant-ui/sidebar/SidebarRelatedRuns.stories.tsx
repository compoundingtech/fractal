/** Related mission runs as served: waiting on you, late, stalled, active ETA, folded finished runs, truncation, loading and failure. Synthetic refs only. */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { SidebarRelatedRuns, type SidebarRelatedRun, type SidebarRelatedRunsState } from './SidebarRelatedRuns'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import { lightTheme, type Scheme } from '../composition-theme'

type Wait = NonNullable<SidebarRelatedRun['current_step']>
const storyNow = Date.UTC(2026, 10, 6, 9, 30, 0)
const eta = (lowMin: number, highMin: number) => ({ remaining_low_ms: lowMin * 60_000, remaining_high_ms: highMin * 60_000, basis: { method: 'critical-path' as const, samples: 12 }, as_of: storyNow - 60_000 })
const wait = (step: string, rest: Partial<Wait> = {}): Wait => ({ step, path: `step/${step}`, state: 'open', kind: 'step', ...rest })
const run = (slug: string, rest: Partial<SidebarRelatedRun>): SidebarRelatedRun => ({
  id: `mission-run/${slug}`, mission_id: `mission/${slug}`, status: 'running', status_chip: 'running', progress: { done: 3, total: 7 }, eta: null, waiting_on_you: false, late: false,
  deadline_at: null, completed_at: null, current_step: null, waits_on: [], urgency_rank: 3, folded: false, ...rest,
})
// Served order: person hold, late, stalled, active, then finished.
const rows: readonly SidebarRelatedRun[] = [
  run('release-notes', { status_chip: 'waiting-on-you', waiting_on_you: true, urgency_rank: 0, progress: { done: 2, total: 5 }, current_step: wait('outline', { title: 'Approve the page outline' }), waits_on: [wait('outline', { who: 'person/owner', kind: 'ask' })] }),
  run('billing-migration', { late: true, urgency_rank: 1, deadline_at: storyNow - 3_600_000, eta: eta(40, 75), progress: { done: 4, total: 9 }, current_step: wait('backfill', { title: 'Backfill the invoice ledger for every archived account' }) }),
  run('flaky-test-hunt', { status_chip: 'stalled', urgency_rank: 2, progress: { done: 1, total: 4 }, current_step: wait('bisect', { title: 'Bisect the failing run' }), waits_on: [wait('ci', { who: 'agent/ci-runner', kind: 'agent' })] }),
  run('sidebar-polish', { eta: eta(12, 18), current_step: wait('review', { title: 'Review hover scopes' }) }),
  run('docs-refresh', { status: 'completed', status_chip: 'completed', folded: true, urgency_rank: 4, progress: { done: 4, total: 4 }, completed_at: storyNow - 7_200_000 }),
  run('search-index', { status: 'failed', status_chip: 'failed', folded: true, urgency_rank: 4, progress: { done: 2, total: 6 }, completed_at: storyNow - 18_000_000 }),
]
const loaded = (value: readonly SidebarRelatedRun[], total = value.length): SidebarRelatedRunsState => ({ _tag: 'Loaded', value: { kind: 'agent-related-runs', agent_id: 'agent/example', as_of: storyNow, work_cut: 1, missions_cut: 1, total_rows: total, truncated: total > value.length, rows: value } })
const states: readonly (readonly [string, SidebarRelatedRunsState])[] = [
  ['loaded', loaded(rows)], ['truncated', loaded(rows.slice(2, 4), 140)], ['empty', loaded([])],
  ['loading', { _tag: 'Loading' }], ['failed', { _tag: 'Failed', message: 'the daemon did not answer' }],
]

function AllStatesHarness({ scheme }: { readonly scheme: Scheme }) {
  const [opened, setOpened] = React.useState('')
  return <main data-scheme={scheme} {...stylex.props(styles.canvas, scheme === 'light' && lightTheme)}>
    {states.map(([name, state]) => <div key={name} data-state={name}><SidebarRelatedRuns state={state} now={storyNow} onOpenRun={value => setOpened(value.id)} /></div>)}
    <output data-testid="opened">{opened}</output>
  </main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Sidebar/Related Runs',
  component: AllStatesHarness,
  args: { scheme: 'dark' },
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Served related-run rows have no catalog slice; synthetic rows exercise every chip, late and waiting-on-you, folding, truncation, loading and failure.' }, layout: 'fullscreen' },
} satisfies Meta<typeof AllStatesHarness>
export default meta
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['AllStates:contract-fixture', 'AllStatesLight:contract-fixture']

const section = (root: HTMLElement, name: string) => within(root.querySelector<HTMLElement>(`[data-state="${name}"]`)!)
export const AllStates: StoryObj<typeof meta> = { tags: ['critical', 'both-widths'], play: async ({ canvasElement }) => {
  const view = section(canvasElement, 'loaded')
  const active = view.getByRole('grid', { name: 'Related missions' })
  await expect([...active.querySelectorAll<HTMLElement>('[data-run]')].map(row => row.dataset.run)).toEqual(rows.filter(row => !row.folded).map(row => row.id))
  await expect(within(active).getByText('waiting on you')).toBeVisible()
  await expect(active.querySelectorAll('[data-late]')).toHaveLength(1)
  const fold = canvasElement.querySelector<HTMLElement>('[data-state="loaded"] [data-fold="finished"]')!
  await expect(fold).not.toHaveAttribute('open')
  await userEvent.click(view.getByText('Finished · 2'))
  await expect(fold).toHaveAttribute('open')
  await expect(view.getByRole('grid', { name: 'Related missions, finished in the last 24 hours' })).toBeVisible()
  await userEvent.click(within(active).getByText('sidebar-polish'))
  await expect(within(canvasElement).getByTestId('opened')).toHaveTextContent('mission-run/sidebar-polish')
  await expect(section(canvasElement, 'truncated').getByText('Showing 2 of 140 runs')).toBeVisible()
  await expect(section(canvasElement, 'empty').getByText('No related missions')).toBeVisible()
  await expect(section(canvasElement, 'loading').getByRole('status')).toHaveTextContent('Loading related missions')
  await expect(section(canvasElement, 'failed').getByRole('status')).toHaveTextContent('the daemon did not answer')
} }
export const AllStatesLight: StoryObj<typeof meta> = { ...AllStates, args: { scheme: 'light' } }

const styles = stylex.create({
  canvas: { display: 'flex', flexDirection: 'column', gap: s.lg, width: g.sidebarDefault, maxWidth: '100%', boxSizing: 'border-box', padding: s.sm, minHeight: '100vh', backgroundColor: surface.sidebar, color: ink.fg },
})
