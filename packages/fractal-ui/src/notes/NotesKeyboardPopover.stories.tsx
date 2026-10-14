import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button, Dialog, Input, Label, Popover, TextField } from 'react-aria-components'
import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { NoteBadge, NoteComposer, type NoteAudienceChoice } from './index'
import { agentNote, diffNote, plannerNote, plannerTarget, publishedNote, target } from './notes-fixtures'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as frame, type NotesFrameArgs } from './notes-story-frame'
import { ThemePortal } from '../assistant-ui/taste/ThemePortal'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'
import { accentVars as accent, borderVars as border, elevationVars as elevation, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'

const targets = [target, plannerTarget] as const
type Target = typeof targets[number]

/**
 * UX variant B — keyboard `n` popover. With focus anywhere outside a text field, `n` opens a
 * composer anchored to the selected row; Enter adds and closes, Escape cancels, focus returns
 * to the row. Typing `n` into a field never opens it.
 */
function KeyboardPopoverStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  const [notes, setNotes] = React.useState<Readonly<Record<Target, readonly SubjectNote[]>>>({ [target]: [diffNote, agentNote], [plannerTarget]: [plannerNote] })
  const [selected, setSelected] = React.useState<Target>(target)
  const [open, setOpen] = React.useState(false)
  const rowRefs = { [target]: React.useRef<HTMLButtonElement>(null), [plannerTarget]: React.useRef<HTMLButtonElement>(null) }
  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'n' || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.isComposing) return
      // Negative control: drop the typing guard, so `n` typed into a field opens the composer.
      if (!negativeControl && event.target instanceof Element && event.target.closest('input, textarea, [contenteditable="true"]') !== null) return
      event.preventDefault()
      setOpen(true)
    }
    document.addEventListener('keydown', onKeyDown, true)
    return () => document.removeEventListener('keydown', onKeyDown, true)
  }, [negativeControl])
  const onSubmit = async (text: string, audience: NoteAudienceChoice) => {
    await Promise.resolve()
    // The planner refuses creates in this fixture, so the popover's failure path stays visible.
    if (selected === plannerTarget) throw new Error('synthetic create refusal')
    setNotes(current => ({ ...current, [selected]: [publishedNote(text, audience, selected), ...current[selected]] }))
    setOpen(false)
  }
  return <NotesFrame scheme={scheme} width={width} title="Keyboard n popover">
    <ThemePortal>
      <p data-testid="popover-hint" {...stylex.props(frame.heading)}>Press <kbd {...stylex.props(styles.kbd)}>N</kbd> to add a note to the selected agent.</p>
      <TextField {...stylex.props(styles.search)}>
        <Label {...stylex.props(frame.heading)}>Filter agents</Label>
        <Input data-testid="popover-search" {...stylex.props(styles.input)} />
      </TextField>
      <ul aria-label="Agents" {...stylex.props(styles.rows)}>
        {targets.map(ref => <li key={ref} {...stylex.props(styles.row, ref === selected && styles.selectedRow)}>
          <Button ref={rowRefs[ref]} aria-pressed={ref === selected} aria-keyshortcuts="N" data-testid={`row-${ref}`} onPress={() => setSelected(ref)} {...stylex.props(styles.rowButton)}>{ref}</Button>
          <span data-testid={`badge-${ref}`}><NoteBadge notes={notes[ref]} /></span>
        </li>)}
      </ul>
      <Popover triggerRef={rowRefs[selected]} isOpen={open} onOpenChange={setOpen} placement="bottom start" offset={6} {...stylex.props(styles.popover)}>
        <Dialog aria-label={`Add note to ${selected}`} {...stylex.props(styles.dialog)}>
          <NoteComposer onSubmit={onSubmit} autoFocus />
        </Dialog>
      </Popover>
    </ThemePortal>
  </NotesFrame>
}

const meta = {
  title: 'Fractal/Explore/Notes/Keyboard n popover', component: KeyboardPopoverStory, parameters: { layout: 'fullscreen' },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Drop the typing guard: typing n into the filter opens the popover and the guard assertion must fail.'),
} satisfies Meta<typeof KeyboardPopoverStory>
export default meta
type Story = StoryObj<typeof meta>

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const count = (ref: string) => within(canvas.getByTestId(`badge-${ref}`)).getByTestId('note-badge').getAttribute('data-note-count')
  await expectVisibleAgentMarker(canvas.getByTestId(`badge-${target}`))
  await expect(canvas.getByTestId('popover-hint')).toHaveTextContent('Press N to add a note')

  // Typing n into a field types it and never opens the composer.
  await userEvent.click(canvas.getByTestId('popover-search'))
  await userEvent.keyboard('n')
  await expect(canvas.getByTestId('popover-search')).toHaveValue('n')
  await expect(canvas.queryByRole('dialog')).toBeNull()

  // n opens the composer on the selected row; Enter adds, closes and restores focus.
  const reviewer = canvas.getByTestId(`row-${target}`)
  await userEvent.click(reviewer)
  await expect(reviewer).toHaveAttribute('aria-pressed', 'true')
  await userEvent.keyboard('n')
  const dialog = await canvas.findByRole('dialog', { name: `Add note to ${target}` })
  const input = within(dialog).getByTestId('note-composer-input')
  await waitFor(() => expect(input).toHaveFocus())
  await expect(input).toHaveValue('')
  await userEvent.keyboard('Ship after review{Enter}')
  await waitFor(() => expect(canvas.queryByRole('dialog')).toBeNull())
  await expect(count(target)).toBe('3')
  await waitFor(() => expect(reviewer).toHaveFocus())

  // A refused create keeps the popover, the text and an announced error; Escape cancels.
  const planner = canvas.getByTestId(`row-${plannerTarget}`)
  await userEvent.click(planner)
  await userEvent.keyboard('n')
  const plannerDialog = await canvas.findByRole('dialog', { name: `Add note to ${plannerTarget}` })
  await waitFor(() => expect(within(plannerDialog).getByTestId('note-composer-input')).toHaveFocus())
  await userEvent.keyboard('Budget{Enter}')
  await expect(await within(plannerDialog).findByRole('alert')).toHaveTextContent("Couldn't add the note. Your text is kept; try again.")
  await expect(within(plannerDialog).getByTestId('note-composer-input')).toHaveValue('Budget')
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('dialog')).toBeNull())
  await expect(count(plannerTarget)).toBe('1')
  await waitFor(() => expect(planner).toHaveFocus())

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }

const styles = stylex.create({
  kbd: { fontFamily: t.fontMono, fontSize: t.denseSize, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.keyboard },
  search: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  input: {
    height: g.controlMd, boxSizing: 'border-box', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong,
    borderRadius: r.control, backgroundColor: surface.controlFill, color: text.fg, fontFamily: t.fontSans, fontSize: t.metaSize,
    outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  rows: { listStyle: 'none', margin: s.zero, padding: s.zero, display: 'flex', flexDirection: 'column', gap: s.xs2, minWidth: 0 },
  row: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0, minHeight: g.menuRow, paddingInline: s.sm, borderRadius: r.control },
  selectedRow: { backgroundColor: surface.rowActive },
  rowButton: {
    flex: '1 1 auto', minWidth: 0, textAlign: 'start', borderWidth: 0, backgroundColor: surface.transparent, color: text.fg, padding: s.zero,
    fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', cursor: 'pointer',
    outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  popover: {
    width: `min(${g.modalMax}, calc(100vw - ${s.panel}))`, boxSizing: 'border-box', padding: s.xs,
    backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.lg, boxShadow: elevation.popover,
  },
  dialog: { outlineStyle: 'none' },
})
