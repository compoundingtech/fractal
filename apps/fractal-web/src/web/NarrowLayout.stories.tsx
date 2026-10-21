import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as Atom from 'effect/reactivity/Atom'
import { RuntimeId, decodeUnknownSync } from '@fractal/client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, waiting } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveAgentWorkspace, narrowLayoutQuery } from './LiveAgentWorkspace.tsx'

// The real workspace over the deterministic fixture world, with resources supported as on the live gateway.
const resources = { byAgent: () => Atom.make(observed({ value: { items: [], nextCursor: null } })), byId: () => Atom.make(waiting), subjects: Atom.make([]), loadMore: () => undefined, refresh: () => undefined, resolveFile: () => undefined }
const storyAgent = 'agent/build-host-a/workbench-shell'
// Fixture agents report no runtime, so their terminal is unavailable ("This agent has no terminal.").
const source = { ...fixtureSource({ world: fixtureProjections }), resources }
// The same world where the roster reports the story agent's runtime, as the live roster does for a seat with a terminal.
// Its screen is the fixture world's terminal screen for that agent. A second seat with a terminal carries a long name,
// the phone header's and tab row's worst case at 390px.
const storyRuntime = decodeUnknownSync(RuntimeId)('runtime/build-host-a/workbench-shell')
const longAgent = 'agent/build-host-a/palette-review'
const longName = 'Row projection reconciler for long-running workspace sessions'
const terminalSource = { ...fixtureSource({ world: { ...fixtureProjections, agents: fixtureProjections.agents.map(row => row.id === storyAgent ? { ...row, runtime_ids: [storyRuntime] } : row.id === longAgent ? { ...row, name: longName, runtime_ids: [decodeUnknownSync(RuntimeId)('runtime/build-host-a/palette-review')] } : row) } }), resources }

const NarrowLayoutStory = ({ terminalRuntime = false }: { readonly scheme: 'dark' | 'light'; readonly terminalRuntime?: boolean }) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  return <DataSourceProvider source={terminalRuntime ? terminalSource : source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>
}

/** Below 900 px the thread is the only thing on screen; one Agent details button opens the agent's actions and Details as a sheet. Wider windows keep the inline panel. */
const meta = {
  title: 'Fractal/Narrow layout',
  component: NarrowLayoutStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  // The workspace keeps its scheme, agent and panes in persisted UI state; every story starts from the same state.
  // A storage event makes the already-loaded persistence merge it, as it does for another tab's write.
  beforeEach: ({ args }) => {
    const entry = (value: unknown) => ({ version: 1, value, clock: Date.now(), writer: 'narrow-layout-story' })
    const raw = JSON.stringify({ version: 1, entries: { 'round2.scheme': entry(args.scheme), 'round2.agent': entry(storyAgent), 'round2.sidebarClosed': entry(false), [`round2.panes.${storyAgent}`]: entry([]) } })
    window.localStorage.setItem('wf.ui@1', raw)
    window.dispatchEvent(new StorageEvent('storage', { key: 'wf.ui@1', newValue: raw }))
  },
} satisfies Meta<typeof NarrowLayoutStory>
export default meta
type Story = StoryObj<typeof meta>

/**
 * The workspace routes through `history.pushState`. A play that selects an agent or a pane puts the frame back on the
 * story's own URL when it ends, so Storybook and its runners still address this story when it finishes.
 */
const restoringStoryUrl = (play: NonNullable<Story['play']>): NonNullable<Story['play']> => async context => {
  const storyUrl = window.location.href
  try { await play(context) } finally { window.history.replaceState(window.history.state, '', storyUrl) }
}

