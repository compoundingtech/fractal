/** RM1 two-line agent row: every canonical G3 state in both schemes at the live default sidebar width, G3 glyph states live and frozen, the 48px two-line anatomy, nesting inside the kit SidebarTree, the hover card, and the subagent count after the title. */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { Key } from 'react-aria-components'
import { MilestoneAgentRow, openAgentRowMenu } from './MilestoneAgentRow'
import { SidebarTree, SidebarTreeDisclosure, SidebarTreeRowAction, type SidebarTreeItem, type SidebarTreeRowState } from './SidebarTree'
import { statuses, type AgentStatus, type SidebarAgentRow as Row } from './model'
import { geometryNumbers, geometryVars as g, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { lightTheme, type Scheme } from '../composition-theme'
import { ThemePortal } from '../taste/ThemePortal'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

const storyNow = Date.UTC(2026, 9, 17, 9, 30, 0)
const base: Row = {
  ref: 'agent/example-base', id: 'base', title: 'Base', status: 'idle', statusLabel: 'Idle', host: 'build-1', freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [],
}
const stateOrder = Object.keys(statuses) as AgentStatus[]
/** One row per canonical status; every row carries a last turn 3 minutes before the story clock. */
const stateRows: readonly Row[] = stateOrder.map(status => ({ ...base, ref: `agent/example-${status}`, id: status, title: `${statuses[status].label} agent`, status, statusLabel: statuses[status].label, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 180_000 } }))

const subagent: Row = { ...base, id: 'parent', parentRef: 'agent/example-parent', host: 'build-2' }
const parent: Row = {
  ...base, ref: 'agent/example-parent', id: 'parent', title: 'Align the metric track across every row', status: 'working', statusLabel: 'Working', needsMe: true, host: 'build-2',
  usage: { _tag: 'Known', scope: 'lifetime-incl-subagents', usd: 3.42, tokens: 1_180_000 }, duration: { _tag: 'Known', scope: 'lifetime', ms: 2_460_000 },
  lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 95_000 },
  children: [
    { ...subagent, ref: 'agent/example-review', title: 'Review metric track diff', status: 'ended', statusLabel: statuses.ended.label, usage: { _tag: 'Known', scope: 'reported-agent', tokens: 48_200 }, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 140_000 } },
    { ...subagent, ref: 'agent/example-checks', title: 'Run row fitting checks', status: 'working', statusLabel: 'Working', unread: 2, lastTurn: { _tag: 'Known', kind: 'activity', at: storyNow - 12_000 } },
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
      {frozen && <MilestoneAgentRow row={{ ...stateRows[0]!, ref: 'agent/example-after-observation', title: 'Activity after the observation', lastTurn: { _tag: 'Known', kind: 'activity', at: storyNow - 30_000 } }} frozen now={storyNow - 120_000} />}
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

const noSessionChild: Row = { ...subagent, ref: 'agent/example-no-session', title: 'Child without session', status: 'unobserved', statusLabel: 'Unobserved', subagent: { id: 'no-session', lease_expires_at: '2026-10-17T10:00:00Z' } }
/** A display-only subagent: the tree row is disabled and the row drops its menu, hover and active presentation. */
const displayOnlyChild = parent.children[0]!
const actionAgent = (depth: number): AgentItem => {
  const root = toItem({ ...parent, children: [...parent.children, noSessionChild] }, depth)
  return { ...root, children: root.children.map(child => child.id === displayOnlyChild.ref ? { ...child, isDisabled: true } : child) }
}
/** A parent whose only subagent is display-only: arrow keys expand it but never move focus into the child. */
const displayOnlyParentRow: Row = { ...parent, ref: 'agent/example-display-parent', title: 'Summarise reported subagents', needsMe: false,
  children: [{ ...displayOnlyChild, ref: 'agent/example-display-review', parentRef: 'agent/example-display-parent', title: 'Reported review summary' }] }
const displayOnlyParent = (depth: number): AgentItem => {
  const root = toItem(displayOnlyParentRow, depth)
  return { ...root, children: root.children.map(child => ({ ...child, isDisabled: true })) }
}
/** Desktop: a refused folder carries an inline retry, and nests a parent agent inside RAC's nested Collection. */
const retryFolder: FolderItem = { kind: 'folder', id: 'folder/example-retry', depth: 0, textValue: 'Reviews', children: [displayOnlyParent(1)] }
const actionItems: readonly Item[] = [actionAgent(0), retryFolder]
/** Phone: the same agents inside a host folder, whose header row is a leading, whole-row disclosure. */
const phoneFolder: FolderItem = { kind: 'folder', id: 'folder/example-host', depth: 0, textValue: 'build-2', children: [actionAgent(1)] }

function TreeWithActions({ phone = false }: { readonly phone?: boolean }) {
  const [expanded, setExpanded] = React.useState<Set<Key>>(() => new Set([phoneFolder.id, retryFolder.id, parent.ref]))
  const [opened, setOpened] = React.useState<string>()
  const [selected, setSelected] = React.useState<string>()
  const [retries, setRetries] = React.useState(0)
  // Stable callbacks: the row is memoized.
  const onOpen = React.useCallback((row: Row) => setOpened(row.ref), [])
  const onRetry = React.useCallback(() => setRetries(count => count + 1), [])
  const renderRow = React.useCallback((item: Item, state: SidebarTreeRowState) => {
    if (item.kind === 'agent') return <MilestoneAgentRow row={item.row} now={storyNow} displayOnly={item.isDisabled} touchDisclosure={touchDisclosure(state)} onOpen={item.id === noSessionChild.ref ? undefined : onOpen} />
    const label = <span data-folder-label {...stylex.props(styles.folderLabel)}>{item.textValue}</span>
    return item.id !== retryFolder.id ? label
      : <div {...stylex.props(styles.folderRow)}>{label}<span role="status" {...stylex.props(styles.folderStatus)}>Layout refused <SidebarTreeRowAction onPress={onRetry}>Try again</SidebarTreeRowAction></span></div>
  }, [onOpen, onRetry])
  return <section aria-label="With actions" data-host="actions" data-opened={opened} data-selected={selected} data-retries={retries} {...stylex.props(styles.column, styles.tree, phone && styles.phoneColumn)}>
    <SidebarTree<Item> aria-label="Agents with actions" items={phone ? [phoneFolder] : actionItems} expandedKeys={expanded} onExpandedChange={setExpanded} renderRow={renderRow} onAction={key => setSelected(String(key))}
      onRowContextMenu={(_item, event) => { if (openAgentRowMenu(event.currentTarget)) event.preventDefault() }} />
  </section>
}

/**
 * The kit's coarse-pointer query, served by one controllable list so a story drives fine → coarse → fine through the
 * same `change` subscription a device's pointer change would. Every other query reaches the browser.
 */
