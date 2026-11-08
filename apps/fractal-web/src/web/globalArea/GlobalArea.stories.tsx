import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { DataSourceProvider } from '../../data/react.tsx'
import type { FeedSyncObservation } from '../../data/feedSync.ts'
import { AlertAnswersContext, createAlertAnswers } from '../../conversation/alerts/alertAnswers.ts'
import { createAppStoryFixture } from '../../fixtures/appStoryStates.ts'
import { fixtureProjections } from '../../fixtures/projections.ts'
import { writeCellsLikeAnotherWindow } from '../../state/cell/testing.ts'
import type { GlassGateway } from '../../workspace/glassSync.ts'
import { tooltipFor } from '../../../../../packages/fractal-ui/src/story-play.ts'
import { LiveAgentWorkspace } from '../LiveAgentWorkspace.tsx'

type Connection = 'live' | 'reconnecting' | 'offline'

interface GlobalAreaArgs {
  readonly scheme: 'dark' | 'light'
  readonly connection: Connection
  readonly collapsed: boolean
  readonly toast: boolean
  readonly unsynced: boolean
}

// The person's glasses are out of reach for this page's first look, so the layout is the page's own.
const unreachableGlasses: GlassGateway = {
  discover: () => Promise.reject(new TypeError('The glasses could not be reached')),
  collectionStream: () => { throw new TypeError('The glasses could not be reached') },
  putGlass: () => Promise.reject(new TypeError('The glasses could not be reached')),
  deleteGlass: () => Promise.reject(new TypeError('The glasses could not be reached')),
}

/** The production workspace over the fixture world, with the gateway, the glasses and the alert answers each set for one global-area state. */
const GlobalAreaSpecimen = ({ connection, toast, unsynced }: Pick<GlobalAreaArgs, 'connection' | 'toast' | 'unsynced'>) => {
  const [fixture] = React.useState(() => createAppStoryFixture(connection === 'offline' ? 'offline' : 'populated'))
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState(() => connection !== 'reconnecting' ? fixture.source : {
    ...fixture.source,
    reconnect: () => undefined,
    // Lost five seconds ago, past the line's grace delay, with the next try four seconds out.
    sync: { ...fixture.source.sync!, gateway: Atom.make<FeedSyncObservation>({ observedAt: fixtureProjections.now - 5000, status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 3, nextAt: fixtureProjections.now + 4000, issue: 'socket closed' } } }) },
  })
  const [answers] = React.useState(() => {
    const store = createAlertAnswers()
    const alert = fixtureProjections.attention[0]
    if (toast && alert !== undefined) store.send(alert, 'Approve', async () => ({ _tag: 'Refused', detail: 'Fractal no longer offers this answer here.' }))
    return store
  })
  return <DataSourceProvider source={source} registry={registry}>
    <AlertAnswersContext.Provider value={answers}>
      <LiveAgentWorkspace initialRef={fixture.agentRef} persistGeometry={false} {...(unsynced ? { glasses: unreachableGlasses } : {})} />
    </AlertAnswersContext.Provider>
  </DataSourceProvider>
}

/**
 * The left sidebar as the app's one global area: the header (toggle, space, palette, Settings), "Layout not syncing"
 * under it, the footer (connection, Usage), a 48px rail when collapsed, and toasts at the bottom-left. A phone's drawer
 * is the same area, and its opener carries a dot for whatever hides behind it.
 */
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/GlobalArea',
  args: { scheme: 'dark', connection: 'live', collapsed: false, toast: false, unsynced: false },
  argTypes: {
    scheme: { control: 'radio', options: ['dark', 'light'] },
    connection: { control: 'radio', options: ['live', 'reconnecting', 'offline'] },
  },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The production LiveAgentWorkspace over fixtureSource feeds, with the gateway observation, the glasses gateway and the alert answers set per story, pins each global-area state without a gateway.' } },
  beforeEach: ({ args }) => {
    writeCellsLikeAnotherWindow({
      'round2.scheme': { value: args.scheme },
      'panels.leftSidebar': { value: { open: !args.collapsed, ratio: 256 / 1440 } },
    })
  },
  render: args => <GlobalAreaSpecimen key={`${args.connection}-${args.toast}-${args.unsynced}`} connection={args.connection} toast={args.toast} unsynced={args.unsynced} />,
} satisfies Meta<GlobalAreaArgs>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AlertToast:contract-fixture',
  'ConnectionLost:contract-fixture',
  'Docked:contract-fixture',
  'LayoutNotSyncing:contract-fixture',
  'Offline:contract-fixture',
  'Rail:contract-fixture',
  'RailConnectionLost:contract-fixture',
]
type Story = StoryObj<typeof meta>