const workspace = async (canvasElement: HTMLElement) => {
  await waitFor(() => expect(within(canvasElement).getByText('Let me find where groups close.')).toBeVisible(), { timeout: 60000 })
  const narrow = window.matchMedia(narrowLayoutQuery).matches
  return { canvas: within(canvasElement), narrow, header: within(within(canvasElement).getByTestId('thread-header')) }
}
/** Wide windows keep the docked sidebar, the full header cluster with the icon terminal toggle, and the inline inspector. */
const assertWide = async (canvasElement: HTMLElement) => {
  const { canvas, header } = await workspace(canvasElement)
  await expect(canvas.getByRole('complementary', { name: 'Agents' })).toBeVisible()
  await expect(canvas.queryByRole('dialog', { name: 'Agents' })).toBeNull()
  await expect(header.queryByRole('button', { name: 'Expand sidebar' })).toBeNull()
  await expect(header.getByRole('button', { name: 'Toggle right panel' })).toBeVisible()
  await expect(header.getByRole('button', { name: 'Actions for WorkbenchShell' })).toBeVisible()
  await expect(header.getByRole('button', { name: 'Toggle terminal drawer' })).toBeVisible()
  await expect(header.getByText('build-host-a')).toBeVisible()
  await expect(header.queryByRole('button', { name: 'Agent details' })).toBeNull()
  await expect(header.queryByRole('button', { name: 'Terminal' })).toBeNull()
}
/**
 * The inspector is a lazy chunk whose loading fallback carries the same "Agent inspector" landmark, so its own tab
 * list marks the loaded panel. Tabs are read from that list: the Changes view's diff panel has its own tab strip.
 */
const loadedInspector = async (scope: HTMLElement) => {
  const views = await within(scope).findByRole('tablist', { name: 'Agent inspector views' }, { timeout: 15000 })
  const inspector = views.closest<HTMLElement>('aside[aria-label="Agent inspector"]')!
  return { inspector, views, tab: (name: 'Changes' | 'Details') => within(views).getByRole('tab', { name }) }
}
/** The docked inspector opens on Details; its separator names the visible view and follows the tab selection. */
const openWideDetails = async (canvasElement: HTMLElement) => {
  await assertWide(canvasElement)
  const { canvas, header } = await workspace(canvasElement)
  await userEvent.click(header.getByRole('button', { name: 'Toggle right panel' }))
  const { inspector, tab } = await loadedInspector(canvasElement)
  await expect(tab('Details')).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByRole('separator', { name: 'Resize Details' })).toBeVisible()
  await userEvent.click(tab('Changes'))
  await expect(tab('Changes')).toHaveAttribute('aria-selected', 'true')
  await expect(await within(inspector).findByRole('complementary', { name: 'Changes' }, { timeout: 15000 })).toBeVisible()
  await expect(canvas.getByRole('separator', { name: 'Resize Changes' })).toBeVisible()
  await expect(canvas.queryByRole('separator', { name: 'Resize Details' })).toBeNull()
  await expect(canvas.queryByRole('dialog', { name: 'Agent details' })).toBeNull()
  // A remounted inspector starts on Details again, and so does its separator.
  await userEvent.click(within(inspector).getByRole('button', { name: 'Close agent inspector' }))
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: 'Agent inspector' })).toBeNull())
  await userEvent.click(header.getByRole('button', { name: 'Toggle right panel' }))
  const reopened = await loadedInspector(canvasElement)
  await expect(reopened.tab('Details')).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByRole('separator', { name: 'Resize Details' })).toBeVisible()
}

/**
 * The phone header's four controls in order (the conversation actions between the chevron and Agent details; Terminal
 * lives in the tab row). Each is a visible 44×44px target inside the window, clear of the breadcrumb and of each other;
 * the breadcrumb keeps at least a quarter of the window, and the header never scrolls sideways.
 */
