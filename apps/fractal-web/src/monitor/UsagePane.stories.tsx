import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { ANCHOR_MS } from '@fractal/scenarios'
import { useScenarioNow, useScenarioSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { assistantDarkTheme, liveAccentTheme, compositionLightTheme } from '@fractal/ui/assistant-ui'
import { colorVars as color, typeVars as type } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { scenarioUsageSource, usageReads, type UsageRead } from './usageFixtures.ts'
import { UsagePane } from './UsagePane.tsx'

interface UsageArgs { readonly reads: readonly UsageRead[]; readonly scheme: 'dark' | 'light'; readonly objectives?: string }

const Specimen = ({ read }: { readonly read: UsageRead }) => {
  const slice = useScenarioSlice('objectives')
  const now = useScenarioNow()
  const [registry] = React.useState(() => AtomRegistry.make())
  const source = React.useMemo(() => scenarioUsageSource({ state: slice.state, loading: slice.loading, read, now, registry }), [slice, read, now, registry])
  return <div data-usage-read={read} {...stylex.props(styles.frame)}><DataSourceProvider source={source} registry={registry}><UsagePane label={`Usage · ${read}`} /></DataSourceProvider></div>
}
const UsageSurface = ({ reads, scheme }: UsageArgs) =>
  <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'dark' && assistantDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
    {reads.map(read => <Specimen key={read} read={read} />)}
  </main>

const meta = {
  title: 'Fractal/App/UsagePane',
  tags: ['critical'],
  component: UsagePane,
  render: UsageSurface,
  args: { reads: ['live'], scheme: 'dark' },
  globals: { scenarioNow: ANCHOR_MS },
  argTypes: { ...scenarioArgTypes(['objectives']), scheme: { control: 'radio', options: ['dark', 'light'] }, reads: { table: { disable: true } } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'slices', slices: ['objectives'] } },
} satisfies Meta<UsageArgs>
export default meta
type Story = StoryObj<typeof meta>
const pane = (canvas: HTMLElement, read: UsageRead) => within(canvas.querySelector<HTMLElement>(`[data-usage-read="${read}"]`)!)

export const AllStates: Story = {
  args: { reads: usageReads },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    for (const read of usageReads) await expect(canvas.getByRole('region', { name: `Usage · ${read}`, exact: true })).toBeVisible()
    for (const read of ['live', 'stale'] as const) {
      for (const group of ['budgets', 'objectives', 'accounts']) {
        await expect(canvas.getByRole('region', { name: `Usage · ${read}: ${group}`, exact: true })).toBeVisible()
      }
    }
    await waitFor(() => expect(pane(canvasElement, 'live').getByRole('heading', { name: 'budgets 4' })).toBeVisible())
    await expect(pane(canvasElement, 'waiting').getByText('Loading objectives and budgets…')).toBeVisible()
    const staleNotices = pane(canvasElement, 'stale').getAllByText('Last reading: could not read again (Could not read objectives and budgets.)')
    await expect(staleNotices).toHaveLength(2)
    for (const notice of staleNotices) await expect(notice).toBeVisible()
    await expect(pane(canvasElement, 'stale').getByRole('heading', { name: 'objectives 4' })).toBeVisible()
    await expect(pane(canvasElement, 'failed').getByText('Could not read objectives and budgets')).toBeVisible()
    await expect(pane(canvasElement, 'unsupported').getByText(/needs an update to show objectives and budgets/)).toBeVisible()
    canvasElement.dataset.proof = 'passed'
  },
}

export const Populated: Story = {
  play: async ({ canvasElement }) => {
    const usage = within(canvasElement)
    const body = await usage.findByTestId('usage-pane')
    // Served tones only: an unknown word is quiet, and every tone shows.
    expect([...new Set([...body.querySelectorAll('[data-tone]')].map(row => row.getAttribute('data-tone')))].sort()).toEqual(['green', 'quiet', 'red', 'yellow'])
    expect(body.querySelector('[data-slo="transcript-freshness"]')!.getAttribute('data-tone')).toBe('quiet')
    await expect(usage.getByText('out in 41m')).toBeVisible()
    await expect(usage.getByText('88% by reset')).toBeVisible()
    const link = usage.getByRole('link', { name: 'History in OpenTelemetry ›' })
    expect(link.getAttribute('href')).toBe('https://otel.example.com/explore?service=fractal')
    expect(link.getAttribute('target')).toBe('_blank')
    expect(link.getAttribute('rel')).toBe('noopener noreferrer')
    const row = usage.getByRole('button', { name: /^ada-claude\/5-hour/ })
    await userEvent.click(row)
    expect(row.getAttribute('aria-expanded')).toBe('true')
    await expect(usage.getByText('runs out in 41m, before it resets (from the last 1h)')).toBeVisible()
    await expect(usage.getByText('allowance left, last 24 hours')).toBeVisible()
    const chart = body.querySelector('[data-chart]')!.textContent!
    expect([...chart]).toHaveLength(48)
    expect(chart.slice(20, 24)).toBe('····')
    await expect(usage.getByText(/low 38% · latest 38% · 1320 readings · a column is its lowest reading · a dot had no reading/)).toBeVisible()
    // One row open at a time; a row with no readings says so instead of drawing zero.
    await userEvent.click(usage.getByRole('button', { name: /^vista-publish/ }))
    expect(row.getAttribute('aria-expanded')).toBe('false')
    await expect(usage.getByText('No readings in this period yet.')).toBeVisible()
    await expect(usage.getByText('error budget left, last 24 hours')).toBeVisible()
    await expect(usage.getByText('credits $128 of $200 this month')).toBeVisible()
    expect(body.scrollWidth).toBeLessThanOrEqual(body.clientWidth)
    canvasElement.dataset.proof = 'passed'
  },
}

export const Empty: Story = {
  args: { objectives: 'none' },
  play: async ({ canvasElement }) => {
    await expect(await within(canvasElement).findByText(/No objectives or budgets/)).toBeVisible()
    canvasElement.dataset.proof = 'passed'
  },
}

export const UnsafeLink: Story = {
  args: { objectives: 'unsafe-link' },
  play: async ({ canvasElement }) => {
    const usage = within(canvasElement)
    await expect(await usage.findByRole('heading', { name: 'budgets 4' })).toBeVisible()
    expect(usage.queryByRole('link')).toBeNull()
    expect(canvasElement.querySelector('a[href^="javascript"]')).toBeNull()
    canvasElement.dataset.proof = 'passed'
  },
}

export const Retry: Story = {
  args: { reads: ['failed'] },
  parameters: { scenario: { _tag: 'slices', slices: ['objectives'], invariant: { objectives: 'A failed first read shows no served rows until Retry reads again' } } },
  play: async ({ canvasElement }) => {
    const usage = within(canvasElement)
    await userEvent.click(await usage.findByRole('button', { name: 'Retry: Could not read objectives and budgets' }))
    await expect(await usage.findByRole('heading', { name: 'budgets 4' })).toBeVisible()
    await expect(usage.getByText('robin/codex-pro')).toBeVisible()
    canvasElement.dataset.proof = 'passed'
  },
}

const styles = stylex.create({
  root: { minHeight: '100vh', display: 'grid', gridAutoRows: 'minmax(420px, 1fr)', gap: 1, backgroundColor: color.canvas, color: color.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading },
  frame: { position: 'relative', minWidth: 0 },
})
