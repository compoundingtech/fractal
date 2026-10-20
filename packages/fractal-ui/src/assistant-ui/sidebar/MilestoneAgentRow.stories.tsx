/** RM1 two-line agent row: every canonical G3 state in both schemes at the live default sidebar width, G3 glyph states live and frozen, the 48px two-line anatomy, nesting inside the kit SidebarTree, and the hover card. */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { Key } from 'react-aria-components'
import { MilestoneAgentRow, openAgentRowMenu } from './MilestoneAgentRow'
import { SidebarTree, SidebarTreeDisclosure, type SidebarTreeItem, type SidebarTreeRowState } from './SidebarTree'
import { statuses, type AgentStatus, type SidebarAgentRow as Row } from './model'
import { geometryNumbers, geometryVars as g, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { lightTheme, type Scheme } from '../composition-theme'
import { ThemePortal } from '../taste/ThemePortal'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

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
    { ...subagent, ref: 'agent/parent/subagent/review', title: 'Review metric track diff', status: 'ended', statusLabel: statuses.ended.label, usage: { _tag: 'Known', scope: 'reported-agent', tokens: 48_200 }, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 140_000 } },
    { ...subagent, ref: 'agent/parent/subagent/checks', title: 'Run row fitting checks', status: 'working', statusLabel: 'Working', unread: 2, lastTurn: { _tag: 'Known', kind: 'activity', at: storyNow - 12_000 } },
  ],
}

/** Agents put a touch disclosure in their trailing column (their own press opens the conversation); folders toggle as a whole row. */
interface AgentItem extends SidebarTreeItem<AgentItem> { readonly kind: 'agent'; readonly row: Row; readonly children: readonly AgentItem[] }
interface FolderItem extends SidebarTreeItem<Item> { readonly kind: 'folder'; readonly children: readonly AgentItem[] }
type Item = AgentItem | FolderItem
const toItem = (row: Row, depth: number): AgentItem => ({ kind: 'agent', id: row.ref, depth, textValue: row.title, row, disclosurePlacement: 'trailing', children: row.children.map(next => toItem(next, depth + 1)) })
const treeItems: readonly AgentItem[] = [toItem(parent, 0), toItem(stateRows[2]!, 0)]
const touchDisclosure = (state: SidebarTreeRowState) => state.trailingDisclosure ? <SidebarTreeDisclosure expanded={state.expanded} /> : undefined

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
      <SidebarTree aria-label="Agents" items={treeItems} expandedKeys={expanded} onExpandedChange={setExpanded} renderRow={(item, state) => <MilestoneAgentRow row={item.row} active={item.id === parent.ref} now={storyNow} touchDisclosure={touchDisclosure(state)} />} />
    </section>
    <section aria-label="Standalone" data-host="standalone" {...stylex.props(styles.column)}>
      <MilestoneAgentRow row={parent} now={storyNow} chevron={null} />
      <MilestoneAgentRow row={parent.children[0]!} depth={1} now={storyNow} chevron={null} />
    </section>
  </div>
}

const noSessionChild: Row = { ...subagent, ref: 'agent/parent/subagent/no-session', title: 'Child without session', status: 'unobserved', statusLabel: 'Unobserved', subagent: { id: 'no-session', lease_expires_at: '2026-10-17T10:00:00Z' } }
/** A display-only subagent: the tree row is disabled and the row drops its menu, hover and active presentation. */
const displayOnlyChild = parent.children[0]!
const actionAgent = (depth: number): AgentItem => {
  const root = toItem({ ...parent, children: [...parent.children, noSessionChild] }, depth)
  return { ...root, children: root.children.map(child => child.id === displayOnlyChild.ref ? { ...child, isDisabled: true } : child) }
}
const actionAgents: readonly AgentItem[] = [actionAgent(0)]
/** Phone: the same agents inside a host folder, whose header row is a leading, whole-row disclosure. */
const phoneFolder: FolderItem = { kind: 'folder', id: 'folder/build-2', depth: 0, textValue: 'build-2', children: [actionAgent(1)] }