const touchPointerQuery = '(pointer: coarse) and (not (any-pointer: fine))'
class PointerMedia extends EventTarget {
  readonly media = touchPointerQuery
  onchange: MediaQueryList['onchange'] = null
  matches = false
  addListener(listener: Parameters<MediaQueryList['addListener']>[0]) { if (listener !== null) this.addEventListener('change', listener as EventListener) }
  removeListener(listener: Parameters<MediaQueryList['removeListener']>[0]) { if (listener !== null) this.removeEventListener('change', listener as EventListener) }
  emulate(coarse: boolean) { this.matches = coarse; this.dispatchEvent(new MediaQueryListEvent('change', { matches: coarse, media: this.media })) }
}
const pointerMedia = new PointerMedia()
const servePointerMedia = () => {
  const native = window.matchMedia
  pointerMedia.matches = false
  window.matchMedia = (query: string): MediaQueryList => query === touchPointerQuery ? pointerMedia : native.call(window, query)
  return () => { window.matchMedia = native }
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

const meta = { excludeStories: ['scenarioExemptions'], title: 'Fractal UI/Sidebar/Milestone agent row', component: MilestoneAgentRow, parameters: { layout: 'fullscreen', docs: { description: { component: 'RM1 SR-2 row (48px, two lines) with SG-1 · G3 glyphs. Frozen rows mute every tone and draw a shape cue for every state, so working and idle stay apart; stale is never the danger red.' } } } } satisfies Meta<typeof MilestoneAgentRow>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'AllStatesLight:contract-fixture',
  'GlyphStates:contract-fixture',
  'NativeOverlaysThemed:contract-fixture',
  'NativeOverlaysThemedLight:contract-fixture',
  'PhoneFocusNoHover:contract-fixture',
  'PhoneFocusNoHoverLight:contract-fixture',
  'PhoneTouchTargets:contract-fixture',
  'PhoneTouchTargetsLight:contract-fixture',
  'PointerSizing:contract-fixture',
  'PointerSizingLight:contract-fixture',
  'ReportedDetails:contract-fixture',
  'RowActions:contract-fixture',
  'SubagentCount:contract-fixture',
  'SubagentCountLight:contract-fixture',
  'SubagentCountPress:contract-fixture',
  'SubagentCountPressLight:contract-fixture',
  'TitleReach:contract-fixture',
  'TitleReachLight:contract-fixture',
  'TwoLineAndNesting:contract-fixture',
]
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
    // Without a mission, line 2 is exactly status · host; metrics remain in reported details.
    const line2 = (row: Element) => row.querySelector('[data-row-column="line2"]')!
    await expect(line2(root!).textContent).toBe('Working·build-2')
    await expect(line2(review!).textContent).toBe('Ended·build-2')
    for (const row of rows) {
      await expect(line2(row).querySelector('[data-box="pill"], [data-row-field="spend"], [data-row-field="duration"]')).toBeNull()
    }
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
    // The trailing slot sizes to its own content (the parent's "!" is wider than nothing), so name columns (title plus
    // subagent count) are compared by reach: on both rows the name runs right up to its trailing slot, one column gap before it.
    for (const row of [root!, leaf!]) await expect(Math.abs(box(row, 'trailing').left - box(row, 'name').right - Number.parseFloat(getComputedStyle(row).columnGap))).toBeLessThanOrEqual(0.5)
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
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed rows with and without conversations, a parent of only display-only subagents and a refused folder exercise pointer and keyboard menus, details dialogs, clipping boundaries, display-only arrow navigation, a fine → coarse → fine pointer change and an inline folder retry.' } },
  beforeEach: servePointerMedia,
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
    // RAC moves focus after mounting the dialog; Escape must reach the dialog, not the body between overlays.
    await waitFor(() => expect(within(document.body).getByRole('dialog', { name: `Reported details for ${parent.title}` })).toHaveFocus())
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
      await waitFor(() => expect(within(document.body).getByRole('dialog', { name: `Reported details for ${noSessionChild.title}` })).toHaveFocus())
      await userEvent.keyboard('{Escape}')
      await waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull())
    }
    const row = (title: string) => within(host).getByRole('row', { name: title })
    // Mixed children: the parent collapses, re-expands keeping focus, then enters its first enabled child past the
    // display-only first one.
    row(parent.title).focus()
    await userEvent.keyboard('{ArrowLeft}')
    await waitFor(() => expect(row(parent.title)).toHaveAttribute('aria-expanded', 'false'))
    await userEvent.keyboard('{ArrowRight}')
    await waitFor(() => expect(row(parent.title)).toHaveAttribute('aria-expanded', 'true'))
    await expect(row(parent.title)).toHaveFocus()
    await userEvent.keyboard('{ArrowRight}')
    await waitFor(() => expect(row(parent.children[1]!.title)).toHaveFocus())
    // Only display-only children: the parent expands and keeps focus on every further expand key, and navigation
    // continues from it.
    const onlyDisplay = displayOnlyParentRow.title
    row(onlyDisplay).focus()
    for (const _ of [0, 1, 2]) {
      await userEvent.keyboard('{ArrowRight}')
      await waitFor(() => expect(row(onlyDisplay)).toHaveAttribute('aria-expanded', 'true'))
      await expect(row(onlyDisplay)).toHaveFocus()
      await expect(row(displayOnlyParentRow.children[0]!.title)).not.toHaveFocus()
    }
    await userEvent.keyboard('{ArrowUp}')
    await waitFor(() => expect(row(retryFolder.textValue)).toHaveFocus())
    // Pointer changes reach rows in the root Tree (the top parent and folder) and in the folder's nested Collection.
    const box = (element: Element) => element.getBoundingClientRect()
    const hit = (element: Element) => { const { left, right, top, bottom } = box(element); return document.elementFromPoint((left + right) / 2, (top + bottom) / 2)! }
    const content = (title: string) => row(title).querySelector(title === retryFolder.textValue ? '[data-folder-label]' : '[data-milestone-row]')!
    const parents = [parent.title, onlyDisplay, retryFolder.textValue]
    const fine = async () => {
      for (const title of parents) {
        await waitFor(() => expect(row(title).querySelector('[data-sidebar-header-toggle], [data-sidebar-disclosure="trailing"]')).toBeNull())
        const chevron = row(title).querySelector('[data-sidebar-disclosure="leading"]')!
        // The desktop 16px × 24px gutter chevron, with the row's content starting right after the gutter.
        await expect({ width: box(chevron).width, height: box(chevron).height }).toEqual({ width: 16, height: 24 })
        await expect(Math.abs(box(content(title)).left - box(chevron).right)).toBeLessThanOrEqual(0.5)
      }
    }
    await fine()
    pointerMedia.emulate(true)
    for (const title of [parent.title, onlyDisplay]) {
      const disclosure = await waitFor(() => { const element = row(title).querySelector('[data-sidebar-disclosure="trailing"]'); expect(element).not.toBeNull(); return element! })
      await expect(row(title).querySelector('[data-sidebar-disclosure="leading"]')).toBeNull()
      await expect(box(disclosure).width).toBeGreaterThanOrEqual(44)
      await expect(box(disclosure).height).toBeGreaterThanOrEqual(44)
    }
    const header = await waitFor(() => { const element = row(retryFolder.textValue).querySelector('[data-sidebar-header-toggle]'); expect(element).not.toBeNull(); return element! })
    await expect(box(header).height).toBeGreaterThanOrEqual(44)
    // The inline retry stays a 44px target above the whole-row header: a tap retries and never toggles the folder.
    const retry = within(row(retryFolder.textValue)).getByRole('button', { name: 'Try again' })
    await expect(box(retry).width).toBeGreaterThanOrEqual(44)
    await expect(box(retry).height).toBeGreaterThanOrEqual(44)
    await expect(retry.contains(hit(retry))).toBe(true)
    await tap(hit(retry))
    await waitFor(() => expect(host).toHaveAttribute('data-retries', '1'))
    await expect(row(retryFolder.textValue)).toHaveAttribute('aria-expanded', 'true')
    // The header's own text is still the disclosure.
    const label = row(retryFolder.textValue).querySelector('[data-folder-label]')!
    await expect(header.contains(hit(label))).toBe(true)
    await tap(hit(label))
    await waitFor(() => expect(row(retryFolder.textValue)).toHaveAttribute('aria-expanded', 'false'))
    await expect(host).toHaveAttribute('data-retries', '1')
    await tap(hit(row(retryFolder.textValue).querySelector('[data-folder-label]')!))
    await waitFor(() => expect(row(onlyDisplay)).toBeVisible())
    pointerMedia.emulate(false)
    await fine()
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
  await waitFor(() => expect(within(document.body).getByRole('dialog', { name: `Reported details for ${noSessionChild.title}` })).toHaveFocus())
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

