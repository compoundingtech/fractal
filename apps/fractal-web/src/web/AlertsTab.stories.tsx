/** The sidebar's Alerts tab over the fleet-mid-refactor attention slice, in a fixed 256px sidebar at 440 and 1440 in both themes. */
import * as Native from '@fractal/client/schema'
import { useScenarioSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, geometryVars as g, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { observed, unavailable, waiting, type Feed } from '../data/source.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { AlertsList } from './AlertsTab.tsx'

const decode = Native.decodeUnknownSync(Native.Attention)

type Args = { readonly scheme: 'dark' | 'light'; readonly feed: 'served' | 'waiting' | 'ungranted' | 'stale'; readonly attention?: string; readonly bounded?: boolean }

/** The bounded sidebar's height: shorter than the slice's four 48px rows, as a workspace column is shorter than a long feed. */
const BOUNDED_HEIGHT = 160

/** The feed the tab reads: the slice's cards as served, or the same cards in the source's waiting, refused and stale states. The play gate sets the scheme and the 440/1440 width. */
const AlertsTabStory = ({ scheme, feed: state, bounded = false }: Args) => {
  const slice = useScenarioSlice('attention')
  // Scenario readers shift committed instants to the current clock.
  const [now] = React.useState(Date.now)
  const [opened, setOpened] = React.useState('')
  const feed = React.useMemo((): Feed<readonly Native.Attention[]> => {
    if (slice.loading || state === 'waiting') return waiting
    if (state === 'ungranted') return unavailable({ reason: 'ungranted', detail: 'Read access to attention has not been granted.' })
    const value = slice.state.attention.map((card) => decode(card))
    return state === 'stale' ? { _tag: 'Observed', value, freshness: 'stale', readAt: now - 5 * 60_000 } : observed({ value })
  }, [slice, state, now])
  return (
    <NarrowLayoutContext.Provider value={window.innerWidth < 900}>
      <main data-scheme={scheme} {...stylex.props(styles.canvas, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
        <aside aria-label="Alerts sidebar" data-sidebar-host {...stylex.props(styles.sidebar, bounded && styles.bounded)}>
          {/* Bounded, the list sits where the workspace puts it: in a `minHeight: 0` column panel that clips its overflow. */}
          {bounded
            ? <div {...stylex.props(styles.panel)}><AlertsList feed={feed} now={now} onOpen={setOpened} /></div>
            : <AlertsList feed={feed} now={now} onOpen={setOpened} />}
        </aside>
        <output data-testid="alerts-opened">{opened}</output>
      </main>
    </NarrowLayoutContext.Provider>
  )
}

const rows = (canvasElement: HTMLElement) => [...canvasElement.querySelectorAll<HTMLElement>('[data-alert-id]')]
const rowButton = (row: HTMLElement) => row.querySelector<HTMLElement>('button[aria-expanded]')!

/** Opens a row's menu (right click, Shift+F10 or the ContextMenu key) and reads its items, then closes it. */
const menuItems = async (row: HTMLElement, open: 'right-click' | 'shift-f10' | 'context-menu-key'): Promise<readonly string[]> => {
  rowButton(row).focus()
  if (open === 'right-click') await userEvent.pointer({ keys: '[MouseRight]', target: rowButton(row) })
  else await userEvent.keyboard(open === 'shift-f10' ? '{Shift>}{F10}{/Shift}' : '{ContextMenu}')
  const menu = await within(document.body).findByRole('menu')
  const items = within(menu).getAllByRole('menuitem').map((item) => item.textContent ?? '')
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(within(document.body).queryByRole('menu')).toBeNull())
  return items
}

// attention.sidebar-tabs: builder's session asks one harness question (no requester, no mission); reviewer
// (holding the first mission), migrator and docs each need you through a card they requested.
const servedPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  const list = await canvas.findByRole('list', { name: 'Alerts' })
  const sidebar = canvas.getByRole('complementary', { name: 'Alerts sidebar' })
  const narrow = window.innerWidth < 900
  await expect(sidebar.getBoundingClientRect().width).toBe(256)
  const all = rows(canvasElement)
  await expect(all.map((row) => row.dataset.alertGlyph).sort()).toEqual(['?', '◆', '◆', '◆'])
  // SR-2 geometry: every row is 48px; the actions button takes a touch target only in the drawer layout.
  for (const row of all) {
    await expect(rowButton(row).getBoundingClientRect().height).toBe(48)
    await expect(within(row).getByRole('button', { name: /^Actions for / }).getBoundingClientRect().width).toBe(narrow ? 44 : 24)
  }
  // Facts that are not served are omitted, never painted as placeholders.
  await expect(list.textContent).not.toMatch(/—|undefined|null|NaN/)
  await expect(rowButton(all.find((row) => row.dataset.alertGlyph === '?')!)).toHaveAccessibleName(/, asks a question$/)

  // The same menu opens from ⋯, a right click, Shift+F10 or the ContextMenu key, and offers only what is served:
  // Answer always; Open conversation for a requesting agent; Open mission only on the card holding a mission.
  // Nothing serves snooze or close, so neither is offered.
  const question = all.find((row) => row.dataset.alertGlyph === '?')!
  await expect(await menuItems(question, 'right-click')).toEqual(['Answer'])
  const needsYou = all.filter((row) => row.dataset.alertGlyph === '◆')
  const menus = []
  for (const row of needsYou) menus.push(await menuItems(row, 'shift-f10'))
  await expect(menus.filter((items) => items.includes('Open mission'))).toEqual([['Answer', 'Open conversation', 'Open mission']])
  await expect(menus.filter((items) => !items.includes('Open mission'))).toEqual([['Answer', 'Open conversation'], ['Answer', 'Open conversation']])
  const holder = needsYou[menus.findIndex((items) => items.includes('Open mission'))]!
  rowButton(holder).focus()
  await userEvent.keyboard('{ContextMenu}')
  await userEvent.click(await within(document.body).findByRole('menuitem', { name: 'Open mission' }))
  await expect(canvas.getByTestId('alerts-opened').textContent).not.toBe('')
  await expect(canvas.getByTestId('alerts-opened').textContent).not.toMatch(/^agent\//)

  // Enter expands in place and moves focus into the answer; Escape collapses and returns focus to the row.
  const button = rowButton(holder)
  button.focus()
  await userEvent.keyboard('{Enter}')
  await expect(button).toHaveAttribute('aria-expanded', 'true')
  const region = within(holder).getByRole('region', { name: /^Answer / })
  await waitFor(() => expect(region.contains(document.activeElement)).toBe(true))
  await expect(within(region).getByTestId('conversation-alerts')).toBeVisible()
  await userEvent.keyboard('{Escape}')
  await expect(button).toHaveAttribute('aria-expanded', 'false')
  await expect(within(holder).queryByRole('region')).toBeNull()
  await expect(button).toHaveFocus()
}

/** Whether an element's top and bottom edges both lie inside the scroll region's visible box. */
const shownIn = (region: HTMLElement, element: HTMLElement) => {
  const port = region.getBoundingClientRect()
  const box = element.getBoundingClientRect()
  return box.top >= port.top - 1 && box.bottom <= port.bottom + 1
}

// The same four cards in a sidebar shorter than they are: the list scrolls inside the panel instead of growing it
// or being clipped, so the last row and every edge of its expanded answer are reachable.
const boundedPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  await canvas.findByRole('list', { name: 'Alerts' })
  const sidebar = canvas.getByRole('complementary', { name: 'Alerts sidebar' })
  const region = sidebar.querySelector<HTMLElement>('[data-alerts-scroll]')!
  await expect(sidebar.getBoundingClientRect().height).toBe(BOUNDED_HEIGHT)
  await expect(region.scrollHeight).toBeGreaterThan(region.clientHeight)
  await expect(region.getBoundingClientRect().bottom).toBeLessThanOrEqual(sidebar.getBoundingClientRect().bottom)
  const all = rows(canvasElement)
  await expect(all).toHaveLength(4)
  const last = all.at(-1)!
  const button = rowButton(last)
  await expect(shownIn(region, button)).toBe(false)

  // Focus scrolls the last row into the region; Enter expands its answer in place, still inside the region.
  button.focus()
  await waitFor(() => expect(shownIn(region, button)).toBe(true))
  await expect(region.scrollTop).toBeGreaterThan(0)
  await userEvent.keyboard('{Enter}')
  await expect(button).toHaveAttribute('aria-expanded', 'true')
  const answer = within(last).getByRole('region', { name: /^Answer / })
  await waitFor(() => expect(answer.contains(document.activeElement)).toBe(true))
  await expect(sidebar.getBoundingClientRect().height).toBe(BOUNDED_HEIGHT)
  await expect(region.scrollHeight).toBeGreaterThan(region.clientHeight)

  // Both edges of the expanded answer scroll into the region: its end at the bottom, then its start.
  region.scrollTop = region.scrollHeight
  await waitFor(() => expect(answer.getBoundingClientRect().bottom).toBeLessThanOrEqual(region.getBoundingClientRect().bottom + 1))
  await expect(answer.getBoundingClientRect().bottom).toBeGreaterThan(region.getBoundingClientRect().top)
  region.scrollTop += answer.getBoundingClientRect().top - region.getBoundingClientRect().top
  await waitFor(() => expect(answer.getBoundingClientRect().top).toBeGreaterThanOrEqual(region.getBoundingClientRect().top - 1))
  await expect(answer.getBoundingClientRect().top).toBeLessThan(region.getBoundingClientRect().bottom)
  await expect(within(answer).getByTestId('conversation-alerts')).toBeVisible()

  // Escape collapses back to the row, which stays in view.
  await userEvent.keyboard('{Escape}')
  await expect(button).toHaveFocus()
  await expect(shownIn(region, button)).toBe(true)
}

const meta = {
  title: 'Fractal/App/Sidebar/Alerts tab',
  component: AlertsTabStory,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'slices', slices: ['attention'], world: 'fleet-mid-refactor' },
    docs: { description: { component: 'Every open alert as an SR-2 row: glyph (? harness question, ◆ needs you, ! fault), title, wait age; line 2 shows only served facts (raised by, the step it holds, a self-resolve countdown when deadline and default answer are both served). Enter expands the alert\'s own answers in place and focuses them; Escape collapses. The ⋯ menu (also right click, Shift+F10, ContextMenu) offers Answer and, only when served, Open conversation and Open mission. The list scrolls itself inside a bounded sidebar. When an answered alert leaves the feed while focus was in its row, focus moves to the next row, else the previous one, else the empty state. Waiting, refused and stale feeds say so; an empty feed says nothing needs you.' } },
  },
  args: { scheme: 'dark', feed: 'served', attention: 'sidebar-tabs' },
  argTypes: { ...scenarioArgTypes(['attention']), scheme: { options: ['dark', 'light'], control: 'radio' }, feed: { options: ['served', 'waiting', 'ungranted', 'stale'], control: 'radio' } },
} satisfies Meta<Args>
export default meta