function TreeWithActions({ phone = false }: { readonly phone?: boolean }) {
  const [expanded, setExpanded] = React.useState<Set<Key>>(() => new Set([phoneFolder.id, parent.ref]))
  const [opened, setOpened] = React.useState<string>()
  const [selected, setSelected] = React.useState<string>()
  // Stable callbacks: the row is memoized.
  const onOpen = React.useCallback((row: Row) => setOpened(row.ref), [])
  const renderRow = React.useCallback((item: Item, state: SidebarTreeRowState) => item.kind === 'folder'
    ? <span data-folder-label {...stylex.props(styles.folderLabel)}>{item.textValue}</span>
    : <MilestoneAgentRow row={item.row} now={storyNow} displayOnly={item.isDisabled} touchDisclosure={touchDisclosure(state)} onOpen={item.id === noSessionChild.ref ? undefined : onOpen} />, [onOpen])
  return <section aria-label="With actions" data-host="actions" data-opened={opened} data-selected={selected} {...stylex.props(styles.column, styles.tree, phone && styles.phoneColumn)}>
    <SidebarTree<Item> aria-label="Agents with actions" items={phone ? [phoneFolder] : actionAgents} expandedKeys={expanded} onExpandedChange={setExpanded} renderRow={renderRow} onAction={key => setSelected(String(key))}
      onRowContextMenu={(_item, event) => { if (openAgentRowMenu(event.currentTarget)) event.preventDefault() }} />
  </section>
}

/** One live row per canonical state in the live default sidebar; the scheme themes the story root, as the app themes its shell. */
function AllStatesSidebar({ scheme }: { readonly scheme: Scheme }) {
  return <main data-testid="all-states" data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}><ThemePortal>
    <span aria-hidden="true" data-tone-probe="" {...stylex.props(styles.toneProbe)} />
    <section aria-label="All states" data-host="all-states" {...stylex.props(styles.column)}>
      {stateRows.map(row => <MilestoneAgentRow key={row.ref} row={row} now={storyNow} />)}
    </section>
  </ThemePortal></main>
}

const meta = { title: 'Fractal UI/Sidebar/Milestone agent row', component: MilestoneAgentRow, parameters: { layout: 'fullscreen', docs: { description: { component: 'RM1 SR-2 row (48px, two lines) with SG-1 · G3 glyphs. Frozen rows mute every tone and draw a shape cue for every state, so working and idle stay apart; stale is never the danger red.' } } } } satisfies Meta<typeof MilestoneAgentRow>
export default meta
type Story = StoryObj<typeof meta>

const glyph = (row: Element) => row.querySelector<SVGElement>('[data-row-column="status"] svg')!
const column = (root: HTMLElement, name: 'live' | 'frozen') => [...root.querySelectorAll(`[data-column="${name}"] [data-milestone-row]`)]