/**
 * Each subagent cause the projection can carry, with the exact sentence the shared Details dialog paints. Reported rows
 * (an explicit empty list from Claude) count their children; an OpenCode session is not reported by its harness, a
 * running Codex session on an older daemon is not reported by that daemon, and a starting OMP session whose first
 * report has not arrived is not reported yet.
 */
const causeRows = [
  { row: { ...base, ref: 'agent/example-cause-reported', title: 'Claude session', harness: 'claude', subagentCount: 0, subagentObservation: { _tag: 'Reported' } }, details: '0 collapsed children' },
  { row: { ...base, ref: 'agent/example-cause-harness', title: 'OpenCode session', harness: 'opencode', subagentObservation: { _tag: 'NotReported', reason: { _tag: 'Harness' } } }, details: 'Not reported by this harness' },
  { row: { ...base, ref: 'agent/example-cause-daemon', title: 'Codex on an older daemon', harness: 'codex', subagentObservation: { _tag: 'NotReported', reason: { _tag: 'DaemonVersion' } } }, details: 'Not reported by this daemon version' },
  { row: { ...base, ref: 'agent/example-cause-pending', title: 'OMP awaiting its first report', harness: 'omp', status: 'pending', statusLabel: 'Pending', subagentObservation: { _tag: 'Unavailable' } }, details: 'Not reported yet' },
] as const satisfies readonly { readonly row: Row; readonly details: string }[]
/** Opens the row's Reported details from its actions menu and returns the focused dialog. */
const openReportedDetails = async (canvasElement: HTMLElement, title: string) => {
  await userEvent.click(within(canvasElement).getByRole('button', { name: `Actions for ${title}` }))
  await userEvent.click(await within(document.body).findByRole('menuitem', { name: 'Reported agent details' }))
  const dialog = await within(document.body).findByRole('dialog', { name: `Reported details for ${title}` })
  await waitFor(() => expect(dialog).toHaveFocus())
  return dialog
}
const closeReportedDetails = async () => {
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(within(document.body).queryByRole('dialog')).toBeNull())
}

export const ReportedDetails: Story = {
  name: 'Live row · only reported status in Details',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'A live row with unreported status exercises the shared Details dialog; a display-only row with the same props must expose no actions. Four live rows carry each subagent cause (reported zero, not reported by the harness, not reported by a running agent\'s older daemon, not reported yet while an observing agent starts), and Details paints its exact sentence beside Subagents.' } },
  args: { row: base },
  render: () => <main {...stylex.props(styles.root)}><ThemePortal><section aria-label="Live sidebar" {...stylex.props(styles.column)}>
    <MilestoneAgentRow row={{ ...base, title: 'Session without status', status: 'unobserved', statusLabel: 'Unobserved' }} reportedStatusOnly now={storyNow} />
    <div data-display-only-check><MilestoneAgentRow row={{ ...base, title: 'Observed subagent' }} reportedStatusOnly displayOnly now={storyNow} /></div>
    {causeRows.map(({ row }) => <MilestoneAgentRow key={row.ref} row={row} reportedStatusOnly now={storyNow} />)}
  </section></ThemePortal></main>,
  play: async ({ canvasElement }) => {
    const passive = canvasElement.querySelector('[data-display-only-check]')!
    await expect(passive.querySelector('button, [data-agent-menu], [aria-haspopup]')).toBeNull()
    const dialog = await openReportedDetails(canvasElement, 'Session without status')
    await expect(within(dialog).getByText('Name', { exact: true })).toBeVisible()
    await expect(within(dialog).getByText('Session without status', { exact: true })).toBeVisible()
    await expect(within(dialog).queryByText('Status', { exact: true })).toBeNull()
    await expect(dialog).not.toHaveTextContent('Unobserved')
    await closeReportedDetails()
    // Reported zero and not-reported rows paint no count on the row; only the pending row keeps its "?".
    for (const { row, details } of causeRows) {
      const painted = [...canvasElement.querySelectorAll<HTMLElement>('[data-milestone-row]')].find(element => element.querySelector('[data-row-column="title"]')?.textContent === row.title)!
      await expect(painted.querySelector('[data-subagent-count]')?.getAttribute('data-subagent-count') ?? null).toBe(row.subagentObservation._tag === 'Unavailable' ? 'unknown' : null)
      const causeDialog = await openReportedDetails(canvasElement, row.title)
      const label = within(causeDialog).getByText('Subagents', { exact: true })
      await expect(label.tagName).toBe('DT')
      await expect(label.nextElementSibling!.textContent).toBe(details)
      await closeReportedDetails()
    }
  },
}

/**
 * Count fixtures: plural, singular, zero (no chip), unreported, and long titles that must truncate before the count.
 * Claude, Codex and OMP report their subagent lists, so a reported count (an explicit empty list is zero) is `Reported`;
 * an observing agent still starting, before its first report, is `Unavailable` ("?"). `NotReported` rows paint no count
 * at all: an OpenCode session (its harness never reports subagents), a running Codex session whose older daemon omits
 * the field, and a child row, whose wire shape carries no nested report.
 */
