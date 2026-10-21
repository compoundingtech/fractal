import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, waiting } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveAgentWorkspace, narrowLayoutQuery } from './LiveAgentWorkspace.tsx'

// The real workspace over the deterministic fixture world, with resources supported as on the live gateway.
const source = {
  ...fixtureSource({ world: fixtureProjections }),
  resources: { byAgent: () => Atom.make(observed({ value: { items: [], nextCursor: null } })), byId: () => Atom.make(waiting), subjects: Atom.make([]), loadMore: () => undefined, refresh: () => undefined, resolveFile: () => undefined },
}
const storyAgent = 'agent/build-host-a/workbench-shell'

const NarrowLayoutStory = (_: { readonly scheme: 'dark' | 'light' }) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  return <DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>
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
/** The docked inspector opens on Details; its separator names the visible view and follows the tab selection. */
const openWideDetails = async (canvasElement: HTMLElement) => {
  await assertWide(canvasElement)
  const { canvas, header } = await workspace(canvasElement)
  await userEvent.click(header.getByRole('button', { name: 'Toggle right panel' }))
  const inspector = await canvas.findByRole('complementary', { name: 'Agent inspector' }, { timeout: 15000 })
  await expect(within(inspector).getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByRole('separator', { name: 'Resize Details' })).toBeVisible()
  await userEvent.click(within(inspector).getByRole('tab', { name: 'Changes' }))
  await expect(await within(inspector).findByRole('complementary', { name: 'Changes' }, { timeout: 15000 })).toBeVisible()
  await expect(canvas.getByRole('separator', { name: 'Resize Changes' })).toBeVisible()
  await expect(canvas.queryByRole('separator', { name: 'Resize Details' })).toBeNull()
  await expect(canvas.queryByRole('dialog', { name: 'Agent details' })).toBeNull()
  // A remounted inspector starts on Details again, and so does its separator.
  await userEvent.click(within(inspector).getByRole('button', { name: 'Close agent inspector' }))
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: 'Agent inspector' })).toBeNull())
  await userEvent.click(header.getByRole('button', { name: 'Toggle right panel' }))
  const reopened = await canvas.findByRole('complementary', { name: 'Agent inspector' }, { timeout: 15000 })
  await expect(within(reopened).getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByRole('separator', { name: 'Resize Details' })).toBeVisible()
}

/** The phone header's controls in order: the conversation actions stay beside a labelled Terminal toggle and Agent details. */
const phoneHeaderNames = ['Expand sidebar', 'Actions for WorkbenchShell', 'Conversation menu', 'Terminal', 'Agent details']
/** Every phone header control is a visible 44×44px target inside the window, clear of the breadcrumb and of each other; the header never scrolls sideways. */
const assertPhoneHeader = async (canvasElement: HTMLElement) => {
  const node = within(canvasElement).getByTestId('thread-header')
  const header = within(node)
  const buttons = header.getAllByRole('button')
  await expect(buttons.map(button => button.getAttribute('aria-label'))).toEqual(phoneHeaderNames)
  for (const name of ['Resources', 'View KDL', 'Toggle terminal drawer', 'Toggle right panel']) await expect(header.queryByRole('button', { name })).toBeNull()
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
  await expect(crumb.width).toBeGreaterThan(0)
  await expect(crumb.left).toBeGreaterThanOrEqual(rects[0]!.right)
  await expect(crumb.right).toBeLessThanOrEqual(rects[1]!.left)
  // The Terminal toggle's visible label carries its meaning; touch has no hover tooltip.
  const terminal = header.getByRole('button', { name: 'Terminal' })
  await expect(terminal).toHaveTextContent('Terminal')
  await expect(terminal).not.toHaveAttribute('title')
  return { header, terminal }
}

/** The thread fills the window; the header keeps the chevron, host / agent breadcrumb, status, the conversation actions, Terminal and one Agent details button. */
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
  const { terminal } = await assertPhoneHeader(canvasElement)
  await expect(terminal).toHaveAttribute('aria-pressed', 'false')
  await expect(canvas.queryByRole('region', { name: 'Terminal drawer' })).toBeNull()
} }
export const DrawersLight: Story = { ...Drawers, args: { scheme: 'light' } }

