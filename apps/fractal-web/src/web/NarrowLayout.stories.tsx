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
/** Wide windows keep the docked sidebar, the full header cluster and the inline Details panel. */
const assertWide = async (canvasElement: HTMLElement) => {
  const { canvas, header } = await workspace(canvasElement)
  await expect(canvas.getByRole('complementary', { name: 'Agents' })).toBeVisible()
  await expect(canvas.queryByRole('dialog', { name: 'Agents' })).toBeNull()
  await expect(header.queryByRole('button', { name: 'Expand sidebar' })).toBeNull()
  await expect(header.getByRole('button', { name: 'Toggle right panel' })).toBeVisible()
  await expect(header.getByRole('button', { name: 'Actions for WorkbenchShell' })).toBeVisible()
  await expect(header.getByText('build-host-a')).toBeVisible()
  await expect(header.queryByRole('button', { name: 'Agent details' })).toBeNull()
}
const openWideDetails = async (canvasElement: HTMLElement) => {
  await assertWide(canvasElement)
  const { canvas, header } = await workspace(canvasElement)
  await userEvent.click(header.getByRole('button', { name: 'Toggle right panel' }))
  await expect(await canvas.findByRole('complementary', { name: 'Changes' })).toBeVisible()
  await expect(canvas.queryByRole('dialog', { name: 'Agent details' })).toBeNull()
}

/** The thread fills the window; the header keeps the chevron, host / agent breadcrumb, status and one Agent details button. */
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
  await expect(breadcrumb.getBoundingClientRect().right).toBeLessThanOrEqual(opener.getBoundingClientRect().left)
  for (const name of ['Actions for WorkbenchShell', 'Resources', 'Conversation menu', 'Toggle terminal drawer', 'Toggle right panel']) await expect(header.queryByRole('button', { name })).toBeNull()
} }
export const DrawersLight: Story = { ...Drawers, args: { scheme: 'light' } }

const openDetailsSheet = async (canvasElement: HTMLElement) => {
  const { canvas, header } = await workspace(canvasElement)
  const opener = header.getByRole('button', { name: 'Agent details' })
  await userEvent.click(opener)
  const sheet = await canvas.findByRole('dialog', { name: 'Agent details' })
  // Changes is a lazy chunk; on a cold dev server its first load takes longer than findBy's default second.
  await expect(await within(sheet).findByRole('complementary', { name: 'Changes' }, { timeout: 15000 })).toBeInTheDocument()
  await waitFor(() => expect(sheet.contains(document.activeElement)).toBe(true))
  return { canvas, sheet, opener }
}
/** The right sheet: the agent's actions in one row above Changes, which scrolls as one column. */
export const DrawersOpen: Story = { play: async ({ canvasElement }) => {
  const { narrow } = await workspace(canvasElement)
  if (!narrow) return openWideDetails(canvasElement)
  const { sheet, opener } = await openDetailsSheet(canvasElement)
  await expect(opener).toHaveAttribute('aria-expanded', 'true')
  const actions = within(sheet).getByRole('toolbar', { name: 'Agent actions' })
  for (const name of ['Actions for WorkbenchShell', 'Resources', 'Conversation menu', 'Terminal']) await expect(within(actions).getByRole('button', { name })).toBeInTheDocument()
  await expect(within(sheet).getByRole('button', { name: 'Close agent details' })).toBeVisible()
  await expect(canvasElement.querySelector('[data-testid="live-agent-workspace"] > aside[aria-label="Changes"]')).toBeNull()
  // The captured-change rows sit above the 'Captured change' header, never under it.
  const header = await within(sheet).findByRole('heading', { name: 'Captured change' })
  const rows = [...sheet.querySelectorAll('[data-captured-change]')]
  await expect(rows.length).toBeGreaterThan(0)
  const headerTop = header.getBoundingClientRect().top
  for (const row of rows) await expect(row.getBoundingClientRect().bottom).toBeLessThanOrEqual(headerTop)
  const changes = within(sheet).getByRole('complementary', { name: 'Changes' })
  await expect(changes.getBoundingClientRect().width).toBeLessThanOrEqual(sheet.getBoundingClientRect().width)
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

// AgentRowActions' compact-details query: phone widths and touch-only devices anchor Reported agent details to the row's menu trigger.
const compactDetailsQuery = '(max-width: 676px), (pointer: coarse)'
const detailsRow = /^MissionsUi,/
const tap = (target: Element) => userEvent.pointer([{ keys: '[TouchA>]', target }, { keys: '[/TouchA]', target }])
const nextFrame = () => {
  const { promise, resolve } = Promise.withResolvers<number>()
  requestAnimationFrame(resolve)
  return promise
}
type Press = 'touch' | 'mouse' | 'keyboard'
const press = async (target: HTMLElement, via: Press) => {
  if (via === 'touch') return tap(target)
  if (via === 'mouse') return userEvent.click(target)
  target.focus()
  await userEvent.keyboard('{Enter}')
}
/** Menu trigger → Reported agent details; resolves once the details dialog owns focus and keeps it across frames. */
const openReportedDetails = async (row: HTMLElement, via: Press) => {
  const trigger = within(row).getByRole('button', { name: /^Actions for / })
  const title = trigger.getAttribute('aria-label')!.replace(/^Actions for /, '')
  await press(trigger, via)
  const menu = await waitFor(() => within(document.body).getByRole('menu', { name: `Actions for ${title}` }))
  await press(within(menu).getByRole('menuitem', { name: 'Reported agent details' }), via)
  const dialog = await waitFor(() => within(document.body).getByRole('dialog', { name: `Reported details for ${title}` }))
  await waitFor(() => expect(within(document.body).queryByRole('menu')).toBeNull())
  await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true))
  // Nothing pulls focus back out between overlays: Escape must reach this dialog.
  await nextFrame()
  await nextFrame()
  await expect(dialog.contains(document.activeElement)).toBe(true)
  return { trigger, dialog, popover: dialog.parentElement! }
}
/** Popover and dialog fully inside the viewport, no horizontal overflow in the popover, its dialog or the page. */
const expectInsideViewport = (popover: HTMLElement, dialog: HTMLElement) => waitFor(() => {
  const width = document.documentElement.clientWidth
  for (const box of [popover.getBoundingClientRect(), dialog.getBoundingClientRect()]) {
    expect(box.width).toBeGreaterThan(0)
    expect(box.left).toBeGreaterThanOrEqual(-0.5)
    expect(box.top).toBeGreaterThanOrEqual(-0.5)
    expect(box.right).toBeLessThanOrEqual(width + 0.5)
    expect(box.bottom).toBeLessThanOrEqual(window.innerHeight + 0.5)
  }
  expect(popover.scrollWidth).toBeLessThanOrEqual(popover.clientWidth)
  expect(dialog.scrollWidth).toBeLessThanOrEqual(dialog.clientWidth)
  expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(width)
})
/** Bottom end on the menu trigger (flipped above when there is no room), at most the viewport less two 8px gutters. */
const expectCompact = (popover: HTMLElement, trigger: HTMLElement) => waitFor(() => {
  const box = popover.getBoundingClientRect()
  const anchor = trigger.getBoundingClientRect()
  const width = document.documentElement.clientWidth
  const placement = popover.getAttribute('data-placement')
  expect(placement).toMatch(/^(bottom|top)$/)
  if (placement === 'bottom') expect(box.top).toBeGreaterThanOrEqual(anchor.bottom - 0.5)
  else expect(box.bottom).toBeLessThanOrEqual(anchor.top + 0.5)
  expect(box.width).toBeLessThanOrEqual(width - 16 + 0.5)
  expect(box.left).toBeGreaterThanOrEqual(8 - 0.5)
  expect(box.right).toBeLessThanOrEqual(width - 8 + 0.5)
  // End-aligned with the trigger unless the start gutter pushes it inward.
  expect(Math.abs(box.right - Math.max(anchor.right, 8 + box.width))).toBeLessThanOrEqual(1)
})
/** Desktop keeps details to the right of the painted row, overlapping it vertically. */
const expectRightOfRow = (popover: HTMLElement, row: Element) => waitFor(() => {
  const box = popover.getBoundingClientRect()
  const anchor = row.getBoundingClientRect()
  expect(popover.getAttribute('data-placement')).toBe('right')
  expect(box.left).toBeGreaterThanOrEqual(anchor.right - 0.5)
  expect(box.top).toBeLessThan(anchor.bottom)
  expect(box.bottom).toBeGreaterThan(anchor.top)
})
const closeWithEscape = async (dialog: HTMLElement, trigger: HTMLElement) => {
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(dialog.isConnected).toBe(false))
  await waitFor(() => expect(trigger).toHaveFocus())
}