const assertPhoneHeader = async (canvasElement: HTMLElement, agentName = 'WorkbenchShell') => {
  const node = within(canvasElement).getByTestId('thread-header')
  const header = within(node)
  const buttons = header.getAllByRole('button')
  await expect(buttons.map(button => button.getAttribute('aria-label'))).toEqual(['Expand sidebar', `Actions for ${agentName}`, 'Conversation menu', 'Agent details'])
  for (const name of ['Resources', 'View KDL', 'Terminal', 'Toggle terminal drawer', 'Toggle right panel']) await expect(header.queryByRole('button', { name })).toBeNull()
  const lane = node.getBoundingClientRect()
  await expect(lane.left).toBeGreaterThanOrEqual(0)
  await expect(lane.right).toBeLessThanOrEqual(window.innerWidth)
  await expect(node.scrollWidth).toBeLessThanOrEqual(node.clientWidth)
  const rects = buttons.map(button => button.getBoundingClientRect())
  for (const [index, rect] of rects.entries()) {
    await expect(buttons[index]).toBeVisible()
    await expect(rect.width).toBeGreaterThanOrEqual(44)
    await expect(rect.height).toBeGreaterThanOrEqual(44)
    await expect(rect.left).toBeGreaterThanOrEqual(0)
    await expect(rect.right).toBeLessThanOrEqual(window.innerWidth)
    for (const other of rects) if (other !== rect) await expect(rect.left < other.right && rect.right > other.left && rect.top < other.bottom && rect.bottom > other.top).toBe(false)
  }
  const breadcrumb = header.getByRole('navigation', { name: 'Breadcrumb' })
  await expect(breadcrumb).toBeVisible()
  const crumb = breadcrumb.getBoundingClientRect()
  await expect(crumb.left).toBeGreaterThanOrEqual(rects[0]!.right)
  await expect(crumb.right).toBeLessThanOrEqual(rects[1]!.left)
  // With four controls the host / agent breadcrumb keeps a readable share of the row.
  await expect(crumb.width).toBeGreaterThanOrEqual(window.innerWidth / 4)
  return { header, breadcrumb }
}
/**
 * The phone tab row: the Workspace views tabs, then one labelled 44px Terminal toggle fixed at the row's end, outside
 * the tab list. The tabs scroll and truncate inside their list, which ends before Terminal; the row never scrolls.
 */
const assertPhoneTabRow = async (canvasElement: HTMLElement) => {
  const canvas = within(canvasElement)
  const list = canvas.getByRole('tablist', { name: 'Workspace views' })
  const row = list.parentElement!
  const terminal = within(row).getByRole('button', { name: 'Terminal' })
  await expect(canvas.getAllByRole('button', { name: 'Terminal' })).toEqual([terminal])
  await expect(list.contains(terminal)).toBe(false)
  // The Terminal toggle's visible label carries its meaning; touch has no hover tooltip.
  await expect(terminal).toBeVisible()
  await expect(terminal).toHaveTextContent('Terminal')
  await expect(terminal).not.toHaveAttribute('title')
  const target = terminal.getBoundingClientRect()
  await expect(target.width).toBeGreaterThanOrEqual(44)
  await expect(target.height).toBeGreaterThanOrEqual(44)
  await expect(target.left).toBeGreaterThanOrEqual(0)
  await expect(target.right).toBeLessThanOrEqual(window.innerWidth)
  const strip = list.getBoundingClientRect()
  await expect(strip.right).toBeLessThanOrEqual(target.left)
  await expect(getComputedStyle(list).overflowX).toBe('auto')
  await expect(row.scrollWidth).toBeLessThanOrEqual(row.clientWidth)
  const tabs = within(list).getAllByRole('tab')
  await expect(tabs.length).toBeGreaterThan(0)
  for (const tab of tabs) {
    const rect = tab.getBoundingClientRect()
    await expect(rect.width).toBeGreaterThanOrEqual(44)
    await expect(rect.height).toBeGreaterThanOrEqual(44)
    // Only the part inside the scrolling list paints; it never reaches under Terminal.
    const visibleRight = Math.min(rect.right, strip.right)
    if (visibleRight > Math.max(rect.left, strip.left)) await expect(visibleRight).toBeLessThanOrEqual(target.left)
    const label = tab.firstElementChild!.getBoundingClientRect()
    await expect(label.right).toBeLessThanOrEqual(rect.right)
  }
  return { terminal, list }
}

