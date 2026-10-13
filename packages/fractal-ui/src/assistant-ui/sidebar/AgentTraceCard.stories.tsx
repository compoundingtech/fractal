/** Sidebar trace card (layout B), opened from a SidebarAgentRow by hover or keyboard focus. Every story uses synthetic fixtures; each chart renders only for a Known series. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { SidebarAgentRow } from './SidebarAgentRow'
import type { SidebarAgentRow as Row } from './model'
import type { AgentTrace, ContextBucket, SpendPoint } from './trace-model'
import { baselineTheme } from '../neutral-theme'
import { lightTheme, type Scheme } from '../composition-theme'
import { ThemePortal } from '../taste/ThemePortal'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

const now = Date.UTC(2026, 9, 8, 12, 0, 0)
const hour = 3_600_000
const range = { start: now - 24 * hour, end: now }
const windowTokens = 200_000
/** 96 quarter-hour buckets: two compactions reset the climb, a model change, and two idle stretches with no reading. */
const buckets: readonly ContextBucket[] = Array.from({ length: 96 }, (_, index) => {
  const start = range.start + index * 15 * 60_000
  const idle = (index >= 30 && index < 34) || (index >= 70 && index < 74)
  const phase = index < 22 ? index : index < 50 ? index - 22 : index - 50
  const base = index < 22 ? 30_000 : index < 50 ? 28_000 : 34_000
  return { start, end: start + 15 * 60_000, windowTokens, ...(idle ? {} : { usedTokens: Math.min(194_000, base + phase * 3_400 + (index % 3) * 1_200) }) }
})
const spendPoints: readonly SpendPoint[] = Array.from({ length: 49 }, (_, index) => ({ at: range.start + index * 30 * 60_000, sessionUsd: Number((71.35 * Math.pow(index / 48, 0.8)).toFixed(2)), subagentsUsd: Number((6.05 * Math.pow(index / 48, 1.6)).toFixed(2)) }))
const fullTrace: AgentTrace = {
  range,
  context: { _tag: 'Known', value: { buckets, marks: [{ at: range.start + 22 * 15 * 60_000, kind: 'compaction' }, { at: range.start + 50 * 15 * 60_000, kind: 'compaction' }, { at: range.start + 60 * 15 * 60_000, kind: 'model-change', model: 'model-b' }] } },
  contextReading: { usedTokens: 144_000, windowTokens, compactions: 2 },
  spend: { _tag: 'Known', value: spendPoints },
  spendTotals: { _tag: 'Known', value: { sessionUsd: 71.35, subagentsUsd: 6.05 } },
  tokens: { _tag: 'Known', value: { input: 3_834_000, cacheRead: 14_058_000, cacheWrite: 1_491_000, output: 1_917_000 } },
  activity: { turns: 41, toolCalls: 318, lastTool: 'bash' },
  subagents: { _tag: 'Known', value: [{ ref: 'agent/fx-a1/subagent/fx-s1', name: 'explore', state: 'running', startedAt: now - 3 * 60_000, spendUsd: 4.1 }, { ref: 'agent/fx-a1/subagent/fx-s2', name: 'reviewer', state: 'running', startedAt: now - 9 * 60_000, spendUsd: 1.95 }] },
}
const baseRow: Row = {
  ref: 'agent/fx-a1', id: 'fx-a1', title: 'Storage migration', description: 'Moves the fixture store to the new layout',
  status: 'working', statusLabel: 'Working', host: 'host-1', harness: 'omp', model: 'model-a',
  statusSince: now - 12 * 60_000, lastActivityAt: now - 30_000, needsMe: false, unread: 0, freshness: 'live',
  usage: { _tag: 'Known', scope: '24h-root-and-subagents', usd: 77.4, tokens: 21_300_000 },
  duration: { _tag: 'Known', scope: '24h-activity-span', ms: 6 * hour },
  lastTurn: { _tag: 'Known', kind: 'turn-completed', at: now - 60_000 },
  branch: 'feat/storage-layout', pullRequest: { ref: 'agent/fx-a1/pr', title: 'Move the fixture store', number: 101, state: 'open' },
  children: [],
  trace: fullTrace,
}
const notRetained = { _tag: 'Unknown', reason: 'not-retained' } as const
const rows = {
  full: baseRow,
  noHistory: { ...baseRow, ref: 'agent/fx-a2', id: 'fx-a2', title: 'Inbox triage', status: 'idle', statusLabel: 'Idle', trace: { ...fullTrace, context: notRetained, spend: notRetained, spendTotals: notRetained, tokens: notRetained, activity: {}, subagents: { _tag: 'Known', value: [] } } },
  pending: { ...baseRow, ref: 'agent/fx-a3', id: 'fx-a3', title: 'Backlog sweep', trace: { ...fullTrace, context: { _tag: 'Pending' }, spend: { _tag: 'Pending' }, spendTotals: { _tag: 'Pending' }, tokens: { _tag: 'Pending' }, activity: {}, subagents: { _tag: 'Pending' } } },
  claude: { ...baseRow, ref: 'agent/fx-a4', id: 'fx-a4', title: 'Release notes pass', harness: 'claude', model: 'model-c', usage: { _tag: 'Unknown' }, branch: 'docs/release-notes', pullRequest: undefined, trace: { ...fullTrace, context: { _tag: 'Unknown', reason: 'no-provider' }, contextReading: { usedTokens: 92_000, windowTokens, compactions: 1 }, spend: { _tag: 'Unknown', reason: 'no-provider' }, spendTotals: { _tag: 'Unknown', reason: 'no-provider' }, tokens: { _tag: 'Unknown', reason: 'no-provider' }, activity: {}, subagents: { _tag: 'Known', value: [{ ref: 'agent/fx-a4/subagent/fx-s3', name: 'general-purpose', state: 'running', startedAt: now - 5 * 60_000 }] } } },
  subagents: { ...baseRow, ref: 'agent/fx-a5', id: 'fx-a5', title: 'Deploy coordinator', status: 'waiting', statusLabel: 'Needs you', needsMe: true, trace: { ...fullTrace, subagents: { _tag: 'Known', value: [
    { ref: 'agent/fx-a5/subagent/fx-s4', name: 'explore', state: 'running', startedAt: now - 3 * 60_000, spendUsd: 4.1 },
    { ref: 'agent/fx-a5/subagent/fx-s5', name: 'reviewer', state: 'needs-you', startedAt: now - 9 * 60_000, spendUsd: 1.95 },
    { ref: 'agent/fx-a5/subagent/fx-s6', name: 'migration-check', state: 'failed', startedAt: now - 2 * hour, spendUsd: 0.62 },
    { ref: 'agent/fx-a5/subagent/fx-s7', name: 'docs', state: 'done', startedAt: now - 5 * hour },
  ] } } },
  unattributed: { ...baseRow, ref: 'agent/fx-a6', id: 'fx-a6', title: 'Shared pool worker', usage: { _tag: 'Unknown' }, trace: { ...fullTrace, spend: { _tag: 'Unknown', reason: 'not-attributed' }, spendTotals: { _tag: 'Unknown', reason: 'not-attributed' }, subagents: { _tag: 'Known', value: [{ ref: 'agent/fx-a6/subagent/fx-s8', name: 'explore', state: 'running', startedAt: now - 4 * 60_000 }] } } },
} satisfies Record<string, Row>