export const GlyphStates: Story = {
  tags: ['critical'], name: 'G3 glyph states · live and frozen',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'The exhaustive live and frozen status matrix fixes timestamps to verify distinct glyph shapes, tones and observation-relative times.' } },
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
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed parent, child and standalone rows verify 48px two-line geometry, nesting indentation and metric alignment.' } },
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
    await expect(line2(review!, 'status-label')?.textContent).toBe('Ended')
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
    // Disclosure gutter: the root parent's glyph and title share the same-depth leaf's edges and title width; the chevron
    // costs no title room, sits wholly in the inline-start gutter before the glyph, and centres on line one.
    const box = (row: Element, column: string) => row.querySelector(`[data-row-column="${column}"]`)!.getBoundingClientRect()
    const disclosure = root!.closest('[role="row"]')!.querySelector('[data-sidebar-disclosure]')!.getBoundingClientRect()
    await expect(leaf!.closest('[role="row"]')!.querySelector('[data-sidebar-disclosure]')).toBeNull()
    await expect(Math.abs(box(root!, 'status').left - box(leaf!, 'status').left)).toBeLessThanOrEqual(0.5)
    await expect(Math.abs(box(root!, 'title').left - box(leaf!, 'title').left)).toBeLessThanOrEqual(0.5)
    await expect(Math.abs(box(root!, 'title').width - box(leaf!, 'title').width)).toBeLessThanOrEqual(0.5)
    await expect(disclosure.width).toBeGreaterThan(0)
    await expect(disclosure.left).toBeGreaterThanOrEqual(root!.closest('[role="row"]')!.getBoundingClientRect().left - 0.5)
    await expect(disclosure.right).toBeLessThanOrEqual(root!.getBoundingClientRect().left + 0.5)
    await expect(Math.abs((disclosure.top + disclosure.bottom) / 2 - (box(root!, 'status').top + box(root!, 'status').bottom) / 2)).toBeLessThanOrEqual(1)
    // Children nest one step past their parent: same-depth subagents share glyph and title edges, and the child guide
    // runs on the parent chevron's centre, clear of the child's own gutter content.
    await expect(box(review!, 'status').left).toBeGreaterThan(box(root!, 'status').left)
    await expect(Math.abs(box(review!, 'status').left - box(checks!, 'status').left)).toBeLessThanOrEqual(0.5)
    await expect(Math.abs(box(review!, 'title').left - box(checks!, 'title').left)).toBeLessThanOrEqual(0.5)
    await expect(Math.abs((box(review!, 'status').left - box(root!, 'status').left) - (box(review!, 'title').left - box(root!, 'title').left))).toBeLessThanOrEqual(0.5)
    const guide = review!.closest('[role="row"]')!.querySelector('[data-sidebar-guide]')!.getBoundingClientRect()
    await expect(Math.abs(guide.left + guide.width / 2 - (disclosure.left + disclosure.right) / 2)).toBeLessThanOrEqual(1)
    await expect(guide.right).toBeLessThanOrEqual(review!.getBoundingClientRect().left)
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
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed rows with and without conversations exercise pointer and keyboard menus, details dialogs and clipping boundaries.' } },
  args: { row: base },
  render: () => <main {...stylex.props(styles.root)}><ThemePortal><div {...stylex.props(styles.clipped)}><TreeWithActions /></div></ThemePortal></main>,
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
    await waitFor(() => expect(document.querySelector('[role="menu"]')).toBeNull())
    // Reported details open as a dialog from the menu.
    await userEvent.click(gear)
    const again = await waitFor(() => { const element = document.querySelector<HTMLElement>('[role="menu"]'); expect(element).not.toBeNull(); return element! })
    await userEvent.click(within(again).getByRole('menuitem', { name: 'Reported agent details' }))
    await waitFor(() => expect(document.querySelector(`[role="dialog"][aria-label="Reported details for ${parent.title}"]`)).not.toBeNull())
    await waitFor(() => expect(document.querySelector('[role="menu"]')).toBeNull())
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull())
    const child = [...host.querySelectorAll<HTMLElement>('[data-milestone-row]')].find(row => row.textContent?.includes(noSessionChild.title) && row.closest('[inert]') === null)!
    for (const keyboard of [false, true]) {
      if (keyboard) {
        child.closest<HTMLElement>('[role="row"]')!.focus()
        // A keyboard context-menu event targets the focused tree row, not its inner painted frame.
        child.closest<HTMLElement>('[role="row"]')!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 0 }))
      } else await userEvent.click(child.querySelector<HTMLElement>('[data-agent-menu]')!)
      const childMenu = await waitFor(() => within(document.body).getByRole('menu', { name: `Actions for ${noSessionChild.title}` }))
      await expect(within(childMenu).getByRole('menuitem', { name: /Open conversation/ })).toHaveAttribute('aria-disabled', 'true')
      await expect(childMenu).toHaveTextContent('No session reported for this subagent')
      const reason = within(childMenu).getByText('No session reported for this subagent')
      await waitFor(() => expect(reason).toBeVisible())
      await expect(reason.scrollWidth).toBeLessThanOrEqual(reason.clientWidth + 1)
      // The overlay must escape sidebar clipping and remain fully visible in the viewport.
      await waitFor(() => {
        const box = childMenu.getBoundingClientRect()
        expect(box.left).toBeGreaterThanOrEqual(0)
        expect(box.top).toBeGreaterThanOrEqual(0)
        expect(box.right).toBeLessThanOrEqual(window.innerWidth)
        expect(box.bottom).toBeLessThanOrEqual(window.innerHeight)
        expect(childMenu.contains(document.elementFromPoint(box.right - 2, box.top + box.height / 2))).toBe(true)
      })
      await expect(within(childMenu).getByRole('menuitem', { name: /Open conversation/ })).toHaveAccessibleDescription('No session reported for this subagent')
      await userEvent.click(within(childMenu).getByRole('menuitem', { name: 'Reported agent details' }))
      await waitFor(() => expect(document.querySelector(`[role="dialog"][aria-label="Reported details for ${noSessionChild.title}"]`)).not.toBeNull())
      await waitFor(() => expect(document.querySelector('[role="menu"]')).toBeNull())
      await userEvent.keyboard('{Escape}')
      await waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull())
    }
  },
}

