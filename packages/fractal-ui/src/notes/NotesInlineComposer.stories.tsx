import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { NoteBadge, NoteComposer, NoteList, type NoteAudienceChoice } from './index'
import { agentNote, diffNote, publishedNote, target, viewer } from './notes-fixtures'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as frame, type NotesFrameArgs } from './notes-story-frame'
import { Button } from '../assistant-ui/composition/Controls'
import { Icon } from '../assistant-ui/composition/Icons'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'

/**
 * UX variant A — inline quick composer. The detail pane keeps its notes in place; one click on
 * Add note opens the composer under the list, Enter adds, and focus returns to Add note.
 */
function InlineQuickComposerStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  const [notes, setNotes] = React.useState<readonly SubjectNote[]>([diffNote, agentNote])
  const [open, setOpen] = React.useState(false)
  // The remounting Add note takes focus itself; no frame timing to race the play.
  const close = () => setOpen(false)
  const onSubmit = async (text: string, audience: NoteAudienceChoice) => {
    await Promise.resolve()
    // Negative control: the host never publishes the created note.
    if (!negativeControl) setNotes(current => [publishedNote(text, audience), ...current])
    close()
  }
  return <NotesFrame scheme={scheme} width={width} title="Inline quick composer">
    <article aria-label="Agent detail" {...stylex.props(styles.pane)}>
      <header {...stylex.props(styles.header)}>
        <span {...stylex.props(styles.name)}>{target}</span>
        <NoteBadge notes={notes} />
      </header>
      <section aria-label="Notes" {...stylex.props(frame.section)}>
        <h2 {...stylex.props(frame.heading)}>Notes</h2>
        <NoteList notes={notes} author={viewer} />
        {open
          ? <div onKeyDown={event => { if (event.key === 'Escape' && (event.target as HTMLTextAreaElement).value === '') close() }}>
            <NoteComposer onSubmit={onSubmit} autoFocus />
          </div>
          : <Button variant="outline" size="md" autoFocus onPress={() => setOpen(true)} data-testid="inline-add-note"><Icon name="plus" />Add note</Button>}
      </section>
    </article>
  </NotesFrame>
}

const meta = {
  title: 'Fractal/Explore/Notes/Inline quick composer', component: InlineQuickComposerStory, parameters: { layout: 'fullscreen' },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('The host never publishes the created note: the list and badge assertions must fail.'),
} satisfies Meta<typeof InlineQuickComposerStory>
export default meta
type Story = StoryObj<typeof meta>

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const badge = canvas.getByTestId('note-badge')
  await expectVisibleAgentMarker(badge)
  await expectVisibleAgentMarker(canvas.getAllByTestId('note-row')[0]!)
  await expect(badge).toHaveAttribute('data-note-count', '2')

  // One click, the text, one confirm.
  await userEvent.click(canvas.getByTestId('inline-add-note'))
  const input = await canvas.findByTestId('note-composer-input')
  await waitFor(() => expect(input).toHaveFocus())
  await userEvent.keyboard('Hand off before the evening run{Enter}')
  await waitFor(() => expect(canvas.queryByTestId('note-composer')).toBeNull())
  const rows = canvas.getAllByTestId('note-row')
  await expect(rows).toHaveLength(3)
  await expect(within(rows[0]!).getByTestId('note-text')).toHaveTextContent('Hand off before the evening run')
  await expect(within(rows[0]!).getByTestId('note-audience')).toHaveTextContent('Only me')
  await expect(canvas.getByTestId('note-badge')).toHaveAttribute('data-note-count', '3')
  await waitFor(() => expect(canvas.getByTestId('inline-add-note')).toHaveFocus())

  // Escape on an empty draft collapses without losing anything; a draft keeps it open.
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(canvas.getByTestId('note-composer-input')).toHaveFocus())
  await userEvent.keyboard('x{Escape}')
  await expect(canvas.getByTestId('note-composer-input')).toHaveValue('x')
  await userEvent.keyboard('{Backspace}{Escape}')
  await waitFor(() => expect(canvas.getByTestId('inline-add-note')).toHaveFocus())

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }

const styles = stylex.create({
  pane: {
    display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0, padding: s.lg,
    backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.base,
  },
  header: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0, flexWrap: 'wrap' },
  name: { fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold, color: text.fg, minWidth: 0, overflowWrap: 'anywhere' },
})