type Args = { readonly scheme: Scheme }
function Sidebar({ row, scheme }: { readonly row: Row; readonly scheme: Scheme }) {
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <nav aria-label="Agents" data-frame-rows {...stylex.props(styles.sidebar)}><SidebarAgentRow item={row} now={now} actions={{ select: () => undefined }} /></nav>
  </ThemePortal></main>
}

/** WCAG relative-luminance contrast of `rgb()/rgba()` colors, compositing alpha over the background. */
function parse(color: string): readonly [number, number, number, number] {
  const parts = color.match(/[\d.]+/g)?.map(Number) ?? []
  return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0, parts[3] ?? 1]
}
function contrast(fg: string, bg: string): number {
  const [br, bgG, bb] = parse(bg)
  const [fr, fgG, fb, alpha] = parse(fg)
  const mix = [fr * alpha + br * (1 - alpha), fgG * alpha + bgG * (1 - alpha), fb * alpha + bb * (1 - alpha)]
  const luminance = (rgb: readonly number[]) => { const [r, gr, b] = rgb.map(value => { const c = value / 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4) }); return 0.2126 * r! + 0.7152 * gr! + 0.0722 * b! }
  const [hi, lo] = [luminance(mix), luminance([br, bgG, bb])].sort((a, b) => b - a)
  return (hi! + 0.05) / (lo! + 0.05)
}
/** Green hue band (90°–160°) with visible saturation; the card must contain none. */
function isGreen(color: string): boolean {
  const [r, gr, b, alpha] = parse(color)
  if (alpha === 0 || !/^rgba?\(/.test(color)) return false
  const max = Math.max(r, gr, b), min = Math.min(r, gr, b)
  if (max === 0 || (max - min) / max < 0.25 || max !== gr) return false
  const hue = 60 * ((b - r) / (max - min) + 2)
  return hue >= 90 && hue <= 160
}

async function openCard(canvasElement: HTMLElement, via: 'hover' | 'focus'): Promise<HTMLElement> {
  const row = within(canvasElement).getByTestId('taste-agent-row')
  await expect(within(document.body).queryByTestId('agent-trace-card')).toBeNull()
  if (via === 'hover') await userEvent.hover(row)
  else { await userEvent.tab(); await expect(row.contains(document.activeElement)).toBe(true) }
  // The tooltip opens after the 150 ms hover intent; assert only on the re-rendered card.
  return waitFor(() => { const card = within(document.body).getByTestId('agent-trace-card'); expect(card).toBeVisible(); return card }, { timeout: 2000 })
}

async function assertCard(card: HTMLElement, row: Row) {
  await expect(card.closest('[role="tooltip"]')).not.toBeNull()
  await expect(card.getBoundingClientRect().width).toBeLessThanOrEqual(Math.min(360, window.innerWidth) + 0.5)
  await expect(card.querySelectorAll('[title]')).toHaveLength(0)
  await expect(card.textContent).not.toMatch(/unknown/i)
  await expect(card.textContent).not.toMatch(/\$0\.00/)
  await expect(within(card).getByText(row.title)).toBeVisible()
  const background = getComputedStyle(card).backgroundColor
  const texts = [...card.querySelectorAll<HTMLElement>('*')].filter(element => [...element.childNodes].some(node => node.nodeType === Node.TEXT_NODE && node.textContent!.trim() !== ''))
  await expect(texts.length).toBeGreaterThan(3)
  for (const element of texts) {
    const ratio = contrast(getComputedStyle(element).color, background)
    if (ratio < 4.5) throw new Error(`Contrast ${ratio.toFixed(2)} < 4.5 for "${element.textContent}"`)
  }
  for (const element of [card, ...card.querySelectorAll<Element>('*')]) {
    const style = getComputedStyle(element)
    for (const color of [style.color, style.backgroundColor, style.fill, style.stroke, style.borderTopColor, style.borderInlineStartColor]) if (isGreen(color)) throw new Error(`Green ${color} on ${element.tagName}`)
  }
  for (const chart of within(card).queryAllByRole('img')) await expect(chart.getAttribute('aria-label')?.length ?? 0).toBeGreaterThan(8)
  // Negative controls: the checks above reject a known-failing pair and a known green.
  await expect(contrast('rgb(129, 129, 129)', 'rgb(122, 122, 122)')).toBeLessThan(4.5)
  await expect(isGreen('rgb(34, 197, 94)')).toBe(true)
}

const meta = {
  title: 'Fractal UI/Sidebar/Agent Trace Card',
  parameters: { layout: 'fullscreen', docs: { description: { component: 'Layout B hover card: header, 24 h context histogram, cumulative spend (session vs subagents), token split and compact facts. A chart renders only for a Known series; otherwise its section shows the scalar reading and the muted reason. Opens on row hover and keyboard focus through the row hover-card trigger.' } } },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<Args>
export default meta
type Story = StoryObj<Args>

export const Full: Story = {
  render: args => <Sidebar row={rows.full} scheme={args.scheme} />,
  play: async ({ canvasElement }) => {
    const card = await openCard(canvasElement, 'hover')
    await assertCard(card, rows.full)
    const view = within(card)
    await expect(view.getByRole('img', { name: /^Context 72% of 200K, peak 97%, 2 compactions, 1 model change, 2 idle gaps/ })).toBeVisible()
    await expect(card.querySelectorAll('[data-trace-mark="compaction"]')).toHaveLength(2)
    await expect(card.querySelectorAll('[data-trace-mark="model-change"]')).toHaveLength(1)
    await expect(view.getByRole('img', { name: 'Cumulative spend over the last 24 hours: session $71.35, subagents $6.05' })).toBeVisible()
    await expect(view.getByRole('img', { name: /^Tokens 21\.3M: input 18%, cache read 66%, cache write 7%, output 9%$/ })).toBeVisible()
    await expect(view.getByText('41 turns · 318 tool calls · last tool bash')).toBeVisible()
    await expect(view.getByText('feat/storage-layout · PR #101')).toBeVisible()
    const table = view.getByRole('table', { name: 'Subagents · 2 running' })
    await expect(within(table).getAllByRole('row')).toHaveLength(3)
    await expect(within(table).getAllByRole('columnheader').map(header => header.textContent)).toEqual(['Name', 'State', 'Spend', 'Age'])
    // Shared grid tracks: every row's Spend cell starts at the same x.
    const spendLefts = within(table).getAllByRole('row').map(row => Math.round(row.children[2]!.getBoundingClientRect().left))
    await expect(new Set(spendLefts).size).toBe(1)
    await expect(view.queryByText('History not retained')).toBeNull()
  },
}

export const NoHistory: Story = {
  render: args => <Sidebar row={rows.noHistory} scheme={args.scheme} />,
  play: async ({ canvasElement }) => {
    const card = await openCard(canvasElement, 'focus')
    await assertCard(card, rows.noHistory)
    const view = within(card)
    await expect(view.queryAllByRole('img').filter(element => element.tagName.toLowerCase() === 'svg')).toHaveLength(0)
    await expect(card.querySelector('[data-trace-chart]')).toBeNull()
    await expect(view.getAllByText('History not retained')).toHaveLength(3)
    // Option-A scalar rows replace the charts.
    const context = card.querySelector<HTMLElement>('[data-trace-section="context"]')!
    await expect(context.textContent).toBe('Context72% · 144K / 200K · 2 compactionsHistory not retained')
    await expect(card.querySelector('[data-trace-section="spend"]')!.textContent).toContain('$77.40 · 24h, includes subagents')
    await expect(card.querySelector('[data-trace-section="tokens"]')!.textContent).toContain('Tokens · 21.3M')
    await expect(card.querySelector('[data-trace-section="subagents"]')).toBeNull()
  },
}

export const Pending: Story = {
  render: args => <Sidebar row={rows.pending} scheme={args.scheme} />,
  play: async ({ canvasElement }) => {
    const card = await openCard(canvasElement, 'hover')
    await assertCard(card, rows.pending)
    const view = within(card)
    await expect(view.getByRole('img', { name: 'Loading context history for Backlog sweep' })).toHaveAttribute('aria-busy', 'true')
    await expect(view.getByRole('img', { name: 'Loading spend history for Backlog sweep' })).toBeVisible()
    await expect(view.getByRole('img', { name: 'Loading token split for Backlog sweep' })).toBeVisible()
    await expect(card.querySelector('[data-trace-chart]')).toBeNull()
    await expect(view.queryByText('History not retained')).toBeNull()
    await expect(view.getByRole('status')).toHaveTextContent('Loading trace history')
  },
}

export const ClaudeSeat: Story = {
  render: args => <Sidebar row={rows.claude} scheme={args.scheme} />,
  play: async ({ canvasElement }) => {
    const card = await openCard(canvasElement, 'focus')
    await assertCard(card, rows.claude)
    const view = within(card)
    await expect(view.getByText(/model-c · claude · host-1/)).toBeVisible()
    await expect(card.querySelector('[data-trace-section="context"]')!.textContent).toBe('Context46% · 92K / 200K · 1 compactionHistory not retained')
    // Usage is unreported for this seat: the spend section names the gap instead of showing $0.
    await expect(card.querySelector('[data-trace-section="spend"]')!.textContent).toBe('SpendHistory not retained')
    await expect(card.querySelector('[data-trace-section="tokens"]')!.textContent).toBe('TokensHistory not retained')
    const table = view.getByRole('table', { name: 'Subagents · 1 running' })
    await expect(within(table).getByRole('cell', { name: 'Not attributed' })).toBeVisible()
    await expect(view.queryByText('Activity')).toBeNull()
  },
}

export const WithSubagents: Story = {
  render: args => <Sidebar row={rows.subagents} scheme={args.scheme} />,
  play: async ({ canvasElement }) => {
    const card = await openCard(canvasElement, 'hover')
    await assertCard(card, rows.subagents)
    const table = within(card).getByRole('table', { name: 'Subagents · 1 running' })
    const body = within(table).getAllByRole('row').slice(1)
    await expect(body.map(row => [...row.children].map(cell => cell.textContent))).toEqual([
      ['explore', 'running', '$4.10', '3m'],
      ['reviewer', 'needs you', '$1.95', '9m'],
      ['migration-check', 'failed', '$0.62', '2h'],
      ['docs', 'done', 'Not attributed', '5h'],
    ])
    for (const cell of within(table).getAllByRole('cell').filter((_, index) => index % 4 >= 2)) await expect(getComputedStyle(cell).textAlign).toBe('end')
  },
}

export const Unattributed: Story = {
  render: args => <Sidebar row={rows.unattributed} scheme={args.scheme} />,
  play: async ({ canvasElement }) => {
    const card = await openCard(canvasElement, 'focus')
    await assertCard(card, rows.unattributed)
    const spend = card.querySelector<HTMLElement>('[data-trace-section="spend"]')!
    await expect(spend.textContent).toBe('SpendSpend not attributed to this agent')
    await expect(spend.querySelector('[data-trace-chart]')).toBeNull()
    // The other series stay Known, so their charts still render.
    await expect(card.querySelector('[data-trace-chart="context"]')).not.toBeNull()
    await expect(card.querySelector('[data-trace-chart="tokens"]')).not.toBeNull()
    await expect(within(card).getByRole('cell', { name: 'Not attributed' })).toBeVisible()
  },
}

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  sidebar: { width: g.sidebarDefault, maxWidth: '100%', backgroundColor: surface.sidebar },
})
