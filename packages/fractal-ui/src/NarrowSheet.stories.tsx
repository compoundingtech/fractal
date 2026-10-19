import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Button } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { NarrowActionRow, NarrowSheet } from './assistant-ui/composition/NarrowLayout'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

/** A phone thread's two edge sheets as a host opens them: Agents from the start edge, Agent details from the end edge. */
function NarrowSheetStory({ scheme = 'dark', side = 'end' }: { scheme?: Scheme; side?: 'start' | 'end' }) {
  const [open, setOpen] = React.useState(false)
  const [portal, setPortal] = React.useState<HTMLDivElement | null>(null)
  const label = side === 'start' ? 'Agents' : 'Agent details'
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <Button aria-haspopup="dialog" aria-expanded={open} onPress={() => setOpen(true)} {...stylex.props(styles.opener)}>Open {label.toLowerCase()}</Button>
    <div ref={setPortal} />
    {portal === null ? null : <NarrowSheet id="story-sheet" label={label} side={side} isOpen={open} onOpenChange={setOpen} width={side === 'start' ? 320 : 400} portalContainer={portal}
      actions={side === 'end' ? <NarrowActionRow label="Agent actions"><Button {...stylex.props(styles.opener)}>Terminal</Button></NarrowActionRow> : undefined}>
      <p {...stylex.props(styles.body)}>{side === 'start' ? 'Example roster for workstation-a.' : 'Example captured changes for the selected agent.'}</p>
    </NarrowSheet>}
  </main>
}
const meta = { title: 'Fractal UI/Narrow sheet', component: NarrowSheetStory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', side: 'end' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, side: { options: ['start', 'end'], control: 'radio' } } } satisfies Meta<typeof NarrowSheetStory>
export default meta
type Story = StoryObj<typeof meta>

/**
 * The close button is a 44px touch target inside the sheet header, the sheet always leaves a 40px strip of
 * the thread visible, and closing returns focus to the opener (close button, then Escape).
 */
const sheetPlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const label = args.side === 'start' ? 'Agents' : 'Agent details'
  const opener = canvas.getByRole('button', { name: `Open ${label.toLowerCase()}` })
  await userEvent.click(opener)
  const dialog = await canvas.findByRole('dialog', { name: label })
  await expect(dialog).toHaveAttribute('data-narrow-sheet', args.side ?? 'end')
  const close = within(dialog).getByRole('button', { name: `Close ${label.toLowerCase()}` })
  const closeRect = close.getBoundingClientRect()
  await expect(closeRect.width).toBeGreaterThanOrEqual(44)
  await expect(closeRect.height).toBeGreaterThanOrEqual(44)
  const sheet = dialog.getBoundingClientRect()
  await expect(closeRect.left).toBeGreaterThanOrEqual(sheet.left)
  await expect(closeRect.right).toBeLessThanOrEqual(sheet.right)
  await expect(closeRect.top).toBeGreaterThanOrEqual(sheet.top)
  await expect(sheet.width).toBeLessThanOrEqual(window.innerWidth - 40)
  await expect(args.side === 'start' ? sheet.left : window.innerWidth - sheet.right).toBeLessThanOrEqual(1)
  await userEvent.click(close)
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: label })).toBeNull())
  await waitFor(() => expect(opener).toHaveFocus())
  await userEvent.keyboard('{Enter}')
  await canvas.findByRole('dialog', { name: label })
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('dialog', { name: label })).toBeNull())
  await waitFor(() => expect(opener).toHaveFocus())
}
export const DetailsEnd: Story = { tags: ['critical'], play: sheetPlay }
export const DetailsEndLight: Story = { ...DetailsEnd, args: { scheme: 'light' } }
export const AgentsStart: Story = { tags: ['critical'], args: { side: 'start' }, play: sheetPlay }
export const AgentsStartLight: Story = { ...AgentsStart, args: { side: 'start', scheme: 'light' } }

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  opener: { minHeight: g.touchTarget, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.control, backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: c.primary } },
  body: { margin: 0, padding: s.lg },
})
