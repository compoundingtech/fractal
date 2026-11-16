import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { EmbraceComposer } from './assistant-ui/EmbraceComposer'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { Markdown } from './assistant-ui/composition/Markdown'
import { CopyAnswerButton } from './assistant-ui/composition/Transcript'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { Icon } from './assistant-ui/composition/Icons'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, interactionVars as interaction, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const answer = 'The composer uses a border for frame focus and a ring for focused actions.'
const source = 'const focus = "border"\n'
const options = { messages: [], isRunning: false, onNew: async () => {} }
/** `nested` renders one labelled region inside a shared main so the matrix keeps unique landmarks. */
function ComposerCopyFeedback({ scheme, width, variant, nested = false }: { scheme: Scheme; width: 440 | 1440; variant: 'C2' | 'C3'; nested?: boolean }) {
  const [revision, setRevision] = React.useState(0)
  const [failure, setFailure] = React.useState(false)
  const Root = nested ? 'section' : 'main'
  return <Root aria-label={nested ? `${scheme} ${width}` : undefined} data-scheme={scheme} data-revision={revision} {...stylex.props(styles.root, styles.width(width), ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <div {...stylex.props(styles.toolbar)}><Button onPress={() => setRevision(value => value + 1)}>Rerender snapshot</Button><Button onPress={() => setFailure(value => !value)}>Toggle copy failure</Button></div>
    <EmbraceRuntimeProvider options={options}><EmbraceComposer variant={variant} history={['Review the composer focus treatment.']} /></EmbraceRuntimeProvider>
    <Markdown text={'```typescript\n' + source + '```'} />
    <section aria-label={nested ? `Answer action ${scheme} ${width}` : 'Answer action'}><p>{answer}</p><CopyAnswerButton text={answer} onCopy={async () => { if (failure) throw new Error('Host copy rejected'); await navigator.clipboard.writeText(answer) }} /></section>
    <span data-testid="focus-reference" {...stylex.props(styles.reference)} />
    <span data-testid="copy-glyph" hidden><Icon name="copy" size={14} /></span>
    <span data-testid="check-glyph" hidden><Icon name="check" size={14} /></span>
  </ThemePortal></Root>
}
const styles = stylex.create({
  root: { boxSizing: 'border-box', minHeight: '100vh', maxWidth: '100%', padding: s.lg, display: 'flex', flexDirection: 'column', gap: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  width: (width: number) => ({ width }),
  toolbar: { display: 'flex', flexWrap: 'wrap', gap: s.sm },
  reference: { borderColor: interaction.composerFocusBorder, outlineColor: interaction.focusRing, outlineWidth: g.focusRing, outlineStyle: 'solid', position: 'absolute', visibility: 'hidden' },
})
const meta = { title: 'Fractal UI/Composer copy feedback', component: ComposerCopyFeedback, excludeStories: ['scenarioExemptions'], parameters: { scenario: { _tag: 'contract-fixture', reason: 'Local clipboard outcomes, focus geometry and acknowledgement timers isolate host-owned composer and copy contracts.' }, layout: 'fullscreen' }, args: { scheme: 'dark', width: 440, variant: 'C3' }, argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, width: { control: 'radio', options: [440, 1440] }, variant: { control: 'radio', options: ['C2', 'C3'] } } } satisfies Meta<typeof ComposerCopyFeedback>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Dark1440:contract-fixture',
  'Dark440:contract-fixture',
  'Light1440:contract-fixture',
  'Light440:contract-fixture',
]
type Story = StoryObj<typeof meta>

const play: Story['play'] = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const descriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
  let denied = false
  const writes: string[] = []
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (text: string) => { if (denied) throw new Error('Permission denied'); writes.push(text) } } })
  try {
    const rerender = () => userEvent.click(canvas.getByRole('button', { name: 'Rerender snapshot' }))
    const editor = canvas.getByRole('textbox')
    await userEvent.click(editor)
    await rerender()
    await userEvent.click(editor)
    const frame = editor.closest('form')!
    const reference = getComputedStyle(canvas.getByTestId('focus-reference'))
    await expect(getComputedStyle(frame).borderColor).toBe(reference.borderColor)
    await expect(getComputedStyle(frame).outlineStyle).toBe('none')
    if (args.variant === 'C3') await userEvent.click(canvas.getByRole('button', { name: 'Recall previous message' }))
    else await userEvent.type(editor, 'Review focus')
    for (const name of args.variant === 'C3' ? ['Recall previous message', 'Send'] : ['Send']) {
      const button = canvas.getByRole('button', { name })
      button.focus()
      await userEvent.keyboard('{Tab}{Shift>}{Tab}{/Shift}')
      await expect(getComputedStyle(button).outlineColor).toBe(reference.outlineColor)
      await expect(getComputedStyle(button).outlineWidth).toBe(reference.outlineWidth)
    }
    const glyph = (testId: string) => canvas.getByTestId(testId).querySelector('svg')!.innerHTML
    const tooltip = () => within(document.body).queryByRole('tooltip')
    for (const name of ['Copy code', 'Copy answer']) {
      // A string name matches the whole accessible name, so "Copy code" never matches "Copy code: Copied".
      const button = canvas.getByRole('button', { name })
      const status = name === 'Copy code' ? button.closest('[data-testid="markdown-code-block"]')!.querySelector('[role="status"]')! : canvas.getByRole('region', { name: 'Answer action' }).querySelector('[role="status"]')!
      await expect(status).toHaveAttribute('aria-live', 'polite')
      await expect(button.querySelector('svg')!.innerHTML).toBe(glyph('copy-glyph'))
      // Pointer copy: the tooltip opens on its own and reads Copied, even though the press closed the hover tooltip.
      await userEvent.click(button)
      await waitFor(() => expect(button).toHaveAccessibleName(`${name}: Copied`))
      await expect(button.querySelector('svg')!.innerHTML).toBe(glyph('check-glyph'))
      await expect(status).toHaveTextContent('Copied')
      await waitFor(() => expect(tooltip()).toHaveTextContent('Copied'))
      // Leaving the button dismisses the tooltip early; the glyph, name and announcement keep the acknowledgement across a rerender.
      await rerender()
      await expect(button).toHaveAccessibleName(`${name}: Copied`)
      await expect(button.querySelector('svg')!.innerHTML).toBe(glyph('check-glyph'))
      await waitFor(() => expect(button).toHaveAccessibleName(name), { timeout: 2200 })
      await expect(button.querySelector('svg')!.innerHTML).toBe(glyph('copy-glyph'))
      await expect(status).toHaveTextContent('')
      await waitFor(() => expect(tooltip()).toBeNull())
      // Keyboard copy keeps focus on the action; Escape dismisses the Copied tooltip without cancelling the acknowledgement.
      button.focus()
      await userEvent.keyboard('{Enter}')
      await waitFor(() => expect(button).toHaveAccessibleName(`${name}: Copied`))
      await expect(button).toHaveFocus()
      await waitFor(() => expect(tooltip()).toHaveTextContent('Copied'))
      await userEvent.keyboard('{Escape}')
      await waitFor(() => expect(tooltip()).toBeNull())
      await expect(button).toHaveAccessibleName(`${name}: Copied`)
      await expect(button).toHaveFocus()
      await waitFor(() => expect(button).toHaveAccessibleName(name), { timeout: 2200 })
      button.blur()
      // Passing negative control: a rejected operation never claims success, including after rerender.
      denied = true
      if (name === 'Copy answer') await userEvent.click(canvas.getByRole('button', { name: 'Toggle copy failure' }))
      await userEvent.click(button)
      await waitFor(() => expect(status).toHaveTextContent('Could not copy'))
      await rerender()
      await expect(button).toHaveAccessibleName(name)
      await expect(button.querySelector('svg')!.innerHTML).toBe(glyph('copy-glyph'))
      await expect(status).not.toHaveTextContent('Copied')
      await expect(tooltip()?.textContent ?? '').not.toContain('Copied')
      denied = false
      if (name === 'Copy answer') await userEvent.click(canvas.getByRole('button', { name: 'Toggle copy failure' }))
    }
    await expect(writes).toEqual([source, source, answer, answer])
  } finally {
    if (descriptor === undefined) Reflect.deleteProperty(navigator, 'clipboard')
    else Object.defineProperty(navigator, 'clipboard', descriptor)
  }
}
export const Dark440: Story = { play }
export const Light440: Story = { args: { scheme: 'light' }, play }
export const Dark1440: Story = { args: { width: 1440 }, play }
export const Light1440: Story = { args: { scheme: 'light', width: 1440, variant: 'C2' }, play }
export const AllStates: Story = { render: () => <main>{(['light', 'dark'] as const).flatMap(scheme => ([440, 1440] as const).map(width => <ComposerCopyFeedback key={`${scheme}-${width}`} scheme={scheme} width={width} variant="C3" nested />))}</main> }
