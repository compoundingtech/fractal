import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { NoteList } from './NoteList'
import { agentNote, allNotes, diffNote, futureAudience, futureNote, longNote, newestFirstIds, retireNote, target, viewer } from './notes-fixtures'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as styles, type NotesFrameArgs } from './notes-story-frame'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'
import { Button } from '../assistant-ui/composition/Controls'

function NoteListStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  // The host owns data: an accepted archive leaves the note in place until the host publishes the
  // next `notes` value, which the story delivers explicitly so the accepted state is observable.
  const [notes, setNotes] = React.useState<readonly SubjectNote[]>(allNotes)
  const [archived, setArchived] = React.useState<readonly string[]>([])
  const onArchive = async (note: SubjectNote) => {
    await Promise.resolve()
    if (note.id === longNote.id) throw new Error('synthetic archive refusal')
    setArchived(current => [...current, note.id])
  }
  const publish = () => setNotes(current => current.filter(candidate => !archived.includes(candidate.id)))
  return <NotesFrame scheme={scheme} width={width} title="Note list">
    <section aria-label="Notes on the reviewer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>{target}</h2>
      <NoteList notes={notes} author={viewer} onArchive={onArchive} />
      {/* Negative control: a host label regression that leaks the raw audience value. */}
      {negativeControl && <span>{futureAudience}</span>}
    </section>
    <span><Button variant="outline" onPress={publish} data-testid="deliver-update">Deliver list update</Button></span>
    <section aria-label="Read-only notes" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Read-only</h2>
      <NoteList notes={[agentNote, futureNote]} />
    </section>
    <section aria-label="No notes" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Empty</h2>
      <NoteList notes={[]} author={viewer} onArchive={onArchive} />
    </section>
  </NotesFrame>
}

const meta = {
  title: 'Fractal/Plugins/Notes/Note list', component: NoteListStory, parameters: { layout: 'fullscreen' },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Leak the raw future audience next to the list: the neutral-label assertion must fail.'),
} satisfies Meta<typeof NoteListStory>
export default meta
type Story = StoryObj<typeof meta>

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const main = canvas.getByRole('region', { name: 'Notes on the reviewer' })
  const rowOf = (id: string) => main.querySelector<HTMLElement>(`[data-note-id="${id}"]`)!
  await expect([...main.querySelectorAll('[data-testid="note-row"]')].map(row => row.getAttribute('data-note-id'))).toEqual(newestFirstIds)

  // Audience: one mapping, agent marker as SVG, neutral label for future values.
  await expect(within(rowOf(retireNote.id)).getByTestId('note-audience')).toHaveTextContent('Only me')
  await expect(within(rowOf(diffNote.id)).getByTestId('note-audience')).toHaveTextContent('Shared with agent')
  await expectVisibleAgentMarker(rowOf(diffNote.id))
  await expect(rowOf(retireNote.id).querySelector('svg[data-note-agent-marker]')).toBeNull()
  await expect(within(rowOf(futureNote.id)).getByTestId('note-audience')).toHaveTextContent('Other audience')
  for (const forbidden of [futureAudience, 'Unknown']) await expect(main.textContent).not.toContain(forbidden)

  // Attribution, session scope, expiry.
  await expect(within(rowOf(retireNote.id)).getByTestId('note-author')).toHaveTextContent('You')
  await expect(within(rowOf(agentNote.id)).getByTestId('note-author')).toHaveTextContent(target)
  await expect(within(rowOf(retireNote.id)).getByTestId('note-session')).toHaveTextContent('This session')
  await expect(within(rowOf(diffNote.id)).getByTestId('note-expires')).toHaveTextContent(/^Expires in /)
  await expect(within(rowOf(longNote.id)).getByTestId('note-text').textContent).toBe(longNote.text)

  // Archive appears only on the viewer's own notes.
  await expect(within(main).getAllByRole('button', { name: 'Archive' })).toHaveLength(3)
  await expect(within(rowOf(agentNote.id)).queryByRole('button')).toBeNull()
  await expect(within(rowOf(futureNote.id)).queryByRole('button')).toBeNull()

  // A refused archive keeps the row and explains the failure.
  await userEvent.click(within(rowOf(longNote.id)).getByRole('button', { name: 'Archive' }))
  await expect(await within(rowOf(longNote.id)).findByRole('alert')).toHaveTextContent("Couldn't archive this note. It is unchanged; try again.")
  await expect(within(rowOf(longNote.id)).getByTestId('note-archive-error-detail')).toHaveTextContent('synthetic archive refusal')
  await expect(within(rowOf(longNote.id)).getByRole('button', { name: 'Archive' })).toBeEnabled()

  // An accepted archive keeps the row, disabled, until the host publishes; focus stays in the list.
  await userEvent.click(within(rowOf(diffNote.id)).getByRole('button', { name: 'Archive' }))
  await waitFor(() => expect(rowOf(diffNote.id)).toHaveAttribute('data-archive-state', 'accepted'))
  await expect(within(rowOf(diffNote.id)).getByRole('button', { name: 'Archived' })).toBeDisabled()
  await expect(within(main).getByRole('status')).toHaveTextContent('Archive accepted. The note leaves the list when it updates.')
  await expect(within(main).getByTestId('note-list')).toHaveFocus()
  await expect(main.querySelectorAll('[data-testid="note-row"]')).toHaveLength(5)
  await userEvent.click(canvas.getByTestId('deliver-update'))
  await waitFor(() => expect(main.querySelector(`[data-note-id="${diffNote.id}"]`)).toBeNull())
  await expect(main.querySelectorAll('[data-testid="note-row"]')).toHaveLength(4)
  // Pending announcements must survive a host publication without naming removed DOM nodes.
  for (const element of canvasElement.ownerDocument.querySelectorAll('[aria-labelledby]')) {
    for (const id of element.getAttribute('aria-labelledby')!.trim().split(/\s+/)) {
      await expect(canvasElement.ownerDocument.getElementById(id)).not.toBeNull()
    }
  }

  const readOnly = canvas.getByRole('region', { name: 'Read-only notes' })
  await expect(within(readOnly).getAllByTestId('note-row')).toHaveLength(2)
  await expect(within(readOnly).queryByRole('button')).toBeNull()
  await expect(within(canvas.getByRole('region', { name: 'No notes' })).getByTestId('note-list-empty')).toHaveTextContent('No notes yet.')

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }
