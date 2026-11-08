/** The live sidebar's three tabs against exact counts and fixed widths: the pills, and how the row steps down when space is tight. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { SidebarTabs, sidebarTabFits, type SidebarTab } from './SidebarTabs'

const tabs: readonly SidebarTab[] = [
  { id: 'agents', label: 'Agents', icon: 'person', needsYouCount: 3, count: 86 },
  { id: 'alerts', label: 'Alerts', icon: 'bell', questionCount: 12, needsYouCount: 2 },
  { id: 'missions', label: 'Missions', icon: 'checklist', count: 42 },
]
const names = ['Agents, 3 need you', 'Alerts, 12 open questions', 'Missions, 42']

/** Every badge width, selected and not: one digit, two, and the 99+ cap. */
const sizes: readonly SidebarTab[] = [
  { id: 'one', label: 'One', icon: 'person', needsYouCount: 1 },
  { id: 'twelve', label: 'Twelve', icon: 'bell', count: 12 },
  { id: 'many', label: 'Many', icon: 'checklist', count: 1234 },
]
const BadgeSizes = () => {
  const [selected, setSelected] = React.useState('one')
  return <main {...stylex.props(styles.host, styles.sidebar)}><SidebarTabs tabs={sizes} selectedId={selected} onSelectionChange={setSelected}><p>Panel</p></SidebarTabs></main>
}
/** The live sidebar at a 1280px window: a 240px sidebar with a few agents, alerts and a stalled mission. */
const live: readonly SidebarTab[] = [
  { id: 'agents', label: 'Agents', icon: 'person', count: 4 },
  { id: 'alerts', label: 'Alerts', icon: 'bell', needsYouCount: 2 },
  { id: 'missions', label: 'Missions', icon: 'checklist', status: { glyph: '!', label: '1 stalled or failed' } },
]
type Width = 'sidebar' | 'tight' | 'window1280' | 'iconsOnly' | 'phone'
const Surface = ({ width }: { readonly width: Width }) => {
  const [selected, setSelected] = React.useState('agents')
  return <NarrowLayoutContext.Provider value={width === 'phone'}>
    <main {...stylex.props(styles.host, styles[width])}><SidebarTabs tabs={width === 'window1280' || width === 'phone' ? live : tabs} selectedId={selected} onSelectionChange={setSelected}><p>Panel</p></SidebarTabs></main>
  </NarrowLayoutContext.Provider>
}
const painted = (tablist: HTMLElement) => within(tablist).getAllByRole('tab').map(tab => {
  const pill = tab.querySelector('[data-sidebar-tab-marker]')
  return `${tab.textContent}${pill === null ? '' : `·${pill.getAttribute('data-sidebar-tab-count') ?? ''}`}`
})
/** Never cramped: nothing overflows or clips, the row keeps its side padding, and adjacent tabs keep the gap plus their padding. */
const assertWhole = async (tablist: HTMLElement) => {
  await expect(tablist.scrollWidth).toBeLessThanOrEqual(tablist.clientWidth)
  for (const label of tablist.querySelectorAll<HTMLElement>('[data-sidebar-tab-label]')) await expect(label.scrollWidth).toBeLessThanOrEqual(label.clientWidth + 1)
  const bar = tablist.getBoundingClientRect(), tabs = within(tablist).getAllByRole('tab').map(tab => tab.getBoundingClientRect())
  // Within the half pixel of fit slack: rects are fractional, and a row that fits by rounding is accepted.
  await expect(tabs[0]!.left - bar.left).toBeGreaterThanOrEqual(8 - fitSlack)
  await expect(bar.right - tabs.at(-1)!.right).toBeGreaterThanOrEqual(8 - fitSlack)
  for (const [index, tab] of tabs.slice(1).entries()) await expect(tab.left - tabs[index]!.right).toBeGreaterThanOrEqual(4 - fitSlack)
  // Painted content (icon, label, badge) of neighbouring tabs: the gap plus both tabs' inline padding.
  const faces = [...tablist.querySelectorAll('[role="tab"] > span')].map(face => face.getBoundingClientRect())
  for (const [index, face] of faces.slice(1).entries()) await expect(face.left - faces[index]!.right).toBeGreaterThanOrEqual(12 - fitSlack)
}
const fitSlack = 0.5
const fitOf = (canvasElement: HTMLElement) => canvasElement.querySelector('[data-sidebar-tabs]')!

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Sidebar/Tabs fit',
  component: Surface,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'Asserts tab pills against exact counts and fixed widths, independent of world data.' } },
} satisfies Meta<typeof Surface>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['At256:primitive', 'BadgeShapes:primitive', 'CountsBeforeIcons:primitive', 'CountsDropped:primitive', 'IconsOnly:primitive', 'NeverCramped:primitive', 'Phone:primitive']