const countRow = (key: string, title: string, subagentCount: number | undefined, extra: Partial<Row> = {}): Row => ({
  ...base, ref: `agent/example-count-${key}`, id: `count-${key}`, title, status: 'working', statusLabel: 'Working', host: 'build-3', subagentCount, subagentObservation: { _tag: subagentCount === undefined ? 'Unavailable' : 'Reported' },
  lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 95_000 }, ...extra,
})
const countRows = [
  countRow('plural', 'Align the metric track across every row', 3, { harness: 'claude', needsMe: true }),
  countRow('singular', 'Fix tabs', 1, { harness: 'codex' }),
  countRow('zero', 'Ship the sidebar count', 0, { harness: 'claude' }),
  countRow('unknown', 'OMP agent awaiting its first subagent report', undefined, { harness: 'omp', status: 'pending', statusLabel: 'Pending' }),
  countRow('many', 'Coordinate the long release train across every package', 14, { harness: 'omp', unread: 2 }),
  countRow('harness', 'OpenCode session', undefined, { harness: 'opencode', subagentObservation: { _tag: 'NotReported', reason: { _tag: 'Harness' } } }),
  countRow('daemon', 'Codex on an older daemon', undefined, { harness: 'codex', subagentObservation: { _tag: 'NotReported', reason: { _tag: 'DaemonVersion' } } }),
] as const satisfies readonly Row[]
const countChild = countRow('child', 'Child of the plural row', undefined, { harness: 'claude', parentRef: countRows[0].ref, subagentObservation: { _tag: 'NotReported', reason: { _tag: 'Harness' } } })
const expectedCount: Readonly<Record<string, string | null>> = {
  [countRows[0].title]: '3 subagents', [countRows[1].title]: '1 subagent', [countRows[2].title]: null,
  [countRows[3].title]: '? subagents', [countRows[4].title]: '14 subagents', [countRows[5].title]: null,
  [countRows[6].title]: null, [countChild.title]: null,
}
const longCountTitles: readonly string[] = [countRows[0].title, countRows[3].title, countRows[4].title]
/** The desktop sidebar at its 240px minimum and 320px, and the 320px phone Agents sheet. */
const countHosts = [
  { key: 'desktop-240', width: 240, narrow: false },
  { key: 'desktop-320', width: 320, narrow: false },
  { key: 'phone-320', width: 320, narrow: true },
] as const
/**
 * Layout stress only: a 240px tree includes depth 8 and continues to depth 12 so main's content-sized trailing slot
 * exercises every count mode. Live child rows never carry nested counts (their wire shape has no report); these
 * fixtures only drive the count's geometry at narrow name columns.
 */
const deepCounts = [3, 1, 14, undefined, 3, 1, 14, undefined, 3, 1, 14, undefined, 3] as const
const deepRow = (depth: number): Row => countRow(`deep-${depth}`, `Depth ${depth} · coordinate the long release train`, deepCounts[depth], {
  ...(depth === 0 ? {} : { parentRef: `agent/example-count-deep-${depth - 1}` }),
  children: depth === deepCounts.length - 1 ? [] : [deepRow(depth + 1)],
})
const deepItems: readonly AgentItem[] = [toItem(deepRow(0), 0)]
function DeepCountTree() {
  const [expanded, setExpanded] = React.useState<Set<Key>>(() => new Set(deepCounts.map((_, depth) => `agent/example-count-deep-${depth}`)))
  return <section aria-label="Deep nesting · 240px" data-host="count-deep" {...stylex.props(styles.column, styles.width240, styles.deepTree)}>
    <SidebarTree aria-label="Deeply nested agents" items={deepItems} expandedKeys={expanded} onExpandedChange={setExpanded} renderRow={(item, state) => <MilestoneAgentRow row={item.row} now={storyNow} touchDisclosure={touchDisclosure(state)} />} />
  </section>
}

function SubagentCountMatrix({ scheme }: { readonly scheme: Scheme }) {
  return <main data-testid="subagent-count" data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}><ThemePortal>
    <span aria-hidden="true" data-tone-probe="" {...stylex.props(styles.toneProbe)} />
    <div {...stylex.props(styles.matrix)}>
      {countHosts.map(host => <NarrowLayoutContext.Provider key={host.key} value={host.narrow}>
        <section aria-label={host.key} data-count-host={host.key} data-width={host.width} {...stylex.props(styles.column, host.width === 240 ? styles.width240 : styles.width320)}>
          <h3 {...stylex.props(styles.heading)}>{host.narrow ? `Phone sheet · ${host.width}px` : `Desktop · ${host.width}px`}</h3>
          {countRows.map(row => <MilestoneAgentRow key={row.ref} row={row} now={storyNow} />)}
          <MilestoneAgentRow row={countChild} depth={1} displayOnly now={storyNow} />
        </section>
      </NarrowLayoutContext.Provider>)}
      <DeepCountTree />
    </div>
  </ThemePortal></main>
}

