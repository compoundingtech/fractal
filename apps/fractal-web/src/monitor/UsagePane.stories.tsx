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
import { accountOf, accounts, FIXTURE_NOW, fixtureSource, historyEnvelope, quotaEnvelope, type Scripted } from './fixtures.ts'
import { scenarioUsageSource, usageReads, type UsageRead } from './usageFixtures.ts'
import { UsagePane } from './UsagePane.tsx'
import type { QuotaAccount, UsageHistoryEnvelope } from './wire.ts'

const HOUR = 3_600_000
const atLimit = accountOf({ harness: 'codex', provider: 'openai-codex', name: 'team-north', label: 'Team North', plan: 'Pro',
  limits: [{ id: 'week', label: 'Weekly', slot: 'governing', fraction: 1, exhausted: true, resetsIn: 4 * 24 * HOUR }, { id: 'five', label: '5 Hour', slot: 'pressure-1', kind: 'rolling', fraction: 0.12, resetsIn: 2 * HOUR }],
  resets: { availableCount: 1, credits: [{ grantedAt: null, expiresAt: new Date(FIXTURE_NOW + 6 * 24 * HOUR).toISOString(), status: 'available' }] } })
const held = accountOf({ harness: 'codex', provider: 'openai-codex', name: 'held', label: 'Held', plan: 'Plus', retained: { observedAgo: 3 * HOUR, reason: 'provider answered 401' },
  limits: [{ id: 'week', label: 'Weekly', slot: 'governing', fraction: 0.74, resetsIn: 2 * 24 * HOUR }] })
const quotaOf = (list: readonly QuotaAccount[], generatedAgo?: number) => fixtureSource({ quota: [{ _tag: 'ok', json: quotaEnvelope({ list, ...(generatedAgo === undefined ? {} : { generatedAgo }) }) }] })
const dayAgo = (days: number) => new Date(FIXTURE_NOW - days * 24 * HOUR).toISOString().slice(0, 10)
/** The producer reported these days' tokens but no rate-card dollars. */
const withoutRate = (envelope: UsageHistoryEnvelope, daysAgo: readonly number[]): UsageHistoryEnvelope =>
  ({ ...envelope, rows: envelope.rows.map(row => daysAgo.some(days => row.bucket_start.startsWith(dayAgo(days))) ? { ...row, usd: null } : row) })
/** Made-up v3 quota envelopes for the quota accounts at the top of the pane, at the fixture clock. */
const quotas = {
  'at limit': quotaOf([accounts.claudeAlpha, atLimit, accounts.codexAlpha]),
  stale: quotaOf([held, accounts.claudeBravo], 2 * HOUR),
  empty: quotaOf([]),
  history: fixtureSource({
    quota: [{ _tag: 'ok', json: quotaEnvelope({ list: [accounts.claudeAlpha, accounts.claudeBravo, accounts.codexAlpha, accounts.codexBravo, accounts.copilotMain, accounts.zaiMain, atLimit] }) }],
    history: (ledgerAccountId): readonly Scripted[] => {
      switch (ledgerAccountId) {
        case 'anthropic/alpha': return [{ _tag: 'ok', json: historyEnvelope({ ledgerAccountId, days: 40, coverageOk: false, idle: [2, 3] }) }]
        case 'anthropic/bravo': return [{ _tag: 'fail', failure: { kind: 'unavailable', reason: 'history returned HTTP 404' } }]
        case 'openai-codex/alpha': return [{ _tag: 'ok', json: { lens: 'usage_over_time', rows: 'none' } }]
        case 'openai-codex/bravo': return [{ _tag: 'fail', failure: { kind: 'http', reason: 'history returned HTTP 502' } }]
        case 'github-copilot/main': return [{ _tag: 'ok', json: historyEnvelope({ ledgerAccountId, days: 3 }) }]
        case 'zai/main': return [{ _tag: 'ok', json: withoutRate(historyEnvelope({ ledgerAccountId, days: 40, idle: [4] }), [5, 6]) }]
        default: return [{ _tag: 'hang' }]
      }
    },
  }),
}

interface UsageArgs { readonly reads: readonly UsageRead[]; readonly scheme: 'dark' | 'light'; readonly objectives?: string; readonly quota?: keyof typeof quotas }

const Specimen = ({ read, quota }: { readonly read: UsageRead; readonly quota: UsageArgs['quota'] }) => {
  const slice = useScenarioSlice('objectives')
  const now = useScenarioNow()
  const [registry] = React.useState(() => AtomRegistry.make())
  const source = React.useMemo(() => ({ ...scenarioUsageSource({ state: slice.state, loading: slice.loading, read, now, registry }), ...(quota === undefined ? {} : { usage: quotas[quota] }) }), [slice, read, now, registry, quota])
  return <div data-usage-read={read} {...stylex.props(styles.frame)}><DataSourceProvider source={source} registry={registry}><UsagePane label={`Usage · ${read}`} /></DataSourceProvider></div>
}
const UsageSurface = ({ reads, scheme, quota }: UsageArgs) =>
  <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'dark' && assistantDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
    {reads.map(read => <Specimen key={read} read={read} quota={quota} />)}
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
} satisfies Meta<UsageArgs & React.ComponentProps<typeof UsagePane>>
export default meta
type Story = StoryObj<typeof meta>
const pane = (canvas: HTMLElement, read: UsageRead) => within(canvas.querySelector<HTMLElement>(`[data-usage-read="${read}"]`)!)

