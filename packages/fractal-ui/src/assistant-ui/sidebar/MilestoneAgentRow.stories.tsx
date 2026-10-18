/** RM1 two-line agent row: G3 glyph states live and frozen, the 48px two-line anatomy, nesting inside the kit SidebarTree, and the hover card. */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { Key } from 'react-aria-components'
import { MilestoneAgentRow, openAgentRowMenu } from './MilestoneAgentRow'
import { SidebarTree, type SidebarTreeItem } from './SidebarTree'
import { statuses, type AgentStatus, type SidebarAgentRow as Row } from './model'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

const storyNow = Date.UTC(2026, 9, 17, 9, 30, 0)
const base: Row = {
  ref: 'agent/base', id: 'base', title: 'Base', status: 'idle', statusLabel: 'Idle', host: 'build-1', freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [],
}
const stateOrder = Object.keys(statuses) as AgentStatus[]
/** One row per canonical status; every row carries a last turn 3 minutes before the story clock. */
const stateRows: readonly Row[] = stateOrder.map(status => ({ ...base, ref: `agent/${status}`, id: status, title: `${statuses[status].label} agent`, status, statusLabel: statuses[status].label, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 180_000 } }))

const subagent: Row = { ...base, id: 'parent', parentRef: 'agent/parent', host: 'build-2' }
const parent: Row = {
  ...base, ref: 'agent/parent', id: 'parent', title: 'Align the metric track across every row', status: 'working', statusLabel: 'Working', needsMe: true, host: 'build-2',
  usage: { _tag: 'Known', scope: 'lifetime-incl-subagents', usd: 3.42, tokens: 1_180_000 }, duration: { _tag: 'Known', scope: 'lifetime', ms: 2_460_000 },
  lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 95_000 },
  children: [
    { ...subagent, ref: 'agent/parent/subagent/review', title: 'Review metric track diff', status: 'idle', statusLabel: 'Done', usage: { _tag: 'Known', scope: 'reported-agent', tokens: 48_200 }, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 140_000 } },
    { ...subagent, ref: 'agent/parent/subagent/checks', title: 'Run row fitting checks', status: 'working', statusLabel: 'Working', unread: 2, lastTurn: { _tag: 'Known', kind: 'activity', at: storyNow - 12_000 } },
  ],
}

interface Item extends SidebarTreeItem<Item> { readonly row: Row; readonly children: readonly Item[] }
const toItem = (row: Row, depth: number): Item => ({ id: row.ref, depth, textValue: row.title, row, children: row.children.map(next => toItem(next, depth + 1)) })
const treeItems: readonly Item[] = [toItem(parent, 0), toItem(stateRows[2]!, 0)]

function StateMatrix() {
  return <div {...stylex.props(styles.matrix)}>
    {[false, true].map(frozen => <section key={String(frozen)} data-column={frozen ? 'frozen' : 'live'} aria-label={frozen ? 'Frozen snapshot' : 'Live'} {...stylex.props(styles.column)}>
      <h3 {...stylex.props(styles.heading)}>{frozen ? 'Frozen · observed 2m before now' : 'Live'}</h3>
      {/* Frozen rows measure against the observation; their last turn (3m ago) still reads 1m, never a clamped 0s. */}
      {stateRows.map(row => <MilestoneAgentRow key={row.ref} row={row} frozen={frozen} now={frozen ? storyNow - 120_000 : storyNow} />)}
      {/* Activity newer than the observation cannot be in the snapshot: the frozen row omits it. */}
      {frozen && <MilestoneAgentRow row={{ ...stateRows[0]!, ref: 'agent/after-observation', title: 'Activity after the observation', lastTurn: { _tag: 'Known', kind: 'activity', at: storyNow - 30_000 } }} frozen now={storyNow - 120_000} />}
    </section>)}
  </div>
}

function TreeAndStandalone() {
  const [expanded, setExpanded] = React.useState<Set<Key>>(() => new Set([parent.ref]))
  return <div {...stylex.props(styles.matrix)}>
    <section aria-label="In SidebarTree" data-host="tree" {...stylex.props(styles.column, styles.tree)}>
      <SidebarTree aria-label="Agents" items={treeItems} expandedKeys={expanded} onExpandedChange={setExpanded} renderRow={item => <MilestoneAgentRow row={item.row} active={item.id === parent.ref} now={storyNow} />} />
    </section>
    <section aria-label="Standalone" data-host="standalone" {...stylex.props(styles.column)}>
      <MilestoneAgentRow row={parent} now={storyNow} chevron={null} />
      <MilestoneAgentRow row={parent.children[0]!} depth={1} now={storyNow} chevron={null} />
    </section>
  </div>
}