/**
 * The shared row menu's Reported agent details from the real roster. Below 900 px the row sits in the real phone
 * Agents drawer (320 px, less at narrower widths): details open bottom end on the row's menu trigger, fit the viewport
 * less 8 px gutters, own focus, and Escape closes only the details, returning focus to the menu trigger with the
 * drawer still open. Wider windows keep details to the right of the docked sidebar row.
 * The kit scenario checker reads only packages/fractal-ui/src stories, so this declaration has no exemption pin.
 */
export const AgentRowDetails: Story = {
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'The real workspace over the existing deterministic fixture roster places one fixed row in the phone Agents drawer and the docked sidebar to verify details placement, viewport bounds, focus ownership and Escape focus return.' } },
  play: async ({ canvasElement }) => {
  const { canvas, narrow, header } = await workspace(canvasElement)
  if (!narrow) {
    const row = within(canvas.getByRole('complementary', { name: 'Agents' })).getByRole('row', { name: detailsRow })
    const painted = row.querySelector('[data-milestone-row]')!
    const compact = window.matchMedia(compactDetailsQuery).matches
    for (const via of ['mouse', 'keyboard'] as const) {
      const { trigger, dialog, popover } = await openReportedDetails(row, via)
      await expectInsideViewport(popover, dialog)
      await (compact ? expectCompact(popover, trigger) : expectRightOfRow(popover, painted))
      await closeWithEscape(dialog, trigger)
    }
    await userEvent.unhover(row)
    return
  }
  await tap(header.getByRole('button', { name: 'Expand sidebar' }))
  const agents = await canvas.findByRole('dialog', { name: 'Agents' })
  await waitFor(() => expect(agents.contains(document.activeElement)).toBe(true))
  // The drawer keeps its strip of thread: 320 px, or the viewport less 40 px when narrower.
  await expect(Math.abs(agents.getBoundingClientRect().width - Math.min(320, document.documentElement.clientWidth - 40))).toBeLessThanOrEqual(1)
  const row = within(agents).getByRole('row', { name: detailsRow })
  for (const via of ['touch', 'keyboard'] as const) {
    const { trigger, dialog, popover } = await openReportedDetails(row, via)
    // Inside the drawer's modal DOM, so its focus containment and inert boundary include the details.
    await expect(agents.contains(dialog)).toBe(true)
    await expectInsideViewport(popover, dialog)
    await expectCompact(popover, trigger)
    await closeWithEscape(dialog, trigger)
    await expect(canvas.getByRole('dialog', { name: 'Agents' })).toBe(agents)
  }
} }
export const AgentRowDetailsLight: Story = { ...AgentRowDetails, args: { scheme: 'light' } }