const subagentCountPlay: NonNullable<StoryObj<{ scheme: Scheme }>['play']> = async ({ canvasElement }) => {
  const surfaceRoot = within(canvasElement).getByTestId('subagent-count')
  const soft = getComputedStyle(surfaceRoot.querySelector('[data-tone-probe]')!).color
  const hosts = [...surfaceRoot.querySelectorAll<HTMLElement>('[data-count-host]')]
  await expect(hosts).toHaveLength(countHosts.length)
  for (const host of hosts) {
    await expect(Math.abs(host.getBoundingClientRect().width - Number(host.dataset.width))).toBeLessThanOrEqual(1)
    // Without a handler the count is text, never a button.
    await expect(host.querySelector('button[data-subagent-count]')).toBeNull()
    for (const [title, expected] of Object.entries(expectedCount)) {
      const row = [...host.querySelectorAll<HTMLElement>('[data-milestone-row]')].find(element => element.querySelector('[data-row-column="title"]')?.textContent === title)!
      await expect(Math.abs(row.getBoundingClientRect().height - 48)).toBeLessThanOrEqual(1)
      const chip = row.querySelector<HTMLElement>('[data-subagent-count]')
      // Zero, not-reported (OpenCode, older daemon) and display-only rows carry no count at all, never a "?".
      if (expected === null) { await expect(chip).toBeNull(); continue }
      const tag = chip!.querySelector<HTMLElement>('[data-count-text]')!
      await expect(tag.textContent).toBe(expected)
      // The unreported count shows "?", is spoken as "Unreported subagents" and is described by why it is unknown: static
      // metadata, yet its own focus stop so keyboard users reach its tooltip too.
      if (expected === '? subagents') {
        await expect(chip).toHaveAttribute('data-subagent-count', 'unknown')
        await expect(tag).toHaveAttribute('aria-hidden', 'true')
        await expect(within(chip!).getByRole('img', { name: unreportedName, description: unreportedReason })).toHaveAttribute('tabindex', '0')
      }
      const titleElement = row.querySelector<HTMLElement>('[data-row-column="title"]')!
      const where = `${host.dataset.countHost} · "${title}"`
      // The 240px sidebar's rows go bare: the count alone, its word visually hidden but still in the text for assistive
      // tech. Desktop 320px rows have room for the word; phone rows reserve space for the touch action column.
      const word = chip!.querySelector<HTMLElement>('[data-count-word]')!
      const compact = getComputedStyle(word).position === 'absolute'
      const nameWidth = row.querySelector('[data-row-column="name"]')!.getBoundingClientRect().width
      if (host.dataset.countHost !== 'phone-320') await expect(compact, `${where}: desktop count word mode`).toBe(host.dataset.width === '240')
      await expect(compact, `${where}: compact mode disagrees with the 192px name threshold (name ${nameWidth}px)`).toBe(nameWidth < 192)
      if (compact) {
        await expect(word.getBoundingClientRect().width, `${where}: compact word still painted`).toBeLessThanOrEqual(1)
        await expect(tag.getBoundingClientRect().width, `${where}: bare count wider than its digits`).toBeLessThanOrEqual(28)
      } else await expect(tag.getBoundingClientRect().width, `${where}: full count lost its word`).toBeGreaterThan(40)
      const titleBox = titleElement.getBoundingClientRect(), chipBox = chip!.getBoundingClientRect(), tagBox = tag.getBoundingClientRect()
      const name = row.querySelector('[data-row-column="name"]')!.getBoundingClientRect()
      const time = row.querySelector('[data-row-column="time"]')!.getBoundingClientRect()
      const line2 = row.querySelector('[data-row-column="line2"]')!.getBoundingClientRect()
      // Immediately after the title (the s.sm gap), on line 1, centred with the title and clear of the time column.
      await expect(Math.abs(chipBox.left - titleBox.right - 6), `${where}: count not 6px after the title`).toBeLessThanOrEqual(1)
      await expect(Math.abs((chipBox.top + chipBox.bottom) / 2 - (titleBox.top + titleBox.bottom) / 2), `${where}: count off the title's centre line`).toBeLessThanOrEqual(1)
      await expect(chipBox.bottom, `${where}: count below line 1`).toBeLessThanOrEqual(line2.top + 0.5)
      await expect(chipBox.right, `${where}: count over the time column`).toBeLessThanOrEqual(time.left + 0.5)
      // One line each: the title and the count never wrap, and the count is never clipped.
      await expect(titleBox.height, `${where}: title wraps`).toBeLessThanOrEqual(24)
      await expect(tagBox.height, `${where}: count wraps`).toBeLessThan(24)
      await expect(tag.scrollWidth, `${where}: count clipped`).toBeLessThanOrEqual(tag.clientWidth + 1)
      await expect(getComputedStyle(tag).whiteSpace).toBe('nowrap')
      await expect(getComputedStyle(tag).color).toBe(soft)
      // The title truncates first: whole while title, gap and count fit the name column, otherwise an ellipsis that keeps
      // a readable stretch: the full count starts only above a 192px name column, reserving at least 96px for the title.
      // At 320px every short title is whole.
      const fits = titleElement.scrollWidth + 6 + tagBox.width <= name.width + 1
      if (longCountTitles.includes(title)) await expect(fits, `${where}: fixture title should be long enough to truncate`).toBe(false)
      else if (host.dataset.width === '320') await expect(fits, `${where}: short title should fit beside the count at 320px`).toBe(true)
      if (fits) await expect(titleElement.scrollWidth, `${where}: title truncated although title + count fit (name ${name.width}px)`).toBeLessThanOrEqual(titleElement.clientWidth + 1)
      else {
        await expect(titleElement.scrollWidth, `${where}: title should truncate (name ${name.width}px, count ${tagBox.width}px)`).toBeGreaterThan(titleElement.clientWidth)
        await expect(getComputedStyle(titleElement).textOverflow).toBe('ellipsis')
        await expect(titleBox.width, `${where}: truncated title under the readable minimum (name ${name.width}px, count ${tagBox.width}px)`).toBeGreaterThanOrEqual(96)
      }
    }
  }
  // Deep nesting at 240px: the count fits its name column (bare, then gone) and never covers time or signal; titles
  // still truncate and keep their text, and every count keeps its full text for assistive tech.
  const deep = surfaceRoot.querySelector<HTMLElement>('[data-host="count-deep"]')!
  await expect(Math.abs(deep.getBoundingClientRect().width - 240)).toBeLessThanOrEqual(1)
  const deepRows = await waitFor(() => { const found = [...deep.querySelectorAll<HTMLElement>('[data-milestone-row]')].filter(row => row.closest('[inert]') === null); expect(found).toHaveLength(deepCounts.length); return found })
  const modes = new Set<'full' | 'compact' | 'hidden'>()
  for (const [depth, row] of deepRows.entries()) {
    const titleElement = row.querySelector<HTMLElement>('[data-row-column="title"]')!
    const where = `count-deep · depth ${depth}`
    await expect(titleElement.textContent, `${where}: title text changed`).toBe(`Depth ${depth} · coordinate the long release train`)
    await expect(titleElement.scrollWidth, `${where}: long title should truncate`).toBeGreaterThan(titleElement.clientWidth)
    const chip = row.querySelector<HTMLElement>('[data-subagent-count]')!
    const tag = chip.querySelector<HTMLElement>('[data-count-text]')!
    const count = deepCounts[depth]
    await expect(tag.textContent).toBe(count === undefined ? '? subagents' : `${count} ${count === 1 ? 'subagent' : 'subagents'}`)
    const name = row.querySelector('[data-row-column="name"]')!.getBoundingClientRect()
    // Last turn and signal share the content-sized trailing slot.
    const trailing = row.querySelector('[data-row-column="trailing"]')!.getBoundingClientRect()
    const titleBox = titleElement.getBoundingClientRect()
    const at = `${where} (name ${name.left}–${name.right}px, title ${titleBox.left}–${titleBox.right}px, trailing from ${trailing.left}px)`
    await expect(name.right, `${at}: name column over the trailing slot`).toBeLessThanOrEqual(trailing.left + 0.5)
    await expect(titleBox.left, `${at}: title starts before the name column`).toBeGreaterThanOrEqual(name.left - 0.5)
    await expect(titleBox.right, `${at}: title ends past the name column`).toBeLessThanOrEqual(name.right + 0.5)
    if (getComputedStyle(chip).display === 'none') {
      // Gone: the title takes the whole name column.
      modes.add('hidden')
      await expect(name.width, `${at}: count hidden on a name column of 48px or more`).toBeLessThan(48 + 0.5)
      await expect(Math.abs(titleBox.width - name.width), `${at}: title does not fill the name column`).toBeLessThanOrEqual(0.5)
      continue
    }
    const word = chip.querySelector<HTMLElement>('[data-count-word]')!
    const compact = getComputedStyle(word).position === 'absolute'
    modes.add(compact ? 'compact' : 'full')
    if (compact) {
      // Bare count: the word is visually hidden but still in the text.
      await expect(name.width, `${at}: bare count on a name column of 192px or more`).toBeLessThan(192 + 0.5)
      await expect(name.width, `${at}: bare count on a name column under 48px`).toBeGreaterThanOrEqual(48 - 0.5)
      await expect(word.getBoundingClientRect().width, `${at}: compact word still painted`).toBeLessThanOrEqual(1)
    } else await expect(name.width, `${at}: full count on a name column under 192px`).toBeGreaterThanOrEqual(192 - 0.5)
    const tagBox = tag.getBoundingClientRect()
    const tagAt = `${at} tag ${tagBox.left}–${tagBox.right}px, ${compact ? 'compact' : 'full'}`
    await expect(tagBox.left, `${tagAt}: count starts before the name column`).toBeGreaterThanOrEqual(name.left - 0.5)
    await expect(tagBox.right, `${tagAt}: count ends past the name column`).toBeLessThanOrEqual(name.right + 0.5)
    await expect(tagBox.right, `${tagAt}: count over the trailing slot`).toBeLessThanOrEqual(trailing.left + 0.5)
    await expect(tag.scrollWidth, `${tagAt}: count clipped (client ${tag.clientWidth}px)`).toBeLessThanOrEqual(tag.clientWidth + 1)
    await expect(Math.abs(tagBox.left - titleBox.right - 6), `${tagAt}: count not 6px after the title`).toBeLessThanOrEqual(1)
  }
  await expect(modes.has('compact'), `deep modes seen: ${[...modes].join(', ')}`).toBe(true)
  await expect(modes.has('hidden'), `deep modes seen: ${[...modes].join(', ')}`).toBe(true)
  const [red, green, blue] = getComputedStyle(surfaceRoot).backgroundColor.match(/\d+(\.\d+)?/g)!.map(Number)
  const light = surfaceRoot.getAttribute('data-scheme') === 'light'
  await expect((0.2126 * red! + 0.7152 * green! + 0.0722 * blue!) / 255 > 0.5).toBe(light)
  // The static "?" explains itself at 240px (bare) and 320px (with its word), by pointer hover and keyboard focus.
  for (const key of ['desktop-240', 'desktop-320'] as const) {
    const host = surfaceRoot.querySelector<HTMLElement>(`[data-count-host="${key}"]`)!
    await expectExplained(within(host).getByRole('img', { name: unreportedName }), surfaceRoot, unreportedReason, light, key)
  }
}

