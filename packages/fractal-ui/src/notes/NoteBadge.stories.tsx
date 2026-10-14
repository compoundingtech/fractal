import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { agentNote, allNotes, diffNote, futureAudience, futureNote, longNote, retireNote } from './notes-fixtures'
import { NoteBadge } from './NoteBadge'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as styles, type NotesFrameArgs } from './notes-story-frame'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'

function NoteBadgeStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  // Negative control: the host drops the newest note, so the newest-preview assertion must fail.
  const many = negativeControl ? allNotes.filter(note => note !== retireNote) : allNotes
  return <NotesFrame scheme={scheme} width={width} title="Note badge">
    {([
      ['single', 'One self note', [retireNote]],
      ['many', 'Five notes, passed out of order', many],
      ['agent', 'Newest shared with agent', [agentNote, diffNote]],
      ['future', 'Future audience', [futureNote]],
      ['long', 'Long newest note', [longNote]],
      ['empty', 'No notes', []],
    ] as const).map(([id, label, notes]) => <section key={id} aria-label={label} data-badge-case={id} {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>{label}</h2>
      <div {...stylex.props(styles.row)}><NoteBadge notes={notes} /></div>
    </section>)}
  </NotesFrame>
}

const meta = {
  title: 'Fractal/Plugins/Notes/Note badge', component: NoteBadgeStory, parameters: { layout: 'fullscreen' },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Drop the newest note from the five-note case: the newest-preview assertion must fail.'),
} satisfies Meta<typeof NoteBadgeStory>
export default meta
type Story = StoryObj<typeof meta>

const badgeIn = (canvasElement: HTMLElement, id: string) => within(canvasElement.querySelector<HTMLElement>(`[data-badge-case="${id}"]`)!).getByTestId('note-badge')

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const single = badgeIn(canvasElement, 'single')
  await expect(single).toHaveAccessibleName(`1 note. Newest, Only me: ${retireNote.text}`)
  await expect(single.querySelector('[data-note-agent-marker]')).toBeNull()

  const many = badgeIn(canvasElement, 'many')
  await expect(many).toHaveAttribute('data-note-count', '5')
  await expect(within(many).getByTestId('note-badge-preview')).toHaveTextContent(retireNote.text)

  const agent = badgeIn(canvasElement, 'agent')
  await expect(agent).toHaveAccessibleName(`2 notes. Newest, Shared with agent: ${diffNote.text}`)
  await expectVisibleAgentMarker(agent)

  const future = badgeIn(canvasElement, 'future')
  await expect(future).toHaveAttribute('data-note-audience', 'other')
  await expect(future.getAttribute('aria-label')).toContain('Other audience')
  for (const forbidden of [futureAudience, 'Unknown']) await expect(canvasElement.textContent + (future.getAttribute('aria-label') ?? '')).not.toContain(forbidden)

  await expect(canvasElement.querySelector('[data-badge-case="empty"] [data-testid="note-badge"]')).toBeNull()

  // The preview is one truncated line; the tooltip carries the full newest text. Focus opens the
  // kit Tooltip (hover support is unchanged kit-wide behavior and not asserted here).
  const long = badgeIn(canvasElement, 'long')
  const preview = within(long).getByTestId('note-badge-preview')
  await expect(preview.scrollWidth).toBeGreaterThan(preview.clientWidth)
  await userEvent.click(canvasElement.querySelector('h1')!)
  await userEvent.tab()
  await expect(single).toHaveFocus()
  await expect(await canvas.findByRole('tooltip', undefined, { timeout: 5000 })).toHaveTextContent(retireNote.text)
  await userEvent.keyboard('{Escape}')
  // Pointer focus intentionally does not open a RAC tooltip; reach the long badge by keyboard.
  for (let index = 0; index < 4; index++) await userEvent.tab()
  await expect(long).toHaveFocus()
  await expect(await canvas.findByRole('tooltip', undefined, { timeout: 5000 })).toHaveTextContent(longNote.text.replace(/\s+/g, ' '))
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('tooltip')).toBeNull(), { timeout: 5000 })

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }
