/** Baseline SR-2 row coverage: known-only hover card facts, omitted unreported fields, an aligned known-vs-none time cohort, and the row hover quick Open action. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, within } from 'storybook/test'
import { AgentHoverCard, SidebarAgentRow } from './SidebarAgentRow'
import type { SidebarAgentRow as Row } from './model'
import { spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t, geometryVars as g, borderVars as border, radiusVars as r, elevationVars as elevation } from '../composition-tokens.stylex'

const storyNow = Date.UTC(2026, 9, 8, 12, 0, 0)
/** Fully reported synthetic identity; every optional fact the card can render is present. */
const knownRow: Row = {
  ref: 'agent/sample', id: 'sample', title: 'Sample session', description: 'Reviews the selection model',
  status: 'working', statusLabel: 'Working', host: 'host-1', harness: 'seat-1', model: 'glm-5',
  statusSince: storyNow - 90_000, lastActivityAt: storyNow - 30_000, needsMe: true, unread: 2, freshness: 'live',
  usage: { _tag: 'Known', scope: '24h-root-and-subagents', usd: 1.84, tokens: 412_000 },
  duration: { _tag: 'Known', scope: '24h-activity-span', ms: 3_600_000 },
  lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 300_000 },
  branch: 'feat/rows', worktree: '~/rows',
  pullRequest: { ref: 'agent/sample/pr', title: 'Align row cohort', number: 42, state: 'open' },
  children: [],
}
/** Same identity, nothing reported beyond title, status, host and counts; every optional fact is omitted. */
const unreportedRow: Row = { ...knownRow, ref: 'agent/unreported', id: 'unreported', title: 'Unreported session', usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, description: undefined, model: undefined, harness: undefined, pullRequest: undefined, branch: undefined, worktree: undefined, terminal: undefined, mission: undefined, statusSince: undefined, lastActivityAt: undefined }
/** Synthetic cohort: every second row carries no last-turn fact, so its time slot renders empty while the shared metric track keeps every title aligned. */
const cohortRows: readonly Row[] = [
  { ...knownRow, ref: 'agent/first', id: 'first', title: 'First session' },
  { ...knownRow, ref: 'agent/second', id: 'second', title: 'Second session', lastTurn: { _tag: 'Unknown' }, needsMe: false, unread: 0 },
  { ...knownRow, ref: 'agent/third', id: 'third', title: 'Third session', status: 'waiting', statusLabel: 'Needs you' },
  { ...knownRow, ref: 'agent/fourth', id: 'fourth', title: 'Fourth session', lastTurn: { _tag: 'Unknown' }, usage: { _tag: 'Unknown' } },
  { ...unreportedRow, ref: 'agent/fifth', id: 'fifth', title: 'Fifth session' },
  { ...knownRow, ref: 'agent/sample/subagent/child', id: 'sample', parentRef: 'agent/sample', title: 'Check output', lastTurn: { _tag: 'Unknown' }, unread: undefined, description: undefined },
]

function CardPair({ row }: { readonly row: Row }) {
  return <div {...stylex.props(styles.spread)}>
    <div data-frame-rows {...stylex.props(styles.list)}><SidebarAgentRow item={row} now={storyNow} active /></div>
    <div {...stylex.props(styles.aside)}>
      <div data-testid="hover-facts"><AgentHoverCard row={row} now={storyNow} /></div>
    </div>
  </div>
}

async function assertCompactCard(canvasElement: HTMLElement) {
  const cards = within(canvasElement).getAllByTestId('agent-hover-card')
  await expect(cards).toHaveLength(1)
  const card = cards[0]!
  await expect(card.getBoundingClientRect().width).toBeLessThanOrEqual(400)
  const labels = [...card.querySelectorAll('dt')].map(label => label.textContent)
  await expect(new Set(labels).size).toBe(labels.length)
  for (const label of ['Name', 'ID', 'Description', 'Last activity', 'Worktree', 'Lease expires']) await expect(labels).not.toContain(label)
  await expect(card.querySelectorAll('li').length).toBeLessThanOrEqual(3)
  await expect(card.textContent).not.toMatch(/\d{4}-\d{2}-\d{2}T\d{2}:/)
}
function CohortList() {
  return <div data-frame-rows {...stylex.props(styles.list)}>{cohortRows.map((row, index) => <SidebarAgentRow key={row.ref} item={row} now={storyNow} active={index === 0} />)}</div>
}

