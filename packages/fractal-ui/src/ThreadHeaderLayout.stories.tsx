import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button, ToggleButton } from 'react-aria-components'
import { ThreadHeader } from './assistant-ui/composition/Shell'
import { Icon } from './assistant-ui/composition/Icons'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const now = Date.parse('2026-01-15T12:00:00Z')
const longTitle = 'Reconcile the arrangement projection after the operator renames the shared workspace folder'
const NativeActions = () => <>
  <Button aria-label="Conversation menu" {...stylex.props(styles.native)}><Icon name="message" /></Button>
  <Button aria-label="Resources" {...stylex.props(styles.native)}><Icon name="folder" /></Button>
</>
const facts = {
  folder: 'dev3', status: 'working', statusLabel: 'Working', statusSince: now - 252_000, freshness: 'live', now, elapsed: '4m 12s',
  terminalAvailable: true, panelOpen: false, drawerOpen: false, onTogglePanel: () => {}, onToggleDrawer: () => {},
} as const

function InteractiveLayout() {
  const [besideTitle, setBesideTitle] = React.useState(false)
  return <section aria-label="Interactive layout" {...stylex.props(styles.example)}>
    <ToggleButton isSelected={besideTitle} onChange={setBesideTitle} {...stylex.props(styles.toggle)}>Status beside title</ToggleButton>
    <ThreadHeader {...facts} title="Row projection" nativeActions={<NativeActions />} layout={besideTitle ? 'status-beside-title' : 'status-with-actions'} />
  </section>
}
function ThreadHeaderLayoutStory({ scheme, narrow }: { scheme: Scheme; narrow: boolean }) {
  return <main data-scheme={scheme} {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <section aria-label="Status beside title" {...stylex.props(styles.example)}>
      <ThreadHeader {...facts} title="Row projection" nativeActions={<NativeActions />} layout="status-beside-title" />
    </section>
    <section aria-label="Long title beside status" {...stylex.props(styles.example)}>
      <ThreadHeader {...facts} title={longTitle} nativeActions={<NativeActions />} layout="status-beside-title" />
    </section>
    <section aria-label="Default layout control" {...stylex.props(styles.example)}>
      <ThreadHeader {...facts} title="Row projection" nativeActions={<NativeActions />} />
    </section>
    <InteractiveLayout />
  </main>
}

const rect = (element: Element) => element.getBoundingClientRect()
const px = (value: string) => Number.parseFloat(value)
const headerIn = (canvasElement: HTMLElement, name: string) => within(within(canvasElement).getByRole('region', { name })).getByTestId('thread-header')
const statusOf = (header: HTMLElement) => within(header).getByRole('img', { name: /^Working;/ }).parentElement!
/** The status starts one header gap after the breadcrumb: no free space separates it from the title. */
const statusBesideTitle = (header: HTMLElement) =>
  Math.abs(rect(statusOf(header)).left - rect(within(header).getByRole('navigation', { name: 'Breadcrumb' })).right - px(getComputedStyle(header).columnGap)) < 1
/** From `start` to the last control the group is contiguous and ends at the header's content edge. */
const trailingFrom = (header: HTMLElement, start: Element) => {
  const actions = start.parentElement!
  const group = [...actions.children].slice([...actions.children].indexOf(start))
  const gap = px(getComputedStyle(actions).columnGap)
  const contiguous = group.every((item, index) => index === 0 || Math.abs(rect(item).left - rect(group[index - 1]!).right - gap) < 1)
  return contiguous && Math.abs(rect(group.at(-1)!).right - (rect(header).right - px(getComputedStyle(header).paddingInlineEnd))) < 1
}
const slotOf = (header: HTMLElement) => within(header).getByTestId('native-action-slot')
/** Nothing in the header paints or scrolls past its own box. */
const overflowFree = (header: HTMLElement) => {
  const box = rect(header)
  return header.scrollWidth <= header.clientWidth && [...header.querySelectorAll('*')].every(element => rect(element).left >= box.left - 0.5 && rect(element).right <= box.right + 0.5)
}

const meta = {
  title: 'Fractal UI/Thread header layout', component: ThreadHeaderLayoutStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', narrow: false },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, narrow: { control: 'boolean' } },
} satisfies Meta<typeof ThreadHeaderLayoutStory>
export default meta
type Story = StoryObj<typeof meta>
/** Status and elapsed sit beside the title; the native action slot leads the trailing controls. */
export const Dark1440: Story = { play: async ({ canvasElement, args }) => {
  await document.fonts.ready
  const beside = headerIn(canvasElement, 'Status beside title')
  await expect(statusBesideTitle(beside)).toBe(true)
  await expect(trailingFrom(beside, slotOf(beside))).toBe(true)
  // Negative control: the free space sits between the status and the slot, so a trailing group started at the status is rejected.
  await expect(trailingFrom(beside, statusOf(beside))).toBe(false)
  const long = headerIn(canvasElement, 'Long title beside status')
  await expect(statusBesideTitle(long)).toBe(true)
  await expect(trailingFrom(long, slotOf(long))).toBe(true)
  const title = within(long).getByText(longTitle)
  if (args.narrow) await expect(title.scrollWidth).toBeGreaterThan(title.clientWidth)
  // Negative control: the default layout keeps status among the trailing actions, and the detector rejects it.
  const control = headerIn(canvasElement, 'Default layout control')
  await expect(statusBesideTitle(control)).toBe(false)
  await expect(trailingFrom(control, statusOf(control))).toBe(true)
  const interactive = headerIn(canvasElement, 'Interactive layout')
  const toggle = within(canvasElement).getByRole('button', { name: 'Status beside title' })
  await expect(statusBesideTitle(interactive)).toBe(false)
  await userEvent.click(toggle)
  await expect(toggle).toHaveAttribute('aria-pressed', 'true')
  await waitFor(() => expect(statusBesideTitle(interactive)).toBe(true))
  await expect(trailingFrom(interactive, slotOf(interactive))).toBe(true)
  await expect(trailingFrom(interactive, statusOf(interactive))).toBe(false)
  await userEvent.click(toggle)
  await waitFor(() => expect(statusBesideTitle(interactive)).toBe(false))
  await expect(trailingFrom(interactive, statusOf(interactive))).toBe(true)
  for (const header of [beside, long, control, interactive]) await expect(overflowFree(header)).toBe(true)
  const root = canvasElement.ownerDocument.documentElement
  await expect(root.scrollWidth).toBeLessThanOrEqual(root.clientWidth)
} }
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', minHeight: '100vh', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.xl, paddingBlock: s.xl, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  narrow: { width: g.polishNarrow },
  example: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.md },
  native: { width: g.controlMd, height: g.controlMd, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', borderWidth: 0, backgroundColor: 'transparent', color: c.fgMuted, borderRadius: r.control, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: 1 } },
  toggle: { height: g.controlSm, marginInline: g.gutter, paddingInline: s.sm, borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.control, backgroundColor: 'transparent', color: c.fg, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: 1 }, '[aria-pressed="true"]': { backgroundColor: c.rowHover } },
})