/** The thread fills the window; the header keeps the chevron, host / agent breadcrumb, status, the conversation actions and one Agent details button; Terminal ends the tab row. */
export const Drawers: Story = { play: async ({ canvasElement }) => {
  const { canvas, narrow, header } = await workspace(canvasElement)
  if (!narrow) return assertWide(canvasElement)
  // The sidebar is not in the layout until opened.
  await expect(canvas.queryByRole('complementary', { name: 'Agents' })).toBeNull()
  await expect(canvas.queryByRole('navigation', { name: 'Agent roster' })).toBeNull()
  await expect(Math.abs(canvas.getByRole('region', { name: 'Agent workspace' }).getBoundingClientRect().width - window.innerWidth)).toBeLessThanOrEqual(1)
  await expect(header.getByRole('button', { name: 'Expand sidebar' })).toBeVisible()
  const opener = header.getByRole('button', { name: 'Agent details' })
  await expect(opener).toHaveAttribute('aria-expanded', 'false')
  await expect(opener).toHaveAttribute('aria-haspopup', 'dialog')
  const breadcrumb = header.getByRole('navigation', { name: 'Breadcrumb' })
  await expect(breadcrumb).toHaveTextContent('build-host-a')
  await expect(breadcrumb).toHaveTextContent('WorkbenchShell')
  await assertPhoneHeader(canvasElement)
  const { terminal } = await assertPhoneTabRow(canvasElement)
  // The fixture roster reports no runtime for this agent: the toggle stays, disabled and described by why.
  await expect(terminal).toHaveAttribute('aria-pressed', 'false')
  await expect(terminal).toHaveAttribute('aria-disabled', 'true')
  await expect(terminal).toHaveAccessibleDescription('This agent has no terminal.')
  await expect(canvas.queryByRole('region', { name: 'Terminal drawer' })).toBeNull()
  // Pressing it paints the reason in a popover instead of toggling; Escape returns focus to the toggle.
  await userEvent.click(terminal)
  const reason = await within(document.body).findByRole('dialog', { name: 'Terminal unavailable' })
  await expect(reason).toHaveTextContent('This agent has no terminal.')
  // The popover mounts at the start of its entrance transition (opacity 0) and is painted once it settles.
  await waitFor(() => expect(reason).toBeVisible())
  await expect(terminal).toHaveAttribute('aria-pressed', 'false')
  await expect(canvas.queryByRole('region', { name: 'Terminal drawer' })).toBeNull()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(within(document.body).queryByRole('dialog', { name: 'Terminal unavailable' })).toBeNull())
  await waitFor(() => expect(terminal).toHaveFocus())
  await assertPhoneTabRow(canvasElement)
} }
export const DrawersLight: Story = { ...Drawers, args: { scheme: 'light' } }