function QuickOpenCohort() {
  const [openedRef, setOpenedRef] = React.useState<string | undefined>()
  const open = React.useCallback((row: Row) => setOpenedRef(row.ref), [])
  const select = React.useCallback((id: string) => setOpenedRef(`agent/${id}`), [])
  return <div data-frame-rows {...stylex.props(styles.list)}>{cohortRows.slice(0, 3).map(row => <SidebarAgentRow key={row.ref} item={row} now={storyNow} actions={{ select }} onOpen={open} active={openedRef === row.ref} />)}</div>
}

const meta = {
  title: 'Fractal UI/Sidebar/Agent Row',
  component: SidebarAgentRow,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'Baseline agent row: reported facts only. The hover card and reported details omit unreported fields instead of showing placeholders; hovering a row swaps the time slot for a quick Open action; rows without a reported time keep the shared metric track so titles stay aligned.' } } },
} satisfies Meta
export default meta
type Story = StoryObj
export const HoverCardKnown: Story = { name: 'Hover card · fully reported', render: () => <main {...stylex.props(styles.root)}><CardPair row={knownRow} /></main>, play: async ({ canvasElement }) => assertCompactCard(canvasElement) }
export const HoverCardUnknown: Story = { name: 'Hover card · unreported fields omitted', render: () => <main {...stylex.props(styles.root)}><CardPair row={unreportedRow} /></main>, play: async ({ canvasElement }) => {
  const card = within(canvasElement).getByTestId('hover-facts')
  await assertCompactCard(canvasElement)
  await expect(card.textContent).not.toMatch(/unknown|unavailable|—/i)
  for (const label of ['Spend', 'Duration', 'Last turn', 'Model', 'PR', 'Branch']) await expect(within(card).queryByText(label)).toBeNull()
} }
export const TimeCohort: Story = { name: 'Time cohort · known vs none', render: () => <main {...stylex.props(styles.root)}><CohortList /></main>, play: async ({ canvasElement }) => {
  const rows = within(canvasElement).getAllByTestId('taste-agent-row')
  const child = rows.find(row => row.textContent?.includes('Check output'))!
  await expect(child).toBeDefined()
  await expect(child.querySelector('[data-row-field="last-turn"]')).toBeNull()
  const titleLeft = rows[0]!.querySelector('[data-row-column="title-text"]')!.getBoundingClientRect().left
  for (const row of rows) await expect(Math.abs(row.querySelector('[data-row-column="title-text"]')!.getBoundingClientRect().left - titleLeft)).toBeLessThanOrEqual(1)
} }
export const QuickOpenAction: Story = { name: 'Row hover · quick Open', render: () => <main {...stylex.props(styles.root)}><QuickOpenCohort /></main>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const rows = canvas.getAllByTestId('taste-agent-row')
  const target = rows[1]!
  await document.fonts.ready
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await userEvent.hover(target)
  await userEvent.click(within(target).getByRole('button', { name: /^Open / }))
  await expect(target.querySelector('[aria-current="page"]')).not.toBeNull()
} }
export const AllStates: Story = { render: () => <main {...stylex.props(styles.root)}><CardPair row={knownRow} /><CardPair row={unreportedRow} /><CohortList /><div data-frame-rows {...stylex.props(styles.list)}>{contextRowElements}</div><MetricWidths /></main> }