const unreportedName = 'Unreported subagents'
const unreportedReason = 'Not reported yet'
/**
 * The count's kit tooltip says `text` on pointer hover and on keyboard focus, portals inside the themed `root` and paints
 * its inverted surface (light on a dark root), and replaces the row's hover card instead of stacking on it.
 */
const expectExplained = async (trigger: HTMLElement, root: HTMLElement, text: string, light: boolean, where: string) => {
  // Establish pointer modality before entering: userEvent sends pointerenter before its first pointermove after a key.
  await userEvent.hover(root)
  await userEvent.hover(trigger)
  const hovered = await within(document.body).findByRole('tooltip').catch(error => { throw new Error(`${where}: hover did not disclose ${text}`, { cause: error }) })
  await expect(hovered, `${where}: hover tooltip`).toHaveTextContent(text)
  await expect(root.contains(hovered), `${where}: tooltip outside the themed root`).toBe(true)
  await expect(paintedLuminance(hovered) > 0.5, `${where}: tooltip surface not inverted`).toBe(!light)
  await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]'), `${where}: hover card stacked under the tooltip`).toBeNull())
  await userEvent.unhover(trigger)
  await waitFor(() => expect(within(document.body).queryByRole('tooltip')).toBeNull())
  // Keyboard modality, then focus: the same explanation opens without a pointer.
  trigger.focus()
  await userEvent.tab({ shift: true })
  await userEvent.tab()
  await expect(trigger).toHaveFocus()
  const focused = await within(document.body).findByRole('tooltip').catch(error => { throw new Error(`${where}: keyboard focus did not disclose ${text}`, { cause: error }) })
  await expect(focused, `${where}: focus tooltip`).toHaveTextContent(text)
  trigger.blur()
  await waitFor(() => expect(within(document.body).queryByRole('tooltip')).toBeNull())
}

export const SubagentCount: StoryObj<{ scheme: Scheme }> = {
  name: 'Subagent count · after the title · 240/320 desktop and phone',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed plural, singular, zero, unreported, not-reported and display-only counts with long titles verify count copy, placement, title truncation and one-line fit at 240px and 320px desktop and phone widths. Claude, Codex and OMP rows report counts (an explicit empty list is zero, no chip); a starting OMP row awaiting its first report shows "?"; an OpenCode row, a running Codex row on an older daemon and a child row (not reported by its harness) paint no count. Tight name columns use a bare count, while desktop 320px retains the word; the full label remains available to assistive tech. The unreported count carries an accessible description and kit tooltip on hover and keyboard focus. A 240px layout-stress tree includes depth 8 and continues to depth 12 to verify bare and hidden count modes without covering trailing metadata; its nested counts drive geometry only, since live child rows carry no nested report.' } },
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <SubagentCountMatrix scheme={scheme} />,
  play: subagentCountPlay,
}
export const SubagentCountLight: StoryObj<{ scheme: Scheme }> = { ...SubagentCount, name: 'Subagent count · after the title · 240/320 desktop and phone · light', args: { scheme: 'light' } }

const countPressItems: readonly AgentItem[] = [toItem(countRow('press', 'Review the subagent switching flow', 2, { harness: 'codex' }), 0), toItem(countRows[3], 0)]
function SubagentCountPressTree({ width }: { readonly width: 240 | 320 }) {
  const [selected, setSelected] = React.useState<string>()
  const [pressed, setPressed] = React.useState<readonly string[]>([])
  // Stable: the row is memoized.
  const onSubagentsPress = React.useCallback((row: Row) => setPressed(refs => [...refs, row.ref]), [])
  return <section aria-label={`Count press · ${width}px`} data-host={width === 320 ? 'count-press' : 'count-press-240'} data-selected={selected} data-subagents={pressed.join(' ') || undefined} {...stylex.props(styles.column, width === 320 ? styles.width320 : styles.width240, styles.tree)}>
    <SidebarTree aria-label="Agents with subagent counts" items={countPressItems} expandedKeys={new Set()} onExpandedChange={() => {}} onAction={key => setSelected(String(key))}
      renderRow={(item, state) => <MilestoneAgentRow row={item.row} now={storyNow} touchDisclosure={touchDisclosure(state)} onSubagentsPress={onSubagentsPress} />} />
  </section>
}

