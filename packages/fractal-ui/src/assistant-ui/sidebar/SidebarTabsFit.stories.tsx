/** The live sidebar's three tabs against exact counts and fixed widths: the pills, and what drops when space is tight. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { expect, waitFor, within } from 'storybook/test'
import { surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import { SidebarTabs, type SidebarTab } from './SidebarTabs'

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
const Surface = ({ width }: { readonly width: 'sidebar' | 'tight' }) => {
  const [selected, setSelected] = React.useState('agents')
  return <main {...stylex.props(styles.host, width === 'sidebar' ? styles.sidebar : styles.tight)}><SidebarTabs tabs={tabs} selectedId={selected} onSelectionChange={setSelected}><p>Panel</p></SidebarTabs></main>
}
const painted = (tablist: HTMLElement) => within(tablist).getAllByRole('tab').map(tab => {
  const pill = tab.querySelector('[data-sidebar-tab-marker]')
  return `${tab.textContent}${pill === null ? '' : `·${pill.getAttribute('data-sidebar-tab-count') ?? ''}`}`
})
const assertWhole = async (tablist: HTMLElement) => {
  await expect(tablist.scrollWidth).toBeLessThanOrEqual(tablist.clientWidth)
  for (const label of tablist.querySelectorAll<HTMLElement>('[data-sidebar-tab-label]')) await expect(label.scrollWidth).toBeLessThanOrEqual(label.clientWidth + 1)
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Sidebar/Tabs fit',
  component: Surface,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'Asserts tab pills against exact counts and fixed widths, independent of world data.' } },
} satisfies Meta<typeof Surface>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['At256:primitive', 'BadgeShapes:primitive', 'CountsDropped:primitive']

export const At256: Story = { tags: ['critical'], name: 'Three tabs · counts fit 256px', args: { width: 'sidebar' }, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(canvasElement.querySelector('[data-sidebar-tabs]')).toHaveAttribute('data-sidebar-tab-counts', 'shown'))
  await expect(painted(tablist)).toEqual(['Agents·3', 'Alerts·12', 'Missions·42'])
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.getAttribute('aria-label'))).toEqual(names)
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.querySelector('[data-sidebar-tab-icon] svg') !== null)).toEqual([true, true, true])
  // Needing the person is filled, a neutral count is not: colour is not the only cue.
  const fill = (kind: string) => getComputedStyle(tablist.querySelector(`[data-sidebar-tab-marker="${kind}"]`)!).backgroundColor
  await expect(fill('needs-you')).not.toBe(fill('count'))
  await assertWhole(tablist)
} }
export const BadgeShapes: Story = { tags: ['critical'], name: 'Badges · 1, 12 and 99+', args: { width: 'sidebar' }, render: () => <BadgeSizes />, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(canvasElement.querySelector('[data-sidebar-tabs]')).toHaveAttribute('data-sidebar-tab-counts', 'shown'))
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
export const CountsDropped: Story = { tags: ['critical'], name: 'Three tabs · counts drop first', args: { width: 'tight' }, play: async ({ canvasElement }) => {
  const tablist = within(canvasElement).getByRole('tablist', { name: 'Sidebar' })
  await waitFor(() => expect(canvasElement.querySelector('[data-sidebar-tabs]')).toHaveAttribute('data-sidebar-tab-counts', 'dropped'))
  await expect(painted(tablist)).toEqual(['Agents·', 'Alerts·', 'Missions'])
  await expect(within(tablist).getAllByRole('tab').map(tab => tab.getAttribute('aria-label'))).toEqual(names)
  await assertWhole(tablist)
} }

const styles = stylex.create({
  host: { backgroundColor: surface.canvas, color: ink.fg, display: 'flex', flexDirection: 'column', height: '120px' },
  sidebar: { width: '256px' },
  tight: { width: '210px' },
})