export const AllStates: Story = {
  tags: ['both-widths'],
  args: { reads: usageReads },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    for (const read of usageReads) await expect(canvas.getByRole('region', { name: `Usage · ${read}` })).toBeVisible()
    for (const read of ['live', 'stale'] as const) {
      for (const group of ['budgets', 'objectives', 'accounts']) {
        await expect(canvas.getByRole('region', { name: `Usage · ${read}: ${group}` })).toBeVisible()
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
  tags: ['both-widths'],
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
    const row = usage.getByRole('button', { name: 'ada/claude-max 5-hour limit' })
    await expect(within(row).getByText('ada/claude-max 5-hour limit')).toBeVisible()
    await userEvent.click(row)
    expect(row.getAttribute('aria-expanded')).toBe('true')
    await expect(usage.getByText('runs out in 41m, before it resets (from the last 1h)')).toBeVisible()
    await expect(usage.getByText('allowance left, last 24 hours')).toBeVisible()
    const chart = body.querySelector('[data-chart]')!.textContent!
    expect([...chart]).toHaveLength(48)
    expect(chart.slice(20, 24)).toBe('····')
    await expect(usage.getByText(/low 38% · latest 38% · 1320 readings · a column is its lowest reading · a dot had no reading/)).toBeVisible()
    // One row open at a time; a row with no readings says so instead of drawing zero.
    await userEvent.click(usage.getByRole('button', { name: 'Vista publications land' }))
    expect(row.getAttribute('aria-expanded')).toBe('false')
    await expect(usage.getByText('No readings in this period yet.')).toBeVisible()
    await expect(usage.getByText('error budget left, last 24 hours')).toBeVisible()
    await expect(usage.getByText(/^credits \$128 of \$200 · .* UTC$/)).toBeVisible()
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

const detailOf = (canvas: HTMLElement) => canvas.querySelector<HTMLElement>('[data-usage-detail]')
// The detail paints a commit before the GridList builds its rows; a click on a row not there yet lands on the body.
const accountRow = (canvas: HTMLElement, id: string) => waitFor(() => {
  const row = canvas.querySelector<HTMLElement>(`[data-usage-account="${id}"]`)
  expect(row).not.toBeNull()
  return row!
})

/** The worst account opens on top with every limit; picking another row, by pointer or arrow key, changes it. */
export const QuotaAtLimit: Story = {
  args: { quota: 'at limit' },
  globals: { scenarioNow: FIXTURE_NOW },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(detailOf(canvasElement)).toHaveAttribute('data-usage-detail', 'codex/team-north'))
    const detail = within(detailOf(canvasElement)!)
    await expect(detail.getByText('100%!')).toBeVisible()
    await expect(detail.getByText('at limit · resets 4d')).toBeVisible()
    await expect(detail.getByText('12%')).toBeVisible()
    await expect(await within(canvasElement).findByText('Daily spend, USD · last 30 days')).toBeVisible()
    expect(canvasElement.querySelectorAll('[data-usage-day]')).toHaveLength(30)
    expect(canvasElement.querySelector('[data-usage-day]:last-child')).toHaveAttribute('data-open', 'true')
    await expect(detail.getByText('1 banked reset · expires in 6d')).toBeVisible()
    await expect(detail.getByText('Reading, 42s old')).toBeVisible()
    await userEvent.click(await accountRow(canvasElement, 'claude/alpha'))
    await waitFor(() => expect(detailOf(canvasElement)).toHaveAttribute('data-usage-detail', 'claude/alpha'))
    await expect(within(detailOf(canvasElement)!).getAllByRole('listitem')).toHaveLength(3)
    await userEvent.keyboard('{ArrowUp}')
    await waitFor(() => expect(detailOf(canvasElement)).not.toHaveAttribute('data-usage-detail', 'claude/alpha'))
    canvasElement.dataset.proof = 'passed'
  },
}

/** A snapshot past its horizon says so; a held account names its age and the producer's reason. */
export const QuotaStale: Story = {
  args: { quota: 'stale' },
  globals: { scenarioNow: FIXTURE_NOW },
  play: async ({ canvasElement }) => {
    const usage = within(canvasElement)
    await expect(await usage.findByText('Last reading, 2h old')).toBeVisible()
    await userEvent.click(await accountRow(canvasElement, 'codex/held'))
    const detail = within(detailOf(canvasElement)!)
    await expect(detail.getByText('Held reading, 3h old')).toBeVisible()
    await expect(detail.getByText('Last attempt: indeterminate 40s ago: provider answered 401')).toBeVisible()
    canvasElement.dataset.proof = 'passed'
  },
}

export const QuotaEmpty: Story = {
  args: { quota: 'empty' },
  globals: { scenarioNow: FIXTURE_NOW },
  play: async ({ canvasElement }) => {
    await expect(await within(canvasElement).findByText(/The quota source reports no accounts/)).toBeVisible()
    expect(detailOf(canvasElement)).toBeNull()
    canvasElement.dataset.proof = 'passed'
  },
}

/** Each history state under the selected account: tokens below USD coverage, none relayed, unreadable, failed, still reading. */
export const QuotaHistory: Story = {
  args: { quota: 'history' },
  globals: { scenarioNow: FIXTURE_NOW },
  play: async ({ canvasElement }) => {
    const usage = within(canvasElement)
    await waitFor(() => expect(detailOf(canvasElement)).not.toBeNull())
    for (const [account, text] of [
      ['claude/alpha', 'Daily tokens · last 30 days'],
      ['claude/bravo', 'No account history here'],
      ['codex/alpha', 'History is in a shape this app cannot read'],
      ['codex/bravo', 'Could not read history'],
      ['codex/team-north', 'Reading history…'],
      ['copilot/main', '27 days not reported'],
      ['zai/main', '2 without a rate'],
    ] as const) {
      await userEvent.click(await accountRow(canvasElement, account))
      await expect(await usage.findByText(text, { exact: false })).toBeVisible()
    }
    expect(canvasElement.querySelectorAll('[data-state="unrated"]')).toHaveLength(2)
    expect(canvasElement.querySelectorAll('[data-state="zero"]')).toHaveLength(1)
    await userEvent.click(await accountRow(canvasElement, 'copilot/main'))
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-state="unreported"]')).toHaveLength(27))
    await userEvent.click(await accountRow(canvasElement, 'claude/alpha'))
    await expect(await usage.findByText('USD coverage is below the producer’s gate, so this shows tokens')).toBeVisible()
    canvasElement.dataset.proof = 'passed'
  },
}

const styles = stylex.create({
  root: { minHeight: '100vh', display: 'grid', gridAutoRows: 'minmax(420px, 1fr)', gap: 1, backgroundColor: color.canvas, color: color.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading },
  frame: { position: 'relative', minWidth: 0 },
})