type Story = StoryObj<Args>
export const Served: Story = { tags: ['critical'], name: 'Served · rows, menu and expand', play: servedPlay }
/** A sidebar shorter than the feed: the list is its own bounded scroll region inside a `minHeight: 0` panel. */
export const BoundedScroll: Story = { tags: ['critical'], args: { bounded: true }, name: 'Bounded · scrolls in a short sidebar', play: boundedPlay }

export const Empty: Story = { args: { attention: 'none' }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(await canvas.findByText('Nothing needs you. Asks, approvals and harness questions show here.')).toBeVisible()
  await expect(canvas.queryByRole('list', { name: 'Alerts' })).toBeNull()
} }
export const Waiting: Story = { args: { feed: 'waiting' }, play: async ({ canvasElement }) => {
  await expect(await within(within(canvasElement).getByRole('complementary', { name: 'Alerts sidebar' })).findByRole('status')).toHaveTextContent('Loading alerts…')
  await expect(rows(canvasElement)).toHaveLength(0)
} }
export const Ungranted: Story = { args: { feed: 'ungranted', scheme: 'light' }, play: async ({ canvasElement }) => {
  await expect(await within(canvasElement).findByRole('alert')).toHaveTextContent('Read access to alerts has not been granted.')
  await expect(rows(canvasElement)).toHaveLength(0)
} }
/** Retained cards stay listed under an honest staleness note rather than vanishing. */
export const Stale: Story = { args: { feed: 'stale' }, play: async ({ canvasElement }) => {
  await expect(await within(within(canvasElement).getByRole('complementary', { name: 'Alerts sidebar' })).findByRole('status')).toHaveTextContent('Last known alerts, as of 5m ago; this list may be out of date.')
  await expect(rows(canvasElement)).toHaveLength(4)
} }

const styles = stylex.create({
  canvas: { display: 'flex', alignItems: 'flex-start', gap: 16, minHeight: '100vh', boxSizing: 'border-box', padding: 12, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  sidebar: { width: g.sidebarDefault, flexShrink: 0, boxSizing: 'border-box', backgroundColor: c.sidebar, border: `1px solid ${c.border}` },
  bounded: { display: 'flex', flexDirection: 'column', height: BOUNDED_HEIGHT },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0, overflow: 'hidden' },
})