const neutralStates = ['idle', 'pending', 'offline', 'ended', 'suspended', 'retired', 'unobserved'] as const satisfies readonly AgentStatus[]
const allStatesPlay: NonNullable<StoryObj<{ scheme: Scheme }>['play']> = async ({ canvasElement }) => {
  const surfaceRoot = within(canvasElement).getByTestId('all-states')
  const host = surfaceRoot.querySelector<HTMLElement>('[data-host="all-states"]')!
  // The fixture is the live default sidebar aside (`sidebarDefault`, roster inset), not a specimen width.
  await expect(Math.abs(host.getBoundingClientRect().width - geometryNumbers.sidebarDefault)).toBeLessThanOrEqual(1)
  const rows = [...host.querySelectorAll<HTMLElement>('[data-milestone-row]')]
  await expect(rows.map(row => row.getAttribute('data-status'))).toEqual(stateOrder)
  const at = (status: AgentStatus) => glyph(rows.find(row => row.getAttribute('data-status') === status)!)
  const color = (status: AgentStatus) => getComputedStyle(at(status)).color
  // G3: working and idle are bare discs told apart by tone; every other state draws its canonical shape over a faded disc.
  for (const status of stateOrder) {
    await expect(at(status)).toHaveAttribute('data-disc-refinement', 'G3')
    await expect(at(status).children.length === 1).toBe(status === 'working' || status === 'idle')
  }
  await expect(new Set(stateOrder.map(status => `${at(status).innerHTML}|${color(status)}`)).size).toBe(stateOrder.length)
  // Ended is not idle: a finished agent draws the stop shape, never the bare idle disc.
  await expect(at('ended').innerHTML).not.toBe(at('idle').innerHTML)
  const probe = getComputedStyle(surfaceRoot.querySelector('[data-tone-probe]')!)
  const soft = probe.color, muted = probe.backgroundColor, danger = probe.borderTopColor
  await expect(color('working')).not.toBe(soft)
  await expect(color('waiting')).not.toBe(soft)
  await expect(color('waiting')).not.toBe(color('working'))
  for (const status of neutralStates) await expect(color(status)).toBe(soft)
  // Stale is an observation gap: muted, never the danger red.
  await expect(color('stale')).toBe(muted)
  await expect(color('stale')).not.toBe(danger)
  // The scheme reaches the surface: light renders on a light canvas, dark on a dark one (relative luminance of the canvas).
  const [red, green, blue] = getComputedStyle(surfaceRoot).backgroundColor.match(/\d+(\.\d+)?/g)!.map(Number)
  await expect((0.2126 * red! + 0.7152 * green! + 0.0722 * blue!) / 255 > 0.5).toBe(surfaceRoot.getAttribute('data-scheme') === 'light')
}

export const AllStates: StoryObj<{ scheme: Scheme }> = {
  tags: ['critical'], name: 'All G3 states · live default width',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'An exhaustive status matrix at the default sidebar width verifies glyph uniqueness and theme tones, including states absent from a single scenario roster.' } },
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <AllStatesSidebar scheme={scheme} />,
  play: allStatesPlay,
}

export const AllStatesLight: StoryObj<{ scheme: Scheme }> = { ...AllStates, name: 'All G3 states · live default width · light', args: { scheme: 'light' } }

const tap = (target: Element) => userEvent.pointer([{ keys: '[TouchA>]', target }, { keys: '[/TouchA]', target }])
const phoneTouchPlay: NonNullable<StoryObj<{ scheme: Scheme }>['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const host = canvasElement.querySelector<HTMLElement>('[data-host="actions"]')!
  const childTitles = [...parent.children, noSessionChild].map(child => child.title)
  const agentTitles = [parent.title, ...childTitles]
  const corners = (box: DOMRect) => [[box.left + 1, box.top + 1], [box.right - 1, box.top + 1], [box.left + 1, box.bottom - 1], [box.right - 1, box.bottom - 1]] as const
  const offset = (title: string) => canvas.getByRole('row', { name: title }).querySelector('[data-milestone-row]')!.getBoundingClientRect().left - canvas.getByRole('row', { name: title }).getBoundingClientRect().left
  // Labels keep the fine-pointer x: a 16px gutter at every depth, and one depth step between parent and child.
  const folderRow = await waitFor(() => canvas.getByRole('row', { name: phoneFolder.textValue }))
  await waitFor(() => expect(canvas.getByRole('row', { name: parent.title })).toBeVisible())
  const step = offset(childTitles[1]!) - offset(parent.title)
  await expect(Math.abs(offset(parent.title) - step - 16)).toBeLessThanOrEqual(0.5)
  await expect(Math.abs(folderRow.querySelector('[data-folder-label]')!.getBoundingClientRect().left - folderRow.getBoundingClientRect().left - 16)).toBeLessThanOrEqual(0.5)
  // Folder header: the whole row is the disclosure, at least 44px tall, its glyph centred in the 16px gutter.
  const header = folderRow.querySelector<HTMLElement>('[data-sidebar-header-toggle]')!
  const headerBox = header.getBoundingClientRect()
  const folderBox = folderRow.getBoundingClientRect()
  await expect(folderRow).toHaveAttribute('aria-expanded', 'true')
  await expect(headerBox.height).toBeGreaterThanOrEqual(44)
  await expect(Math.abs(headerBox.width - folderBox.width)).toBeLessThanOrEqual(0.5)
  await expect(headerBox.bottom).toBeLessThanOrEqual(canvas.getByRole('row', { name: parent.title }).getBoundingClientRect().top + 0.5)
  const headerGlyph = header.querySelector('svg')!.getBoundingClientRect()
  await expect(Math.abs((headerGlyph.left + headerGlyph.right) / 2 - folderBox.left - 8)).toBeLessThanOrEqual(0.5)
  for (const [x, y] of corners(headerBox)) await expect(header.contains(document.elementFromPoint(x, y))).toBe(true)
  const label = folderRow.querySelector('[data-folder-label]')!.getBoundingClientRect()
  await expect(header.contains(document.elementFromPoint((label.left + label.right) / 2, (label.top + label.bottom) / 2))).toBe(true)
  // Tapping the header collapses the folder; tapping it again brings every agent back.
  await tap(header)
  await waitFor(() => {
    expect(canvas.getByRole('row', { name: phoneFolder.textValue })).toHaveAttribute('aria-expanded', 'false')
    for (const title of agentTitles) expect(canvas.queryByRole('row', { name: title })).toBeNull()
  })
  await tap(canvas.getByRole('row', { name: phoneFolder.textValue }).querySelector('[data-sidebar-header-toggle]')!)
  await waitFor(() => {
    expect(canvas.getByRole('row', { name: phoneFolder.textValue })).toHaveAttribute('aria-expanded', 'true')
    for (const title of agentTitles) expect(canvas.getByRole('row', { name: title })).toBeVisible()
  })
  // Agent parent: no gutter chevron; a 44px trailing disclosure inside the row, after the title and just before the
  // actions button, with every corner hit-testing to it.
  const parentRow = canvas.getByRole('row', { name: parent.title })
  await expect(parentRow).toHaveAttribute('aria-expanded', 'true')
  await expect(parentRow.querySelector('[data-sidebar-disclosure="leading"]')).toBeNull()
  const disclosure = within(parentRow).getByRole('button', { name: 'Hide subagents' })
  await expect(disclosure).toHaveAttribute('data-sidebar-disclosure', 'trailing')
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true')
  const hit = disclosure.getBoundingClientRect()
  const row = parentRow.getBoundingClientRect()
  const title = parentRow.querySelector('[data-row-column="title"]')!.getBoundingClientRect()
  const line2 = parentRow.querySelector('[data-row-column="line2"]')!.getBoundingClientRect()
  const parentActions = within(parentRow).getByRole('button', { name: `Actions for ${parent.title}` }).getBoundingClientRect()
  await expect(hit.width).toBeGreaterThanOrEqual(44)
  await expect(hit.height).toBeGreaterThanOrEqual(44)
  await expect(hit.top).toBeGreaterThanOrEqual(row.top - 0.5)
  await expect(hit.bottom).toBeLessThanOrEqual(row.bottom + 0.5)
  await expect(hit.left).toBeGreaterThanOrEqual(title.right - 0.5)
  await expect(hit.left).toBeGreaterThanOrEqual(line2.right - 0.5)
  await expect(hit.right).toBeLessThanOrEqual(parentActions.left + 0.5)
  await expect(parentActions.left - hit.right).toBeLessThanOrEqual(8)
  for (const [x, y] of corners(hit)) await expect(disclosure.contains(document.elementFromPoint(x, y))).toBe(true)
  // Tap collapses: every child row leaves the tree. Tap again expands: each child is back and visible. Neither tap
  // selects or opens the parent.
  for (const title of childTitles) await expect(canvas.getByRole('row', { name: title })).toBeVisible()
  await tap(disclosure)
  await waitFor(() => {
    expect(canvas.getByRole('row', { name: parent.title })).toHaveAttribute('aria-expanded', 'false')
    for (const title of childTitles) expect(canvas.queryByRole('row', { name: title })).toBeNull()
  })
  const collapsed = within(canvas.getByRole('row', { name: parent.title })).getByRole('button', { name: 'Show subagents' })
  await expect(collapsed).toHaveAttribute('aria-expanded', 'false')
  await tap(collapsed)
  await waitFor(() => {
    expect(canvas.getByRole('row', { name: parent.title })).toHaveAttribute('aria-expanded', 'true')
    for (const title of childTitles) expect(canvas.getByRole('row', { name: title })).toBeVisible()
  })
  await expect(host).not.toHaveAttribute('data-selected')
  await expect(host).not.toHaveAttribute('data-opened')
  // Leaf agents are unchanged: no disclosure of either kind.
  await expect(canvas.getByRole('row', { name: noSessionChild.title }).querySelector('[data-sidebar-disclosure]')).toBeNull()
  // A display-only child is no press target: no button, link or menu, no tab or arrow stop, and click, double-click and
  // Enter keep both the selection (a sibling, so a press reaching the parent would also show) and the open conversation.
  const sibling = parent.children[1]!
  await tap(canvas.getByRole('row', { name: sibling.title }).querySelector('[data-row-column="title"]')!)
  await waitFor(() => expect(host).toHaveAttribute('data-selected', sibling.ref))
  await expect(host).not.toHaveAttribute('data-opened')
  const displayOnly = canvas.getByRole('row', { name: displayOnlyChild.title })
  await expect(displayOnly).toHaveAttribute('aria-disabled', 'true')
  await expect(displayOnly).not.toHaveAttribute('tabindex', '0')
  for (const role of ['button', 'link', 'menu'] as const) await expect(within(displayOnly).queryAllByRole(role)).toHaveLength(0)
  await expect(displayOnly.querySelector('[data-agent-menu], [data-row-action]')).toBeNull()
  // Display-only is not disabled-looking: full opacity and the same title ink as an enabled sibling.
  const titleInk = (title: string) => getComputedStyle(canvas.getByRole('row', { name: title }).querySelector('[data-row-column="title"]')!).color
  await expect(getComputedStyle(displayOnly).opacity).toBe('1')
  await expect(titleInk(displayOnlyChild.title)).toBe(titleInk(sibling.title))
  await userEvent.click(displayOnly)
  await userEvent.dblClick(displayOnly)
  displayOnly.focus()
  await expect(displayOnly).not.toHaveFocus()
  await userEvent.keyboard('{Enter}')
  await expect(host).toHaveAttribute('data-selected', sibling.ref)
  await expect(host).not.toHaveAttribute('data-opened')
  // Arrow navigation from the parent steps over the display-only child to the next enabled sibling.
  canvas.getByRole('row', { name: parent.title }).focus()
  await userEvent.keyboard('{ArrowDown}')
  await waitFor(() => expect(canvas.getByRole('row', { name: sibling.title })).toHaveFocus())
  const trigger = await waitFor(() => canvas.getByRole('button', { name: `Actions for ${noSessionChild.title}` }))
  // No mouse hover/focus setup: the narrow action must already be painted and touch-sized.
  await expect(trigger).toBeVisible()
  await expect(getComputedStyle(trigger).opacity).toBe('1')
  await expect(trigger.getBoundingClientRect().width).toBeGreaterThanOrEqual(44)
  await expect(trigger.getBoundingClientRect().height).toBeGreaterThanOrEqual(44)
  await tap(trigger)
  const menu = await waitFor(() => within(document.body).getByRole('menu', { name: `Actions for ${noSessionChild.title}` }))
  await expect(within(menu).getByRole('menuitem', { name: /Open conversation/ })).toHaveAttribute('aria-disabled', 'true')
  await waitFor(() => {
    const box = menu.getBoundingClientRect()
    expect(box.left).toBeGreaterThanOrEqual(0)
    expect(box.right).toBeLessThanOrEqual(Math.min(window.innerWidth, 390))
    expect(box.top).toBeGreaterThanOrEqual(0)
    expect(box.bottom).toBeLessThanOrEqual(window.innerHeight)
    expect(menu.contains(document.elementFromPoint(box.right - 2, box.top + box.height / 2))).toBe(true)
    for (const item of within(menu).getAllByRole('menuitem')) expect(item.getBoundingClientRect().height).toBeGreaterThanOrEqual(44)
  })
  const details = within(menu).getByRole('menuitem', { name: 'Reported agent details' })
  await tap(details)
  await waitFor(() => expect(within(document.body).getByRole('dialog', { name: `Reported details for ${noSessionChild.title}` })).toBeVisible())
  await waitFor(() => expect(document.querySelector('[role="menu"]')).toBeNull())
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull())
}