const openDetailsSheet = async (canvasElement: HTMLElement) => {
  const { canvas, header } = await workspace(canvasElement)
  const opener = header.getByRole('button', { name: 'Agent details' })
  await userEvent.click(opener)
  const sheet = await canvas.findByRole('dialog', { name: 'Agent details' })
  // On a cold dev server the inspector chunk's first load takes longer than findBy's default second.
  const inspector = await loadedInspector(sheet)
  await waitFor(() => expect(sheet.contains(document.activeElement)).toBe(true))
  return { canvas, sheet, opener, ...inspector }
}
/** The right sheet holds the agent inspector: Details first, Changes one tab away; the conversation actions stay in the header. */
export const DrawersOpen: Story = { play: async ({ canvasElement }) => {
  const { narrow } = await workspace(canvasElement)
  if (!narrow) return openWideDetails(canvasElement)
  const { canvas, sheet, inspector, views, tab, opener } = await openDetailsSheet(canvasElement)
  await expect(opener).toHaveAttribute('aria-expanded', 'true')
  // The sheet header's X is its only close; the header's actions are not repeated in the sheet.
  await expect(within(sheet).getByRole('button', { name: 'Close agent details' })).toBeVisible()
  await expect(within(sheet).queryByRole('button', { name: 'Close agent inspector' })).toBeNull()
  await expect(within(sheet).queryByRole('toolbar', { name: 'Agent actions' })).toBeNull()
  for (const name of ['Actions for WorkbenchShell', 'Conversation menu', 'Terminal']) await expect(within(sheet).queryByRole('button', { name })).toBeNull()
  await expect(canvas.queryByRole('separator', { name: /^Resize / })).toBeNull()
  const tabs = within(views).getAllByRole('tab')
  await expect(tabs.map(node => node.textContent?.trim())).toEqual(['Changes', 'Details'])
  for (const node of tabs) {
    const rect = node.getBoundingClientRect()
    await expect(rect.width).toBeGreaterThanOrEqual(44)
    await expect(rect.height).toBeGreaterThanOrEqual(44)
  }
  await expect(tab('Details')).toHaveAttribute('aria-selected', 'true')
  // Changes scrolls as one column inside the sheet; the captured-change rows sit above the 'Captured change' header, never under it.
  await userEvent.click(tab('Changes'))
  await expect(tab('Changes')).toHaveAttribute('aria-selected', 'true')
  const changes = await within(inspector).findByRole('complementary', { name: 'Changes' }, { timeout: 15000 })
  const capturedHeader = await within(changes).findByRole('heading', { name: 'Captured change' })
  const rows = [...changes.querySelectorAll('[data-captured-change]')]
  await expect(rows.length).toBeGreaterThan(0)
  const headerTop = capturedHeader.getBoundingClientRect().top
  for (const row of rows) await expect(row.getBoundingClientRect().bottom).toBeLessThanOrEqual(headerTop)
  await expect(changes.getBoundingClientRect().width).toBeLessThanOrEqual(sheet.getBoundingClientRect().width)
  // A reopened sheet mounts a fresh inspector on Details.
  await userEvent.click(within(sheet).getByRole('button', { name: 'Close agent details' }))
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: 'Agent details' })).toBeNull())
  const reopened = await openDetailsSheet(canvasElement)
  await expect(reopened.tab('Details')).toHaveAttribute('aria-selected', 'true')
} }
/** Keyboard path through both sheets: each opens with focus inside, Escape or the backdrop closes it, and focus returns to its opener. */
export const DrawersFlow: Story = { play: restoringStoryUrl(async ({ canvasElement }) => {
  const { canvas, narrow, header } = await workspace(canvasElement)
  if (!narrow) return assertWide(canvasElement)
  // Agents, from the left.
  const chevron = header.getByRole('button', { name: 'Expand sidebar' })
  await expect(chevron).toHaveAttribute('aria-haspopup', 'dialog')
  await expect(chevron).toHaveAttribute('aria-expanded', 'false')
  await userEvent.hover(chevron)
  chevron.focus()
  await userEvent.keyboard('{Enter}')
  const agents = await canvas.findByRole('dialog', { name: 'Agents' })
  await expect(within(agents).getByRole('navigation', { name: 'Agent roster' })).toBeVisible()
  await expect(agents.getBoundingClientRect().left).toBe(0)
  await waitFor(() => expect(agents.contains(document.activeElement)).toBe(true))
  await expect(chevron).toHaveAttribute('aria-expanded', 'true')
  await expect(agents.id).not.toBe('')
  await expect(chevron).toHaveAttribute('aria-controls', agents.id)
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: 'Agents' })).toBeNull())
  await waitFor(() => expect(chevron).toHaveFocus())
  await expect(chevron).toHaveAttribute('aria-expanded', 'false')
  await expect(chevron).not.toHaveAttribute('aria-controls')
  await userEvent.keyboard('{Enter}')
  const backdrop = (await canvas.findByRole('dialog', { name: 'Agents' })).parentElement!.parentElement!
  await userEvent.click(backdrop)
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: 'Agents' })).toBeNull())
  await waitFor(() => expect(chevron).toHaveFocus())
  // Choosing an agent closes the sheet on its thread.
  await userEvent.keyboard('{Enter}')
  await userEvent.click(within(await canvas.findByRole('dialog', { name: 'Agents' })).getByRole('row', { name: /^MissionsUi,/ }))
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: 'Agents' })).toBeNull())
  await waitFor(() => expect(header.getByRole('navigation', { name: 'Breadcrumb' })).toHaveTextContent('MissionsUi'))
  // Agent details, from the right.
  const opener = header.getByRole('button', { name: 'Agent details' })
  await userEvent.hover(opener)
  opener.focus()
  await userEvent.keyboard('{Enter}')
  const sheet = await canvas.findByRole('dialog', { name: 'Agent details' })
  await waitFor(() => expect(sheet.contains(document.activeElement)).toBe(true))
  await expect(opener).toHaveAttribute('aria-expanded', 'true')
  await userEvent.tab()
  await expect(sheet.contains(document.activeElement)).toBe(true)
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: 'Agent details' })).toBeNull())
  await waitFor(() => expect(opener).toHaveFocus())
  await expect(opener).toHaveAttribute('aria-expanded', 'false')
  // The close button is the pointer path out.
  await userEvent.keyboard('{Enter}')
  const reopened = await canvas.findByRole('dialog', { name: 'Agent details' })
  await userEvent.click(within(reopened).getByRole('button', { name: 'Close agent details' }))
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: 'Agent details' })).toBeNull())
}) }
/**
 * The terminal toggle, off then on then off, for an agent whose roster row reports a runtime: on a phone it is the
 * labelled 44px Terminal button at the end of the tab row; wider windows keep the header's icon toggle. On a phone the
 * play then selects a seat with a long name: the breadcrumb truncates within its share and the tab row, with the
 * terminal tab added, still ends before Terminal.
 */