const narrowWidth = 900
const isNarrow = () => window.innerWidth < narrowWidth
const sidebarOf = (root: ParentNode) => root.querySelector<HTMLElement>('aside[aria-label="Agents"]')
/** The global area: the dock or rail on a desktop, the drawer's sidebar on a phone (opened if needed). */
const globalArea = async (canvasElement: HTMLElement): Promise<HTMLElement> => {
  const canvas = within(canvasElement)
  await waitFor(() => expect(canvas.getAllByTestId('thread-header').length).toBeGreaterThan(0), { timeout: 60000 })
  if (!isNarrow()) return waitFor(() => { const sidebar = sidebarOf(canvasElement); expect(sidebar).not.toBeNull(); return sidebar! })
  const dialog = canvas.queryByRole('dialog', { name: 'Agents' }) ?? (await userEvent.click(canvas.getByRole('button', { name: 'Expand sidebar' })), await canvas.findByRole('dialog', { name: 'Agents' }))
  return sidebarOf(dialog)!
}
const labels = (scope: ParentNode, selector = 'button') => [...scope.querySelectorAll<HTMLElement>(selector)].map(node => node.getAttribute('aria-label') ?? node.textContent)
/** A keyboard focus shows the one shared ring: a solid outline, not a fill or a native ring. */
const expectRing = async (control: HTMLElement) => {
  await waitFor(() => expect(control).toHaveFocus())
  await expect(getComputedStyle(control).outlineStyle).toBe('solid')
}

/** The header holds the toggle, the space, the palette and Settings; the footer holds Usage and stays quiet while live. */
export const Docked: Story = {
  play: async ({ canvasElement }) => {
    const sidebar = await globalArea(canvasElement)
    const header = sidebar.querySelector<HTMLElement>(':scope > header')!
    await expect(labels(header)).toEqual([isNarrow() ? 'Close agents' : 'Collapse agents', 'Space: Main', 'Command palette', 'Settings'])
    const footer = sidebar.querySelector<HTMLElement>(':scope > footer')!
    await expect(footer.querySelector('[data-usage-section]')).not.toBeNull()
    await expect(footer.querySelector('[data-connection-line]')).toBeNull()
    await expect(footer.getBoundingClientRect().bottom).toBeGreaterThan(sidebar.getBoundingClientRect().bottom - 2)
    // Keyboard reaches the header's controls in order, each with the shared ring.
    const [first, space] = [...header.querySelectorAll<HTMLElement>('button')]
    first!.focus()
    await userEvent.keyboard('{Tab}')
    await expectRing(space!)
    canvasElement.dataset.proof = 'passed'
  },
}

/** Collapsed, the dock is a 48px rail of the same controls; a tab opens the sidebar on itself. A phone has no rail. */
export const Rail: Story = {
  args: { collapsed: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getAllByTestId('thread-header').length).toBeGreaterThan(0), { timeout: 60000 })
    if (isNarrow()) {
      await expect(canvasElement.querySelector('[data-sidebar-presentation="rail"]')).toBeNull()
      await expect(canvas.getByRole('button', { name: 'Expand sidebar' })).toBeVisible()
      canvasElement.dataset.proof = 'passed'
      return
    }
    const rail = await waitFor(() => { const node = canvasElement.querySelector<HTMLElement>('aside[data-sidebar-presentation="rail"]'); expect(node).not.toBeNull(); return node! })
    await expect(Math.round(rail.getBoundingClientRect().width)).toBe(48)
    await expect(labels(rail).slice(0, 3)).toEqual(['Expand sidebar', 'Space: Main', 'Command palette'])
    await expect(labels(rail).slice(-2)).toEqual(['Usage', 'Settings'])
    for (const header of canvas.getAllByTestId('thread-header')) await expect(within(header).queryByRole('button', { name: 'Expand sidebar' })).toBeNull()
    const alerts = within(rail).getByRole('button', { name: /^Alerts/ })
    // The tooltip carries the detail beside the control, never over the rail's other controls.
    // As a pointer does, cross the rail before the control.
    await userEvent.hover(rail)
    await userEvent.hover(alerts)
    const tip = await tooltipFor(alerts, alerts.getAttribute('aria-label')!)
    await expect(tip.getBoundingClientRect().left).toBeGreaterThanOrEqual(alerts.getBoundingClientRect().right)
    await userEvent.unhover(alerts)
    // Keyboard: each rail control takes the shared ring.
    within(rail).getByRole('button', { name: 'Command palette' }).focus()
    await userEvent.keyboard('{Tab}')
    await expectRing(within(rail).getByRole('button', { name: /^Agents/ }))
    await userEvent.click(alerts)
    const dock = await waitFor(() => { const node = canvasElement.querySelector<HTMLElement>('aside[data-sidebar-presentation="dock"]'); expect(node).not.toBeNull(); return node! })
    await expect(within(dock).getByRole('tab', { selected: true })).toHaveAttribute('data-sidebar-tab', 'alerts')
    await waitFor(() => expect(dock.contains(document.activeElement)).toBe(true))
    canvasElement.dataset.proof = 'passed'
  },
}