export const SubagentCountPress: StoryObj<{ scheme: Scheme }> = {
  name: 'Subagent count · own press never activates the row',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed 320px and 240px trees with a counted and an unreported agent verify the count button by pointer and keyboard reaches only its own callback, never tree selection, while the row title still activates; the 240px button shows the bare count, and each button explains itself in the themed kit tooltip on hover and keyboard focus, the unreported one also as its accessible description.' } },
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <main data-testid="count-press-root" data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}><ThemePortal><div {...stylex.props(styles.matrix)}><SubagentCountPressTree width={320} /><SubagentCountPressTree width={240} /></div></ThemePortal></main>,
  play: async ({ canvasElement }) => {
    const host = canvasElement.querySelector<HTMLElement>('[data-host="count-press"]')!
    const [counted, unknown] = countPressItems.map(item => item.row) as [Row, Row]
    const row = (title: string) => within(host).getByRole('row', { name: title })
    // The count is not part of the row's accessible name.
    const chip = await waitFor(() => within(row(counted.title)).getByRole('button', { name: '2 subagents' }))
    await expect(chip.getBoundingClientRect().height).toBeGreaterThanOrEqual(24)
    await userEvent.click(chip)
    await waitFor(() => expect(host).toHaveAttribute('data-subagents', counted.ref))
    await expect(host).not.toHaveAttribute('data-selected')
    await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull())
    // Keyboard: Enter on the focused count is its own press too.
    chip.focus()
    await expect(chip).toHaveFocus()
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(host).toHaveAttribute('data-subagents', `${counted.ref} ${counted.ref}`))
    await expect(host).not.toHaveAttribute('data-selected')
    // Unreported: "? subagents" on screen, "Unreported subagents" spoken.
    const unknownChip = within(row(unknown.title)).getByRole('button', { name: 'Unreported subagents' })
    await expect(unknownChip.querySelector('[data-count-text]')).toHaveTextContent('? subagents')
    await userEvent.click(unknownChip)
    await waitFor(() => expect(host).toHaveAttribute('data-subagents', `${counted.ref} ${counted.ref} ${unknown.ref}`))
    await expect(host).not.toHaveAttribute('data-selected')
    // Control: the title still activates the tree item, so the negatives above are not vacuous.
    await userEvent.click(row(counted.title).querySelector('[data-row-column="title"]')!)
    await waitFor(() => expect(host).toHaveAttribute('data-selected', counted.ref))
    await expect(host).toHaveAttribute('data-subagents', `${counted.ref} ${counted.ref} ${unknown.ref}`)
    // Explained at both widths: bare "2" at 240px with "2 subagents" as its name and tooltip, the word painted at 320px;
    // the unreported button is described by its reason, which is also its tooltip.
    const root = within(canvasElement).getByTestId('count-press-root')
    const light = root.getAttribute('data-scheme') === 'light'
    for (const key of ['count-press', 'count-press-240'] as const) {
      const tree = root.querySelector<HTMLElement>(`[data-host="${key}"]`)!
      const known = within(within(tree).getByRole('row', { name: counted.title })).getByRole('button', { name: '2 subagents' })
      await expect(getComputedStyle(known.querySelector('[data-count-word]')!).position, `${key}: count word mode`).toBe(key === 'count-press-240' ? 'absolute' : 'static')
      await expectExplained(known, root, '2 subagents', light, `${key} · counted`)
      const unreported = within(within(tree).getByRole('row', { name: unknown.title })).getByRole('button', { name: unreportedName, description: unreportedReason })
      await expectExplained(unreported, root, unreportedReason, light, `${key} · unreported`)
    }
  },
}
export const SubagentCountPressLight: StoryObj<{ scheme: Scheme }> = { ...SubagentCountPress, name: 'Subagent count · own press never activates the row · light', args: { scheme: 'light' } }
/** The first painted background at or above `element`, as relative luminance: overlays are judged by the surface they paint. */
const paintedLuminance = (element: Element) => {
  for (let node: Element | null = element; node !== null; node = node.parentElement) {
    const [red, green, blue, alpha = 1] = getComputedStyle(node).backgroundColor.match(/\d+(\.\d+)?/g)!.map(Number)
    if (alpha > 0) return (0.2126 * red! + 0.7152 * green! + 0.0722 * blue!) / 255
  }
  throw new Error('No painted background above the overlay')
}
const contextRow: Row = { ...parent, ref: 'agent/example-context', title: 'Reconcile gateway schema', children: [], context: { _tag: 'Known', usedPercent: 18, usedTokens: 36_000, windowTokens: 200_000, observedAt: storyNow - 60_000 } }

/**
 * The row's hover card and Reported details dialog in a themed root with no host portal layer, as an app story or a
 * bare host renders them: both must portal inside the themed surface and paint its scheme, never the document body's.
 */
export const NativeOverlaysThemed: StoryObj<{ scheme: Scheme }> = {
  tags: ['critical'], name: 'Native hover card and details · scheme reaches overlays',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'A fixed row with reported context opens the hover card and the Reported details dialog inside a scheme-themed root without a host portal layer, verifying both overlays portal into and paint that scheme.' } },
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <main data-testid="overlay-root" data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}>
    <section aria-label="Sidebar" {...stylex.props(styles.column)}><MilestoneAgentRow row={contextRow} now={storyNow} /></section>
  </main>,
  play: async ({ canvasElement }) => {
    const root = within(canvasElement).getByTestId('overlay-root')
    const light = root.getAttribute('data-scheme') === 'light'
    const row = root.querySelector<HTMLElement>('[data-milestone-row]')!
    await userEvent.hover(row)
    const card = await waitFor(() => { const element = document.querySelector<HTMLElement>('[data-testid="agent-hover-card"]'); expect(element).not.toBeNull(); return element! })
    // Portalled inside the themed root, so the theme's tokens resolve on the overlay itself.
    await expect(root.contains(card)).toBe(true)
    await expect(paintedLuminance(card) > 0.5).toBe(light)
    await userEvent.unhover(row)
    await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull())
    await userEvent.click(row.querySelector<HTMLElement>('[data-agent-menu]')!)
    const menu = await within(document.body).findByRole('menu', { name: `Actions for ${contextRow.title}` })
    await expect(root.contains(menu)).toBe(true)
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Reported agent details' }))
    const dialog = await within(document.body).findByRole('dialog', { name: `Reported details for ${contextRow.title}` })
    await expect(root.contains(dialog)).toBe(true)
    await expect(paintedLuminance(dialog) > 0.5).toBe(light)
    await expect(dialog.querySelector('[data-context-observation]')).toHaveTextContent('18%')
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(within(document.body).queryByRole('dialog')).toBeNull())
  },
}
export const NativeOverlaysThemedLight: StoryObj<{ scheme: Scheme }> = { ...NativeOverlaysThemed, name: 'Native hover card and details · scheme reaches overlays · light', args: { scheme: 'light' } }

const reachTitle = 'GatewaySchemaReconciliationAcrossEveryHost'
const reachRows: readonly Row[] = [
  { ...base, ref: 'agent/example-reach-empty', title: reachTitle },
  { ...base, ref: 'agent/example-reach-time', title: reachTitle, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 95_000 } },
  { ...base, ref: 'agent/example-reach-waiting', title: 'Waiting on your answer', status: 'waiting', statusLabel: statuses.waiting.label, needsMe: true, lastTurn: { _tag: 'Known', kind: 'activity', at: storyNow - 30_000 } },
  { ...base, ref: 'agent/example-reach-needs', title: 'Working but asking', status: 'working', statusLabel: 'Working', needsMe: true },
]