/** The row's accessible description, whichever ARIA attribute carries it. */
const rowDescription = (element: Element): string => element.getAttribute('aria-description') ?? document.getElementById(element.getAttribute('aria-describedby') ?? '')?.textContent ?? ''
const contextRows: readonly Row[] = [
  { ...unreportedRow, ref: 'agent/context-known', id: 'context-known', title: 'Reported occupancy', context: { _tag: 'Known', usedTokens: 61_206, windowTokens: 1_000_000, usedPercent: 6.1206, observedAt: storyNow - 30_000 } },
  { ...unreportedRow, ref: 'agent/context-zero', id: 'context-zero', title: 'Reported empty context', context: { _tag: 'Known', usedTokens: 0, windowTokens: 200_000, usedPercent: 0, observedAt: storyNow - 30_000 } },
  { ...unreportedRow, ref: 'agent/context-percent', id: 'context-percent', title: 'Reported percentage only', context: { _tag: 'Known', usedPercent: 42, observedAt: storyNow - 30_000 } },
  { ...unreportedRow, ref: 'agent/context-stale', id: 'context-stale', title: 'Retained stale occupancy', freshness: 'stale', context: { _tag: 'Known', usedTokens: 61_206, windowTokens: 1_000_000, usedPercent: 6.1206, observedAt: storyNow - 30_000 } },
  { ...unreportedRow, ref: 'agent/context-unknown', id: 'context-unknown', title: 'Occupancy unreported', context: { _tag: 'Unknown' } },
]
const contextRowElements = contextRows.map(row => <SidebarAgentRow key={row.ref} item={row} now={storyNow} extraSignals={[]} onOpen={() => undefined} />)
export const ContextOccupancy: Story = {
  render: () => <main {...stylex.props(styles.root)}><div data-frame-rows {...stylex.props(styles.list)}>{contextRowElements}</div></main>,
  play: async ({ canvasElement }) => {
    const rows = within(canvasElement).getAllByTestId('taste-agent-row')
    const signal = rows[0]!.querySelector<HTMLElement>('[data-row-field="context-percent"]')!
    const assertOccupancy = async () => { await expect(signal).toHaveTextContent('Context 6%') }
    await assertOccupancy()
    await expect(rowDescription(within(rows[0]!).getByRole('button', { name: /^Reported occupancy/ }))).toContain('Reported main-session context: 6%; 61,206 used tokens; 1,000,000 token window; as of 2026-10-08T11:59:30.000Z')
    await expect(rows[1]!.querySelector('[data-row-field="context-percent"]')).toHaveTextContent('Context 0%')
    await expect(rowDescription(within(rows[2]!).getByRole('button', { name: /^Reported percentage only/ }))).toContain('used tokens unreported; window unreported')
    await expect(rowDescription(within(rows[3]!).getByRole('button', { name: /^Retained stale occupancy/ }))).toContain('Last observed main-session context')
    await expect(rows[4]!.querySelector('[data-row-field="context-percent"]')).toBeNull()
    await expect(rowDescription(within(rows[4]!).getByRole('button', { name: /^Occupancy unreported/ }))).not.toContain('main-session context')
    // No native tooltip carries the occupancy; the description does.
    await expect(canvasElement.querySelector('[data-row-field="context-percent"][title], [data-row-field="context-percent"][aria-label]')).toBeNull()
    // Negative control: a cumulative-token ratio must not pass the occupancy display assertion.
    const text = signal.textContent
    try { signal.textContent = 'Context 4800%'; await expect(assertOccupancy()).rejects.toThrow() }
    finally { signal.textContent = text }
  },
}