export const TerminalToggle: Story = { args: { terminalRuntime: true }, play: restoringStoryUrl(async ({ canvasElement }) => {
  const { canvas, narrow, header } = await workspace(canvasElement)
  if (!narrow) await assertWide(canvasElement)
  if (narrow) await assertPhoneHeader(canvasElement)
  const terminal = narrow ? (await assertPhoneTabRow(canvasElement)).terminal : header.getByRole('button', { name: 'Toggle terminal drawer' })
  await expect(terminal).toHaveAttribute('aria-pressed', 'false')
  await expect(terminal).not.toHaveAttribute('aria-disabled')
  await expect(canvas.queryByRole('region', { name: 'Terminal drawer' })).toBeNull()
  const thread = canvasElement.querySelector<HTMLElement>('[data-testid="conversation-history-host"]')
  await expect(thread).not.toBeNull()
  await userEvent.click(terminal)
  await waitFor(() => expect(terminal).toHaveAttribute('aria-pressed', 'true'))
  // The terminal opens as a drawer under the thread, which keeps its node and stays on screen.
  await expect(await canvas.findByRole('region', { name: 'Terminal drawer' }, { timeout: 15000 })).toBeVisible()
  await expect(canvasElement.querySelector('[data-testid="conversation-history-host"]')).toBe(thread)
  await expect(thread!.getBoundingClientRect().height).toBeGreaterThan(0)
  if (narrow) { await assertPhoneHeader(canvasElement); await assertPhoneTabRow(canvasElement) }
  terminal.focus()
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(terminal).toHaveAttribute('aria-pressed', 'false'))
  await waitFor(() => expect(canvas.queryByRole('region', { name: 'Terminal drawer' })).toBeNull())
  await waitFor(() => expect(terminal).toHaveFocus())
  if (!narrow) return
  await assertPhoneHeader(canvasElement)
  await assertPhoneTabRow(canvasElement)
  // A long agent name at 390px: the title truncates inside the breadcrumb, which still keeps its share beside four controls.
  await userEvent.click(header.getByRole('button', { name: 'Expand sidebar' }))
  await userEvent.click(within(await canvas.findByRole('dialog', { name: 'Agents' })).getByRole('row', { name: new RegExp(`^${longName},`) }))
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: 'Agents' })).toBeNull())
  await waitFor(() => expect(within(canvasElement).getByTestId('thread-header')).toHaveTextContent(longName))
  const { breadcrumb } = await assertPhoneHeader(canvasElement, longName)
  const title = within(breadcrumb).getByText(longName)
  await expect(title.scrollWidth).toBeGreaterThan(title.clientWidth)
  await expect(title.getBoundingClientRect().right).toBeLessThanOrEqual(breadcrumb.getBoundingClientRect().right)
  // The previous seat keeps its own tab; opening this seat's terminal adds the terminal tab, and the tabs scroll before Terminal.
  const longTerminal = (await assertPhoneTabRow(canvasElement)).terminal
  await expect(longTerminal).not.toHaveAttribute('aria-disabled')
  await userEvent.click(longTerminal)
  await waitFor(() => expect(longTerminal).toHaveAttribute('aria-pressed', 'true'))
  await assertPhoneHeader(canvasElement, longName)
  await assertPhoneTabRow(canvasElement)
  await userEvent.click(longTerminal)
  await waitFor(() => expect(longTerminal).toHaveAttribute('aria-pressed', 'false'))
  await waitFor(() => expect(longTerminal).toHaveFocus())
  await assertPhoneTabRow(canvasElement)
}) }
export const TerminalToggleLight: Story = { ...TerminalToggle, args: { ...TerminalToggle.args, scheme: 'light' } }