/** Title reach: an empty trailing slot reserves nothing, hover actions overlay and fade line 2, and the "!" shows only as a sole cue. */
export const TitleReach: StoryObj<{ scheme: Scheme }> = {
  tags: ['critical'], name: 'Title reach · empty trailing slot, overlay fade, sole-cue attention',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed long titles with and without a last turn, a waiting needs-you row and a working needs-you row at the default sidebar width verify that the title reaches the row edge when nothing trails it, that the ghost action trigger overlays a fading line 2, and that the attention mark appears only when it is the row\'s sole needs-you cue.' } },
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}><ThemePortal>
    <section aria-label="Sidebar" data-host="reach" {...stylex.props(styles.column)}>{reachRows.map(row => <MilestoneAgentRow key={row.ref} row={row} now={storyNow} />)}</section>
  </ThemePortal></main>,
  play: async ({ canvasElement }) => {
    const rows = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="reach"] [data-milestone-row]')]
    const [empty, timed, waiting, needs] = rows
    const box = (row: Element, column: string) => row.querySelector(`[data-row-column="${column}"]`)!.getBoundingClientRect()
    // Nothing trails: no trailing slot, and the title ends where line 2 ends.
    await expect(empty!.querySelector('[data-row-column="trailing"]')).toBeNull()
    await expect(Math.abs(box(empty!, 'name').right - box(empty!, 'line2').right)).toBeLessThanOrEqual(1)
    // A last turn takes only its own width: the name column (title plus count) still ends before it, and gains everything the empty row gained.
    await expect(box(timed!, 'name').right).toBeLessThanOrEqual(box(timed!, 'time').left)
    await expect(box(empty!, 'name').width - box(timed!, 'name').width).toBeLessThanOrEqual(box(timed!, 'trailing').width + Number.parseFloat(getComputedStyle(timed!).columnGap) + 1)
    // Waiting already says "Needs you" with its glyph: no third "!" signal. A working row asking for you keeps it, bold.
    await expect(waiting!.querySelector('[data-row-column="attention"]')).toBeNull()
    await expect(waiting!.querySelector('[data-row-field="status-label"]')).toHaveTextContent('Needs you')
    const attention = needs!.querySelector<HTMLElement>('[data-row-column="attention"]')!
    await expect(attention).toBeVisible()
    await expect(getComputedStyle(attention).fontWeight).toBe('700')
    // Hover: the ghost trigger shows over line 2's end, which fades beneath it instead of reserving a column.
    const trigger = empty!.querySelector<HTMLElement>('[data-agent-menu]')!
    await expect(getComputedStyle(trigger).opacity).toBe('0')
    await expect(empty!.querySelector('[data-row-column="line2"]')).not.toHaveAttribute('data-menu-overlay')
    await userEvent.hover(empty!)
    await waitFor(() => expect(getComputedStyle(trigger).opacity).toBe('1'))
    const line2 = empty!.querySelector<HTMLElement>('[data-row-column="line2"]')!
    await expect(line2).toHaveAttribute('data-menu-overlay', 'true')
    const mask = getComputedStyle(line2).maskImage || getComputedStyle(line2).getPropertyValue('-webkit-mask-image')
    await expect(mask).toMatch(/gradient/)
    await expect(getComputedStyle(trigger).borderTopWidth).toBe('0px')
    await expect(getComputedStyle(trigger).backgroundColor).toBe('rgba(0, 0, 0, 0)')
    await expect(Math.abs(box(empty!, 'name').right - line2.getBoundingClientRect().right)).toBeLessThanOrEqual(1)
    await userEvent.unhover(empty!)
    await waitFor(() => expect(line2).not.toHaveAttribute('data-menu-overlay'))
  },
}
export const TitleReachLight: StoryObj<{ scheme: Scheme }> = { ...TitleReach, name: 'Title reach · empty trailing slot, overlay fade, sole-cue attention · light', args: { scheme: 'light' } }

/**
 * Target size follows the pointer, not the width: the same rows at the desktop sidebar width stay dense under a fine
 * pointer and grow 44px always-visible ghost actions, without hover UI, under a coarse one.
 */
export const PointerSizing: StoryObj<{ scheme: Scheme }> = {
  tags: ['critical'], name: 'Pointer sizing · fine stays dense, coarse grows ghost targets',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed rows at the desktop sidebar width with no narrow host switch fine → coarse → fine through the kit pointer query, verifying dense hover-only actions for fine pointers and 44px always-visible borderless actions with no hover card for coarse ones.' } },
  beforeEach: servePointerMedia,
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}><ThemePortal>
    <section aria-label="Sidebar" data-host="pointer" {...stylex.props(styles.column)}>{[contextRow, stateRows[0]!].map(row => <MilestoneAgentRow key={row.ref} row={row} now={storyNow} />)}</section>
  </ThemePortal></main>,
  play: async ({ canvasElement }) => {
    const row = canvasElement.querySelector<HTMLElement>('[data-host="pointer"] [data-milestone-row]')!
    const trigger = row.querySelector<HTMLElement>('[data-agent-menu]')!
    const fine = async () => {
      await waitFor(() => expect(row).not.toHaveAttribute('data-touch'))
      await expect(trigger.getBoundingClientRect().width).toBeLessThan(44)
      await expect(getComputedStyle(trigger).opacity).toBe('0')
      await expect(Math.abs(row.getBoundingClientRect().height - 48)).toBeLessThanOrEqual(1)
    }
    await fine()
    pointerMedia.emulate(true)
    await waitFor(() => expect(row).toHaveAttribute('data-touch', 'true'))
    const target = trigger.getBoundingClientRect()
    await expect(target.width).toBeGreaterThanOrEqual(44)
    await expect(target.height).toBeGreaterThanOrEqual(44)
    await expect(getComputedStyle(trigger).opacity).toBe('1')
    // A quiet trailing control: no border and no fill at rest.
    await expect(getComputedStyle(trigger).borderTopWidth).toBe('0px')
    await expect(getComputedStyle(trigger).backgroundColor).toBe('rgba(0, 0, 0, 0)')
    // The title stops before the reserved action column.
    await expect(row.querySelector('[data-row-column="title"]')!.getBoundingClientRect().right).toBeLessThanOrEqual(target.left)
    // Coarse pointers get no hover UI, even past the hover-intent delay.
    await userEvent.hover(row)
    const started = performance.now()
    do {
      await expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull()
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
    } while (performance.now() - started < 200)
    await userEvent.unhover(row)
    pointerMedia.emulate(false)
    await fine()
  },
}
export const PointerSizingLight: StoryObj<{ scheme: Scheme }> = { ...PointerSizing, name: 'Pointer sizing · fine stays dense, coarse grows ghost targets · light', args: { scheme: 'light' } }

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
  folderRow: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0 },
  folderStatus: { display: 'flex', alignItems: 'center', gap: s.xs, flexShrink: 0, color: ink.fgMuted, fontSize: t.denseSize },
  clipped: { width: g.sidebarDefault, overflow: 'hidden', transform: 'translateX(0)' },
  width240: { width: '240px' },
  width320: { width: '320px' },
  // Thirteen 48px rows, all mounted.
  deepTree: { height: '760px' },
  heading: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium },
  // Resolves the surface's own tokens for the tone assertions: soft (neutral states), muted (stale), danger (never stale).
  toneProbe: { position: 'absolute', width: s.zero, height: s.zero, overflow: 'hidden', color: ink.fgSoft, backgroundColor: ink.fgMuted, borderTopColor: tone.dangerFg },
})