const metricRow: Row = { ...knownRow, ref: 'agent/metric', id: 'metric', context: { _tag: 'Known', usedPercent: 18, observedAt: storyNow }, spend: {
  last24h: { _tag: 'Known', usd: 1.84, since: storyNow - 86_400_000, until: storyNow, observedAt: storyNow },
  last7d: { _tag: 'Known', usd: 9.25, since: storyNow - 604_800_000, until: storyNow, observedAt: storyNow },
} }
const metricWidths = [256, 320] as const
function MetricWidths() {
  return <section data-metric-widths {...stylex.props(styles.metricPair)}>{metricWidths.map(width => <div key={width} data-metric-width={width} {...stylex.props(styles.metricFrame(width))}>
    {[metricRow, unreportedRow].map(row => <SidebarAgentRow key={row.ref} item={row} now={storyNow} variant={width < 300 ? 'SR-1' : 'SR-2'} layout="SR2-C" metricLayout={width < 300 ? 'compact' : 'expanded'} extraSignals={['X-needs']} onOpen={() => undefined} />)}
  </div>)}</section>
}
export const LayoutCWidths: Story = {
  render: () => <main {...stylex.props(styles.root)}><MetricWidths /></main>,
  play: async ({ canvasElement }) => {
    const [compact, compactUnknown] = canvasElement.querySelectorAll<HTMLElement>('[data-metric-width="256"] [data-testid="taste-agent-row"]')
    const [expanded, expandedUnknown] = canvasElement.querySelectorAll<HTMLElement>('[data-metric-width="320"] [data-testid="taste-agent-row"]')
    const assertDensity = async () => {
      await expect(compact!.getBoundingClientRect().height).toBe(32)
      await expect(expanded!.getBoundingClientRect().height).toBe(48)
    }
    await assertDensity()
    await expect(compact!.querySelector('[data-row-column="metrics"]')).toBeNull()
    await expect(compact!.querySelector('[data-row-context-ring]')).toHaveAttribute('aria-hidden', 'true')
    await expect(compact!.querySelector('[data-row-field="last-24h-usd"]')).toHaveTextContent('$1.8')
    const metrics = expanded!.querySelector('[data-row-column="metrics"]')!
    await expect(metrics).toHaveTextContent('24h $1.8')
    await expect(metrics).toHaveTextContent('7d $9.3')
    await expect(expanded!.querySelector('[data-row-field="context-percent"]')).toHaveTextContent('Context 18%')
    await expect(expanded!.querySelector('[data-row-field="last-activity"]')).not.toBeNull()
    // Expanded rows move every metric to line two; the time column keeps only the hover quick Open host.
    await expect(expanded!.querySelector('[data-row-column="time"] [data-row-field]')).toBeNull()
    await userEvent.hover(expanded!)
    await expect(await within(expanded!).findByRole('button', { name: 'Open Sample session' })).toBeInTheDocument()
    await userEvent.unhover(expanded!)
    await expect(rowDescription(within(expanded!).getByRole('button', { name: /^Sample session/ }))).toContain('24h API-equivalent spend including subagents: $1.84')
    // Unreported windows and occupancy are omitted, never rendered as a placeholder or zero.
    const unknownMetrics = expandedUnknown!.querySelector('[data-row-column="metrics"]')!
    await expect(unknownMetrics.querySelector('[data-row-field="last-24h-usd"], [data-row-field="context-percent"]')).toBeNull()
    await expect(unknownMetrics.textContent ?? '').not.toMatch(/\$|\?|unknown/i)
    await expect(compactUnknown!.querySelector('[data-row-field="last-24h-usd"]')).toBeNull()
    // Negative control: the old identical 48px layout at 256px must fail the same geometry proof.
    compact!.style.height = '48px'
    try { await expect(assertDensity()).rejects.toThrow() } finally { compact!.style.removeProperty('height') }
  },
}

const styles = stylex.create({
  root: { height: '100vh', minHeight: 0, overflowY: 'auto', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, display: 'flex', flexDirection: 'column', gap: s.xl },
  spread: { display: 'flex', alignItems: 'flex-start', flexWrap: 'wrap', gap: s.xl },
  list: { display: 'flex', flexDirection: 'column', gap: s.xs, width: g.specimenAside, minWidth: 0 },
  aside: { width: g.specimenAside, maxWidth: '100%', minWidth: 0, boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, boxShadow: elevation.popover },
  metricPair: { display: 'flex', flexWrap: 'wrap', gap: s.lg, padding: s.md, backgroundColor: surface.sidebar, color: ink.fg },
  metricFrame: (width: number) => ({ flexShrink: 0, width }),
})