const openDetailsSheet = async (canvasElement: HTMLElement) => {
  const { canvas, header } = await workspace(canvasElement)
  const opener = header.getByRole('button', { name: 'Agent details' })
  await userEvent.click(opener)
  const sheet = await canvas.findByRole('dialog', { name: 'Agent details' })
  // The inspector is a lazy chunk; on a cold dev server its first load takes longer than findBy's default second.
  const inspector = await within(sheet).findByRole('complementary', { name: 'Agent inspector' }, { timeout: 15000 })
  await waitFor(() => expect(sheet.contains(document.activeElement)).toBe(true))
  return { canvas, sheet, inspector, opener }
}
/** The right sheet holds the agent inspector: Details first, Changes one tab away; the conversation actions stay in the header. */
export const DrawersOpen: Story = { play: async ({ canvasElement }) => {
  const { narrow } = await workspace(canvasElement)
  if (!narrow) return openWideDetails(canvasElement)
  const { canvas, sheet, inspector, opener } = await openDetailsSheet(canvasElement)
  await expect(opener).toHaveAttribute('aria-expanded', 'true')
  // The sheet header's X is its only close; the header's actions are not repeated in the sheet.
  await expect(within(sheet).getByRole('button', { name: 'Close agent details' })).toBeVisible()
  await expect(within(sheet).queryByRole('button', { name: 'Close agent inspector' })).toBeNull()
  await expect(within(sheet).queryByRole('toolbar', { name: 'Agent actions' })).toBeNull()
  for (const name of ['Actions for WorkbenchShell', 'Conversation menu', 'Terminal']) await expect(within(sheet).queryByRole('button', { name })).toBeNull()
  await expect(canvas.queryByRole('separator', { name: /^Resize / })).toBeNull()
  const tabs = within(within(inspector).getByRole('tablist', { name: 'Agent inspector views' })).getAllByRole('tab')
  await expect(tabs.map(tab => tab.textContent?.trim())).toEqual(['Changes', 'Details'])
  for (const tab of tabs) {
    const rect = tab.getBoundingClientRect()
    await expect(rect.width).toBeGreaterThanOrEqual(44)
    await expect(rect.height).toBeGreaterThanOrEqual(44)
  }
  const details = within(inspector).getByRole('tab', { name: 'Details' })
  await expect(details).toHaveAttribute('aria-selected', 'true')
  // Changes scrolls as one column inside the sheet; the captured-change rows sit above the 'Captured change' header, never under it.
  await userEvent.click(within(inspector).getByRole('tab', { name: 'Changes' }))
  await expect(within(inspector).getByRole('tab', { name: 'Changes' })).toHaveAttribute('aria-selected', 'true')
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
  await expect(within(reopened.inspector).getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true')
} }
/** Keyboard path through both sheets: each opens with focus inside, Escape or the backdrop closes it, and focus returns to its opener. */
export const DrawersFlow: Story = { play: async ({ canvasElement }) => {
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
} }
/** The header's terminal toggle, off then on then off: on a phone it is the labelled 44px Terminal button; wider windows keep the icon toggle. */
export const TerminalToggle: Story = { play: async ({ canvasElement }) => {
  const { canvas, narrow, header } = await workspace(canvasElement)
  if (!narrow) await assertWide(canvasElement)
  const terminal = narrow ? (await assertPhoneHeader(canvasElement)).terminal : header.getByRole('button', { name: 'Toggle terminal drawer' })
  await expect(terminal).toHaveAttribute('aria-pressed', 'false')
  await expect(terminal).not.toHaveAttribute('aria-disabled')
  await expect(canvas.queryByRole('region', { name: 'Terminal drawer' })).toBeNull()
  await userEvent.click(terminal)
  await waitFor(() => expect(terminal).toHaveAttribute('aria-pressed', 'true'))
  // The terminal opens as a drawer under the thread, which stays on screen.
  await expect(await canvas.findByRole('region', { name: 'Terminal drawer' }, { timeout: 15000 })).toBeVisible()
  await expect(canvas.getByText('Let me find where groups close.')).toBeVisible()
  if (narrow) await assertPhoneHeader(canvasElement)
  terminal.focus()
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(terminal).toHaveAttribute('aria-pressed', 'false'))
  await waitFor(() => expect(canvas.queryByRole('region', { name: 'Terminal drawer' })).toBeNull())
  if (narrow) await assertPhoneHeader(canvasElement)
} }
export const TerminalToggleLight: Story = { ...TerminalToggle, args: { scheme: 'light' } }