function TreeWithActions() {
  const [expanded, setExpanded] = React.useState<Set<Key>>(() => new Set([parent.ref]))
  const [opened, setOpened] = React.useState<string>()
  // Stable callbacks: the row is memoized.
  const onOpen = React.useCallback((row: Row) => setOpened(row.ref), [])
  const renderRow = React.useCallback((item: Item) => <MilestoneAgentRow row={item.row} now={storyNow} onOpen={onOpen} />, [onOpen])
  return <section aria-label="With actions" data-host="actions" data-opened={opened} {...stylex.props(styles.column, styles.tree)}>
    <SidebarTree aria-label="Agents with actions" items={treeItems} expandedKeys={expanded} onExpandedChange={setExpanded} renderRow={renderRow}
      onRowContextMenu={(_item, event) => { if (openAgentRowMenu(event.currentTarget)) event.preventDefault() }} />
  </section>
}

const meta = { title: 'Fractal UI/Sidebar/Milestone agent row', component: MilestoneAgentRow, parameters: { layout: 'fullscreen', docs: { description: { component: 'RM1 SR-2 row (48px, two lines) with SG-1 · G3 glyphs. Frozen rows mute every tone and draw a shape cue for every state, so working and idle stay apart; stale is never the danger red.' } } } } satisfies Meta<typeof MilestoneAgentRow>
export default meta
type Story = StoryObj<typeof meta>

const glyph = (row: Element) => row.querySelector<SVGElement>('[data-row-column="status"] svg')!
const column = (root: HTMLElement, name: 'live' | 'frozen') => [...root.querySelectorAll(`[data-column="${name}"] [data-milestone-row]`)]

export const GlyphStates: Story = {
  tags: ['critical'], name: 'G3 glyph states · live and frozen',
  args: { row: base },
  render: () => <main {...stylex.props(styles.root)}><StateMatrix /></main>,
  play: async ({ canvasElement }) => {
    const live = column(canvasElement, 'live')
    const frozen = column(canvasElement, 'frozen').filter(row => !row.textContent?.includes('after the observation'))
    await expect(live).toHaveLength(stateOrder.length)
    await expect(frozen).toHaveLength(stateOrder.length)
    // Frozen: one tone, so every canonical state needs its own shape — working and idle included.
    await expect(new Set(frozen.map(row => glyph(row).innerHTML)).size).toBe(stateOrder.length)
    await expect(new Set(frozen.map(row => getComputedStyle(glyph(row)).color)).size).toBe(1)
    const byStatus = (rows: Element[], status: AgentStatus) => rows.find(row => row.getAttribute('data-status') === status)!
    // Live keeps its accents; stale is an observation gap and shares the muted frozen tone, never danger red.
    const frozenTone = getComputedStyle(glyph(frozen[0]!)).color
    await expect(getComputedStyle(glyph(byStatus(live, 'working'))).color).not.toBe(frozenTone)
    await expect(getComputedStyle(glyph(byStatus(live, 'stale'))).color).toBe(frozenTone)
    // The snapshot is separate from the row's own status and freshness.
    const frozenWorking = byStatus(frozen, 'working')
    await expect(frozenWorking.getAttribute('data-observation-freshness')).toBe('live')
    await expect(glyph(frozenWorking)).toHaveAttribute('aria-label', expect.stringMatching(/^Working; .*live observation; last-observed snapshot$/))
    await expect(glyph(byStatus(live, 'working')).getAttribute('aria-label')).not.toMatch(/snapshot/)
    // Frozen times measure against the observation: 3m ago reads 1m; activity after the observation is omitted, not 0s.
    await expect(frozenWorking.querySelector('[data-row-field="last-turn"] time')?.textContent).toBe('1m')
    const after = within(canvasElement).getByText('Activity after the observation').closest('[data-milestone-row]')!
    await expect(after.querySelector('[data-row-field="last-turn"] time')).toBeNull()
  },
}

