import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button as KitButton, Input } from './kit'
import { Button, Menu, MenuItem, MenuTrigger, Popover } from './assistant-ui/composition/Controls'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, interactionVars as interaction, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from './assistant-ui/composition-tokens.stylex'

/** One focus treatment: the shared 1px ring on keyboard focus only; a field shows focus with its border. */
function FocusTreatmentStory({ scheme = 'dark', width = 1280 }: { scheme?: Scheme; width?: number }) {
  const [query, setQuery] = React.useState('')
  return <main data-scheme={scheme} data-testid="focus-treatment" {...stylex.props(styles.root, styles.width(width), ...baselineTheme, scheme === 'light' && lightTheme)}>
    <span data-testid="ring-reference" aria-hidden="true" {...stylex.props(styles.reference)} />
    <div {...stylex.props(styles.row)}>
      <Button>Shared control</Button>
      <KitButton>Kit button</KitButton>
      <button type="button" {...stylex.props(styles.native)}>Native button</button>
      <MenuTrigger>
        <Button>Open menu</Button>
        <Popover><Menu aria-label="Actions"><MenuItem id="first">First action</MenuItem><MenuItem id="second">Second action</MenuItem></Menu></Popover>
      </MenuTrigger>
    </div>
    <Input label="Search" value={query} onChange={setQuery} placeholder="Search" />
  </main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Focus treatment',
  component: FocusTreatmentStory,
  parameters: { scenario: { _tag: 'primitive', reason: 'Exercises the shared focus ring and its pointer guard without domain state.' }, layout: 'fullscreen' },
  args: { scheme: 'dark', width: 1280 },
} satisfies Meta<typeof FocusTreatmentStory>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Dark1280:contract-fixture',
  'Dark440:contract-fixture',
  'Light1280:contract-fixture',
  'Light440:contract-fixture',
]
type Story = StoryObj<typeof meta>

const ring = (element: Element) => {
  const style = getComputedStyle(element)
  return { style: style.outlineStyle, width: style.outlineWidth, color: style.outlineColor, offset: style.outlineOffset }
}

/** True when a property transition with a duration covers the outline, so a ring would fade in. */
const transitionsOutline = (element: Element) => {
  const style = getComputedStyle(element)
  const durations = style.transitionDuration.split(',').map(value => parseFloat(value))
  return style.transitionProperty.split(',').some((property, index) => /^\s*(?:all|outline|outline-color|outline-width|outline-offset)\s*$/.test(property) && (durations[index % durations.length] ?? 0) > 0)
}

const play: Story['play'] = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const doc = canvasElement.ownerDocument
  const reference = getComputedStyle(canvas.getByTestId('ring-reference'))
  const controls = ['Shared control', 'Kit button', 'Native button'].map(name => canvas.getByRole('button', { name }))
  // Keyboard: every control shows the same 1px ring, 2px outside, in the shared ink.
  doc.body.focus()
  for (const control of controls) {
    await userEvent.tab()
    await waitFor(() => expect(doc.activeElement).toBe(control))
    await waitFor(() => expect(ring(control)).toEqual({ style: 'solid', width: '1px', color: reference.outlineColor, offset: '2px' }))
    await expect(transitionsOutline(control)).toBe(false)
  }
  // A pointer press on the control that already holds keyboard focus clears the ring; the next Tab restores it.
  const last = controls[controls.length - 1]!
  await userEvent.pointer({ keys: '[MouseLeft]', target: last })
  await waitFor(() => expect(ring(last).style).toBe('none'))
  await userEvent.tab({ shift: true })
  await waitFor(() => expect(ring(controls[1]!).style).toBe('solid'))
  // Menu items: the soft fill and the inset ring, never clipped.
  await userEvent.tab({ shift: true })
  await userEvent.tab({ shift: true })
  for (let i = 0; i < 4; i++) await userEvent.tab()
  await waitFor(() => expect(doc.activeElement).toBe(canvas.getByRole('button', { name: 'Open menu' })))
  await userEvent.keyboard('{Enter}')
  const item = await within(doc.body).findByRole('menuitem', { name: 'First action' })
  await waitFor(() => expect(doc.activeElement).toBe(item))
  await waitFor(() => expect(ring(item)).toEqual({ style: 'solid', width: '1px', color: reference.outlineColor, offset: '-1px' }))
  await userEvent.keyboard('{Escape}')
  // A text field: a border change on any focus, never an outline ring.
  const field = canvas.getByRole('textbox', { name: 'Search' })
  const frame = field.parentElement!
  const rest = getComputedStyle(frame).borderTopColor
  await userEvent.click(field)
  await waitFor(() => expect(getComputedStyle(frame).borderTopColor).toBe(reference.outlineColor))
  await expect(getComputedStyle(field).outlineStyle).toBe('none')
  await expect(rest).not.toBe(reference.outlineColor)
}

export const Dark1280: Story = { play }
export const Light1280: Story = { args: { scheme: 'light' }, play }
export const Dark440: Story = { args: { width: 440 }, play }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play }

const styles = stylex.create({
  root: { boxSizing: 'border-box', minHeight: '100vh', display: 'flex', flexDirection: 'column', gap: space.lg, padding: space.lg, backgroundColor: surface.canvas, color: text.fg, fontFamily: type.fontSans, fontSize: type.uiSize },
  width: (width: number) => ({ width, maxWidth: '100%' }),
  row: { display: 'flex', flexWrap: 'wrap', gap: space.md },
  native: { paddingInline: space.md, minHeight: g.controlMd, borderWidth: 0, borderRadius: space.xs, backgroundColor: surface.controlFill, color: text.fg, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  reference: { position: 'absolute', visibility: 'hidden', outlineColor: interaction.focusRing },
})