/** Reconnecting is the footer's to say, with the next try and Reconnect; no pane carries a band. On a phone the opener says it. */
export const ConnectionLost: Story = {
  args: { connection: 'reconnecting' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getAllByTestId('thread-header').length).toBeGreaterThan(0), { timeout: 60000 })
    if (isNarrow()) {
      const opener = canvas.getByRole('button', { name: 'Expand sidebar' })
      await waitFor(() => expect(opener).toHaveAccessibleDescription(/^Reconnecting/))
      await expect(opener.querySelector('[data-chrome-badge] [data-status-dot]')).not.toBeNull()
    }
    await expect(canvasElement.querySelector('[data-connection-state]')).toBeNull()
    const sidebar = await globalArea(canvasElement)
    const line = await waitFor(() => { const node = sidebar.querySelector<HTMLElement>(':scope > footer [data-connection-line="reconnecting"]'); expect(node).not.toBeNull(); return node! })
    await expect(line).toHaveTextContent(/Reconnecting.*Next try in 4s · attempt 3/)
    await expect(within(line).getByRole('button', { name: 'Reconnect' })).toBeVisible()
    canvasElement.dataset.proof = 'passed'
  },
}

/** Collapsed, a lost connection stays on screen as a rail control whose tooltip says the detail and whose press opens the footer. */
export const RailConnectionLost: Story = {
  args: { connection: 'reconnecting', collapsed: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getAllByTestId('thread-header').length).toBeGreaterThan(0), { timeout: 60000 })
    if (isNarrow()) { canvasElement.dataset.proof = 'passed'; return }
    const rail = canvasElement.querySelector<HTMLElement>('aside[data-sidebar-presentation="rail"]')!
    const control = within(rail).getByRole('button', { name: 'Connection: Reconnecting' })
    await userEvent.hover(rail)
    await userEvent.hover(control)
    await expect(await tooltipFor(control, 'Reconnecting · Next try in 4s · attempt 3')).toBeVisible()
    await userEvent.unhover(control)
    await userEvent.click(control)
    await waitFor(() => expect(canvasElement.querySelector('aside[data-sidebar-presentation="dock"] footer [data-connection-line="reconnecting"]')).not.toBeNull())
    canvasElement.dataset.proof = 'passed'
  },
}

/** Offline is gone, not recovering: the footer says it and the focused pane gets a band, on a phone too. */
export const Offline: Story = {
  args: { connection: 'offline' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getAllByTestId('thread-header').length).toBeGreaterThan(0), { timeout: 60000 })
    const band = await waitFor(() => { const node = canvasElement.querySelector<HTMLElement>('[data-pane][data-focused] [data-connection-state="offline"]'); expect(node).not.toBeNull(); return node! })
    await expect(band).toBeVisible()
    if (isNarrow()) await expect(canvas.getByRole('button', { name: 'Expand sidebar' }).querySelector('[data-status-dot="danger"]')).not.toBeNull()
    const sidebar = await globalArea(canvasElement)
    await expect(sidebar.querySelector(':scope > footer [data-connection-line="offline"]')).not.toBeNull()
    canvasElement.dataset.proof = 'passed'
  },
}

/** "Layout not syncing" sits under the header, beside the space switcher it is about, never in a pane. */
export const LayoutNotSyncing: Story = {
  args: { unsynced: true },
  play: async ({ canvasElement }) => {
    const sidebar = await globalArea(canvasElement)
    const note = await waitFor(() => { const node = sidebar.querySelector<HTMLElement>('[data-layout-sync]'); expect(node).not.toBeNull(); return node! })
    await expect(note.previousElementSibling?.matches('header')).toBe(true)
    await expect(canvasElement.querySelector('[data-pane] [data-layout-sync]')).toBeNull()
    await expect(within(note).getByRole('button', { name: 'Retry' })).toBeVisible()
    canvasElement.dataset.proof = 'passed'
  },
}

/** A refused answer is said once as a toast at the bottom-left, beside the sidebar and over the panes, without taking focus. */
export const AlertToast: Story = {
  args: { toast: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getAllByTestId('thread-header').length).toBeGreaterThan(0), { timeout: 60000 })
    const toast = await canvas.findByTestId('alert-flash', {}, { timeout: 10000 })
    await expect(toast).toHaveAttribute('role', 'alert')
    await expect(toast.closest('[role="region"][aria-label="Notifications"]')).not.toBeNull()
    await expect(toast.closest('[data-pane], aside')).toBeNull()
    await expect(toast.contains(document.activeElement)).toBe(false)
    const box = toast.getBoundingClientRect()
    const sidebar = sidebarOf(canvasElement)
    await expect(box.left).toBeGreaterThanOrEqual((sidebar?.getBoundingClientRect().right ?? 0))
    await expect(box.bottom).toBeGreaterThan(window.innerHeight - 40)
    await expect(box.right).toBeLessThanOrEqual(window.innerWidth)
    canvasElement.dataset.proof = 'passed'
  },
}