export const PhoneTouchTargets: StoryObj<{ scheme: Scheme }> = {
  tags: ['critical'], name: 'Phone · native RM1 actions without hover',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'A fixed 390px frame with a host folder, a parent with a display-only child and a child without a conversation verify the whole-row folder toggle and the trailing 44px subagent disclosure collapsing and expanding by tap at the fine-pointer label x, display-only rows ignoring every activation, always-visible 44px touch targets, menu bounds and details access without hover.' } },
  args: { scheme: 'dark' },
  render: ({ scheme }) => <main {...stylex.props(styles.root, styles.phoneFrame, scheme === 'light' && lightTheme)}><NarrowLayoutContext.Provider value={true}><ThemePortal><TreeWithActions phone /></ThemePortal></NarrowLayoutContext.Provider></main>,
  play: phoneTouchPlay,
}
export const PhoneTouchTargetsLight: StoryObj<{ scheme: Scheme }> = { ...PhoneTouchTargets, name: 'Phone · native RM1 actions without hover · light', args: { scheme: 'light' } }

export const PhoneFocusNoHover: StoryObj<{ scheme: Scheme }> = {
  ...PhoneTouchTargets, name: 'Phone · native RM1 focus never opens hover UI',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'A fixed narrow tree row receives hover and focus beyond the opening delay to verify that phone interaction never opens hover UI.' } },
  play: async ({ canvasElement }) => {
    const row = await waitFor(() => within(canvasElement).getByRole('row', { name: parent.title }))
    await userEvent.hover(row)
    row.focus()
    // Observe beyond the shared hook's 150ms opening delay, not just its initial closed render.
    const started = performance.now()
    do {
      await expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull()
      await expect(document.querySelector('[role="tooltip"]')).toBeNull()
      await expect(row.querySelector('[data-row-action="open"]')).toBeNull()
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
    } while (performance.now() - started < 200)
    await expect(row).toHaveFocus()
    await userEvent.unhover(row)
    row.blur()
  },
}
export const PhoneFocusNoHoverLight: StoryObj<{ scheme: Scheme }> = { ...PhoneFocusNoHover, name: 'Phone · native RM1 focus never opens hover UI · light', args: { scheme: 'light' } }

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  matrix: { display: 'flex', alignItems: 'flex-start', flexWrap: 'wrap', gap: s.xl },
  // The live default sidebar: the app's aside opens at `sidebarDefault` and its roster insets rows by `s.md` with 2px gaps.
  column: { display: 'flex', flexDirection: 'column', gap: s.xs2, width: g.sidebarDefault, boxSizing: 'border-box', padding: s.md, backgroundColor: surface.sidebar },
  tree: { height: '320px' },
  phoneFrame: { width: '390px', maxWidth: '100%', padding: 0 },
  phoneColumn: { width: '100%' },
  // Starts at the tree's content edge, so the label's x is the 16px disclosure gutter.
  folderLabel: { display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium },
  clipped: { width: g.sidebarDefault, overflow: 'hidden', transform: 'translateX(0)' },
  heading: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium },
  // Resolves the surface's own tokens for the tone assertions: soft (neutral states), muted (stale), danger (never stale).
  toneProbe: { position: 'absolute', width: s.zero, height: s.zero, overflow: 'hidden', color: ink.fgSoft, backgroundColor: ink.fgMuted, borderTopColor: tone.dangerFg },
})