export const TwoLineAndNesting: Story = {
  tags: ['critical'], name: 'Two lines · nesting in SidebarTree and standalone',
  args: { row: base },
  render: () => <main {...stylex.props(styles.root)}><TreeAndStandalone /></main>,
  play: async ({ canvasElement }) => {
    const tree = canvasElement.querySelector<HTMLElement>('[data-host="tree"]')!
    const rows = await waitFor(() => { const found = [...tree.querySelectorAll<HTMLElement>('[data-milestone-row]')].filter(row => row.closest('[inert]') === null); expect(found).toHaveLength(4); return found })
    const [root, review, checks, leaf] = rows
    for (const row of rows) {
      await expect(Math.abs(row.getBoundingClientRect().height - 48)).toBeLessThanOrEqual(1)
      // Line 1 (glyph, title, time, signal) sits wholly above line 2.
      const title = row.querySelector('[data-row-column="title"]')!.getBoundingClientRect()
      const line2 = row.querySelector('[data-row-column="line2"]')!.getBoundingClientRect()
      await expect(title.bottom).toBeLessThanOrEqual(line2.top + 1)
      await expect(Math.abs(row.querySelector('[data-row-column="status"]')!.getBoundingClientRect().top - title.top)).toBeLessThanOrEqual(4)
      // SidebarTree owns the disclosure and indentation: no row-level chevron column.
      await expect(row.querySelector('[data-row-column="chevron"]')).toBeNull()
    }
    // Line 2: status word (working included) · host · spend and tokens · duration.
    const line2 = (row: Element, field: string) => row.querySelector(`[data-row-column="line2"] [data-row-field="${field}"]`)
    await expect(line2(root!, 'status-label')?.textContent).toBe('Working')
    await expect(line2(root!, 'host')?.textContent).toBe('build-2')
    await expect(line2(root!, 'spend')?.textContent).toBe('$3.4 · 1.2M')
    await expect(line2(root!, 'duration')).not.toBeNull()
    await expect(line2(review!, 'status-label')?.textContent).toBe('Done')
    // A reported-agent aggregate without a USD cost shows tokens only, never a substituted $0.
    await expect(line2(review!, 'spend')?.textContent).toBe('48K tokens')
    await expect(line2(review!, 'spend')).toHaveAttribute('aria-label', 'Reported by agent: 48200 tokens')
    // Attention and unread share the trailing signal column.
    await expect(root!.querySelector('[data-row-column="attention"]')).toHaveTextContent('!')
    await expect(checks!.querySelector('[data-row-signal="unread"]')).toHaveTextContent('2')
    // Nesting: subagents are level-2 tree rows, indented past a root-level leaf and aligned with each other.
    await expect(review!.closest('[role="row"]')).toHaveAttribute('aria-level', '2')
    await expect(root!.closest('[role="row"]')).toHaveAttribute('aria-level', '1')
    await expect(review!.getBoundingClientRect().left).toBeGreaterThan(leaf!.getBoundingClientRect().left)
    await expect(Math.abs(review!.getBoundingClientRect().left - checks!.getBoundingClientRect().left)).toBeLessThanOrEqual(1)
    // Standalone hosts pass depth and their own chevron slot; depth indents the row content.
    const standalone = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="standalone"] [data-milestone-row]')]
    await expect(standalone[0]!.querySelector('[data-row-column="chevron"]')).not.toBeNull()
    await expect(standalone[1]!.querySelector('[data-row-column="status"]')!.getBoundingClientRect().left).toBeGreaterThan(standalone[0]!.querySelector('[data-row-column="status"]')!.getBoundingClientRect().left)
    // Pointer hover shows the kit hover card for that row; leaving dismisses it.
    await userEvent.hover(root!.querySelector('[data-row-column="title"]')!)
    const card = await waitFor(() => { const element = document.querySelector('[data-testid="agent-hover-card"]'); expect(element).not.toBeNull(); return element! })
    await expect(card).toHaveTextContent(parent.title)
    await userEvent.unhover(root!.querySelector('[data-row-column="title"]')!)
    await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull())
  },
}

export const RowActions: Story = {
  tags: ['critical'], name: 'Row actions · shared menu and reported details',
  args: { row: base },
  render: () => <main {...stylex.props(styles.root)}><TreeWithActions /></main>,
  play: async ({ canvasElement }) => {
    const host = canvasElement.querySelector<HTMLElement>('[data-host="actions"]')!
    const root = await waitFor(() => { const row = [...host.querySelectorAll<HTMLElement>('[data-milestone-row]')].find(element => element.closest('[inert]') === null); expect(row).toBeDefined(); return row! })
    const gear = root.querySelector<HTMLElement>('[data-agent-menu]')!
    // The trigger overlays line 2 without a track of its own: the row stays 48px and the `!` stays visible.
    await expect(Math.abs(root.getBoundingClientRect().height - 48)).toBeLessThanOrEqual(1)
    await expect(gear.getBoundingClientRect().top).toBeGreaterThanOrEqual(root.querySelector('[data-row-column="line2"]')!.getBoundingClientRect().top - 1)
    await expect(root.querySelector('[data-row-column="attention"]')).toBeVisible()
    // A pointer context menu opens the shared menu.
    await userEvent.pointer({ keys: '[MouseRight]', target: root.querySelector('[data-row-column="title"]')! })
    const menu = await waitFor(() => { const element = document.querySelector<HTMLElement>('[role="menu"]'); expect(element).not.toBeNull(); return element! })
    await expect(menu).toHaveTextContent('Open conversation')
    await expect(menu).toHaveTextContent('Reported agent details')
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Open conversation' }))
    await waitFor(() => expect(host).toHaveAttribute('data-opened', parent.ref))
    // Reported details open as a dialog from the menu.
    await userEvent.click(gear)
    const again = await waitFor(() => { const element = document.querySelector<HTMLElement>('[role="menu"]'); expect(element).not.toBeNull(); return element! })
    await userEvent.click(within(again).getByRole('menuitem', { name: 'Reported agent details' }))
    await waitFor(() => expect(document.querySelector(`[role="dialog"][aria-label="Reported details for ${parent.title}"]`)).not.toBeNull())
    await userEvent.keyboard('{Escape}')
  },
}

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  matrix: { display: 'flex', alignItems: 'flex-start', flexWrap: 'wrap', gap: s.xl },
  column: { display: 'flex', flexDirection: 'column', gap: s.xs2, width: g.specimenSidebar, padding: s.xs, backgroundColor: surface.sidebar },
  tree: { height: '320px' },
  heading: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium },
})
