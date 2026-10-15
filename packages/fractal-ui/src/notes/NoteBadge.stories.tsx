import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { agentNote, allNotes, diffNote, futureNote, longNote, retireNote } from './notes-fixtures'
import { NoteBadge } from './NoteBadge'
import { NotesFrame, expectNoAudiencePresentation, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as styles, type NotesFrameArgs } from './notes-story-frame'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'

function NoteBadgeStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  // Negative control: the host drops the newest note from the five-note case and the stored-`agent`
  // note from the mixed case, so the newest-preview and count assertions must fail.
  const many = negativeControl ? allNotes.filter(note => note !== retireNote) : allNotes
  const mixed = negativeControl ? [retireNote] : [diffNote, retireNote]
  return <NotesFrame scheme={scheme} width={width} title="Note badge">
    {([
      ['single', 'One note stored as self', [retireNote]],
      ['many', 'Five notes, passed out of order', many],
      ['agent', 'Two notes stored as agent', [agentNote, diffNote]],
      ['future', 'One note with a future stored value', [futureNote]],
      ['long', 'Long newest note', [longNote]],
      ['mixed', 'Older agent note, newer self note', mixed],
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
  argTypes: notesFrameArgTypes('Drop the newest note from the five-note case and the stored-agent note from the mixed case: the newest-preview and count assertions must fail.'),
} satisfies Meta<typeof NoteBadgeStory>
export default meta
type Story = StoryObj<typeof meta>

const badgeIn = (canvasElement: HTMLElement, id: string) => within(canvasElement.querySelector<HTMLElement>(`[data-badge-case="${id}"]`)!).getByTestId('note-badge')

/** Scope only: the badge must never claim the agent received, read or acknowledged a note. */
const deliveryClaim = /deliver|receiv|acknowledg|has read|was read|seen by|reads this/i

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  // Every stored audience renders alike: count, newest preview, no audience signal.
  const single = badgeIn(canvasElement, 'single')
  await expect(single).toHaveAccessibleName(`1 note. Newest: ${retireNote.text}`)

  const many = badgeIn(canvasElement, 'many')
  await expect(many).toHaveAttribute('data-note-count', '5')
  await expect(within(many).getByTestId('note-badge-preview')).toHaveTextContent(retireNote.text)

  await expect(badgeIn(canvasElement, 'agent')).toHaveAccessibleName(`2 notes. Newest: ${diffNote.text}`)
  await expect(badgeIn(canvasElement, 'future')).toHaveAccessibleName(`1 note. Newest: ${futureNote.text}`)
  const mixed = badgeIn(canvasElement, 'mixed')
  await expect(mixed).toHaveAccessibleName(`2 notes. Newest: ${retireNote.text}`)

  for (const badge of canvasElement.querySelectorAll('[data-testid="note-badge"]')) {
    await expect(badge.querySelector('[data-note-agent-marker]')).toBeNull()
    await expect(badge.getAttribute('aria-label')).not.toMatch(deliveryClaim)
  }
  await expectNoAudiencePresentation(canvasElement)

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
  // Focus disclosure carries count and newest text only.
  await userEvent.tab()
  await expect(mixed).toHaveFocus()
  const mixedTip = await canvas.findByRole('tooltip', undefined, { timeout: 5000 })
  await expect(mixedTip).toHaveTextContent(`2 notes. Newest: ${retireNote.text}`)
  await expectNoAudiencePresentation(mixedTip)
  await expect(mixedTip.textContent).not.toMatch(deliveryClaim)
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('tooltip')).toBeNull(), { timeout: 5000 })

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }
