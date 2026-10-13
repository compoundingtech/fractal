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
const shortTitle = 'Triage'
const longTitle = 'Reconcile the arrangement projection after the operator renames the shared workspace folder'

/** One header per story: every header carries a "Breadcrumb" navigation landmark, which must stay unique. */
function ThreadHeaderLayoutStory({ scheme, narrow, long }: { scheme: Scheme; narrow: boolean; long: boolean }) {
  const [besideTitle, setBesideTitle] = React.useState(false)
  return <main data-scheme={scheme} {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <ToggleButton isSelected={besideTitle} onChange={setBesideTitle} {...stylex.props(styles.toggle)}>Status beside title</ToggleButton>
    <ThreadHeader
      folder="fractal" title={long ? longTitle : shortTitle}
      status="working" statusLabel="Working" statusSince={now - 252_000} freshness="live" now={now} elapsed="4m 12s"
      nativeActions={<>
        <Button aria-label="Conversation menu" {...stylex.props(styles.native)}><Icon name="message" /></Button>
        <Button aria-label="Resources" {...stylex.props(styles.native)}><Icon name="folder" /></Button>
      </>}
      terminalAvailable panelOpen={false} drawerOpen={false} onTogglePanel={() => {}} onToggleDrawer={() => {}}
      layout={besideTitle ? 'status-beside-title' : 'status-with-actions'}
    />
  </main>
}

const rect = (element: Element) => element.getBoundingClientRect()
const px = (value: string) => Number.parseFloat(value)
const statusOf = (header: HTMLElement) => within(header).getByRole('img', { name: /^Working;/ }).parentElement!
/** The status starts one header gap after the breadcrumb: no free space separates it from the title. */
const statusBesideTitle = (header: HTMLElement) =>
  Math.abs(rect(statusOf(header)).left - rect(within(header).getByRole('navigation', { name: 'Breadcrumb' })).right - px(getComputedStyle(header).columnGap)) < 1
/** From `start` to the last control the group is contiguous and ends at the header's content edge. */
const trailingFrom = (header: HTMLElement, start: Element) => {
  const items = [...start.parentElement!.children]
  const group = items.slice(items.indexOf(start))
  const gap = px(getComputedStyle(start.parentElement!).columnGap)
  const contiguous = group.every((item, index) => index === 0 || Math.abs(rect(item).left - rect(group[index - 1]!).right - gap) < 1)
  return contiguous && Math.abs(rect(group.at(-1)!).right - (rect(header).right - px(getComputedStyle(header).paddingInlineEnd))) < 1
}
/** Nothing in the header scrolls or paints past its own box, and the page never scrolls sideways. */
const overflowFree = (header: HTMLElement) => {
  const box = rect(header), page = header.ownerDocument.documentElement
  return header.scrollWidth <= header.clientWidth && page.scrollWidth <= page.clientWidth
    && [...header.querySelectorAll('*')].every(element => rect(element).left >= box.left - 0.5 && rect(element).right <= box.right + 0.5)
}
const meta = {
  title: 'Fractal/Shell/Thread header layout', component: ThreadHeaderLayoutStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', narrow: false, long: false },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, narrow: { control: 'boolean' }, long: { control: 'boolean' } },
} satisfies Meta<typeof ThreadHeaderLayoutStory>
export default meta
type Story = StoryObj<typeof meta>
/** Status and elapsed move beside the title; the native action slot leads the trailing controls at the end edge. */
export const Dark1440: Story = { play: async ({ canvasElement, args }) => {
  await document.fonts.ready
  const canvas = within(canvasElement)
  const header = canvas.getByTestId('thread-header')
  const slot = within(header).getByTestId('native-action-slot')
  const toggle = canvas.getByRole('button', { name: 'Status beside title' })
  // A long title at 440 fills the band, leaving no free space for either control to observe.
  const roomy = !(args.long && args.narrow)
  // Negative control: the default layout keeps the status in the trailing group, and the detector rejects it.
  if (roomy) await expect(statusBesideTitle(header)).toBe(false)
  await expect(trailingFrom(header, slot)).toBe(true)
  await userEvent.click(toggle)
  await expect(toggle).toHaveAttribute('aria-pressed', 'true')
  await waitFor(() => expect(statusBesideTitle(header)).toBe(true))
  await expect(trailingFrom(header, slot)).toBe(true)
  // Negative control: the free space now sits between the status and the slot, so a group started at the status is rejected.
  if (roomy) await expect(trailingFrom(header, statusOf(header))).toBe(false)
  const title = within(header).getByText(args.long ? longTitle : shortTitle)
  if (!roomy) await expect(title.scrollWidth).toBeGreaterThan(title.clientWidth)
  await expect(overflowFree(header)).toBe(true)
  await userEvent.click(toggle)
  await expect(toggle).toHaveAttribute('aria-pressed', 'false')
  if (roomy) await waitFor(() => expect(statusBesideTitle(header)).toBe(false))
  await expect(trailingFrom(header, statusOf(header))).toBe(true)
  await expect(overflowFree(header)).toBe(true)
} }
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
/** A long title truncates; the status, the slot and the controls keep their width without overflow. */
export const LongTitleDark440: Story = { ...Dark1440, args: { narrow: true, long: true } }
export const LongTitleLight440: Story = { ...Dark1440, args: { narrow: true, long: true, scheme: 'light' } }
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', minHeight: '100vh', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.md, paddingBlock: s.xl, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  narrow: { width: g.polishNarrow },
  native: { width: g.controlMd, height: g.controlMd, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', borderWidth: 0, backgroundColor: 'transparent', color: c.fgMuted, borderRadius: r.control, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: 1 } },
  toggle: { alignSelf: 'flex-start', height: g.controlSm, marginInline: g.gutter, paddingInline: s.sm, borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.control, backgroundColor: 'transparent', color: c.fg, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', ':is([data-selected])': { backgroundColor: c.rowHover }, ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: 1 } },
})