export const At256: Story = { tags: ['critical'], name: 'Three tabs · counts fit 256px once icons give way', args: { width: 'sidebar' }, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-fit', 'labels'))
  await expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-counts', 'shown')
  await expect(painted(tablist)).toEqual(['Agents·3', 'Alerts·12', 'Missions·42'])
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.getAttribute('aria-label'))).toEqual(names)
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.querySelector('[data-sidebar-tab-icon] svg') !== null)).toEqual([false, false, false])
  // Needing the person is filled, a neutral count is not: colour is not the only cue.
  const fill = (kind: string) => getComputedStyle(tablist.querySelector(`[data-sidebar-tab-marker="${kind}"]`)!).backgroundColor
  await expect(fill('needs-you')).not.toBe(fill('count'))
  await assertWhole(tablist)
} }
export const BadgeShapes: Story = { tags: ['critical'], name: 'Badges · 1, 12 and 99+', args: { width: 'sidebar' }, render: () => <BadgeSizes />, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-counts', 'shown'))
  await expect(painted(tablist)).toEqual(['One·1', 'Twelve·12', 'Many·99+'])
  const [one, twelve, many] = [...tablist.querySelectorAll('[data-sidebar-tab-marker]')].map(pill => pill.getBoundingClientRect())
  // One height, never stretched by the row; one digit is a true circle and more digits only grow sideways.
  for (const badge of [one!, twelve!, many!]) await expect(badge.height).toBe(16)
  await expect(one!.width).toBe(16)
  await expect(twelve!.width).toBeGreaterThan(one!.width)
  await expect(many!.width).toBeGreaterThan(twelve!.width)
  // The exact count stays spoken past the painted cap.
  await expect(within(tablist).getAllByRole('tab')[2]).toHaveAttribute('aria-label', 'Many, 1234')
} }
export const CountsDropped: Story = { tags: ['critical'], name: 'Three tabs · counts become compact badges when even the iconless row overflows', args: { width: 'tight' }, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-fit', 'compact'))
  await expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-counts', 'dropped')
  await expect(painted(tablist)).toEqual(['Agents·', 'Alerts·', 'Missions'])
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.getAttribute('aria-label'))).toEqual(names)
  await assertWhole(tablist)
} }
// Numbers carry information and tab icons only repeat the label, so icons give way first and counts drop only when even
// the iconless row cannot hold them.
export const CountsBeforeIcons: Story = { tags: ['critical'], name: 'Three tabs · icons give way before counts at 240px', args: { width: 'window1280' }, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-fit', 'labels'))
  await expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-counts', 'shown')
  await expect(painted(tablist)).toEqual(['Agents·4', 'Alerts·2', 'Missions·'])
  await assertWhole(tablist)
} }

// Narrowest: each tab is its icon and count; the label moves to a tooltip and stays first in the tab's name.
export const IconsOnly: Story = { tags: ['critical'], name: 'Three tabs · icon and count at 180px, label in the tooltip', args: { width: 'iconsOnly' }, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-fit', 'icons'))
  await expect(painted(tablist)).toEqual(['·3', '·12', '·42'])
  await expect(tablist.querySelectorAll('[data-sidebar-tab-label]')).toHaveLength(0)
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.querySelector('[data-sidebar-tab-icon] svg') !== null)).toEqual([true, true, true])
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.getAttribute('aria-label'))).toEqual(names)
  await assertWhole(tablist)
  // A pointer press puts React Aria in pointer modality, where hover opens a tooltip after its delay.
  await userEvent.click(within(canvasElement).getByText('Panel'))
  await userEvent.hover(within(tablist).getAllByRole('tab')[1]!)
  await expect(await within(canvasElement.ownerDocument.body).findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent('Alerts')
  await userEvent.unhover(within(tablist).getAllByRole('tab')[1]!)
} }
/** Every width from a wide sidebar down to 120px: the row only ever steps down, and no step is cramped. */
export const NeverCramped: Story = { tags: ['critical'], name: 'Three tabs · roomy at every width from 340 to 120px', args: { width: 'sidebar' }, play: async ({ canvasElement }) => {
  const host = canvasElement.querySelector<HTMLElement>('main')!
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  const order: readonly string[] = sidebarTabFits.map(step => step.fit)
  const seen: string[] = []
  for (let width = 340; width >= 120; width -= 4) {
    host.style.width = `${width}px`
    // The observer refits on the next frame.
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))
    const fit = fitOf(canvasElement).getAttribute('data-sidebar-tab-fit')!
    if (seen.at(-1) !== fit) seen.push(fit)
    await assertWhole(tablist)
  }
  await expect(seen).toEqual(order)
  host.style.width = ''
} }
// Phone drawer: 44px touch tabs, the same padding and gap.
export const Phone: Story = { tags: ['critical'], name: 'Three tabs · phone drawer, touch targets with room', args: { width: 'phone' }, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(fitOf(canvasElement)).toHaveAttribute('data-sidebar-tab-fit', 'full'))
  for (const tab of within(tablist).getAllByRole('tab')) {
    await expect(tab.getBoundingClientRect().height).toBe(44)
    await expect(tab.getBoundingClientRect().width).toBeGreaterThanOrEqual(44)
  }
  await assertWhole(tablist)
} }

const styles = stylex.create({
  host: { backgroundColor: surface.canvas, color: ink.fg, display: 'flex', flexDirection: 'column', height: '120px' },
  sidebar: { width: '256px' },
  tight: { width: '220px' },
  window1280: { width: '240px' },
  iconsOnly: { width: '180px' },
  // The phone drawer's sidebar width.
  phone: { width: '319px', height: '160px' },
})
