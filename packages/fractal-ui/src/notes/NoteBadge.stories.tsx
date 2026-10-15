import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { agentNote, allNotes, diffNote, futureAudience, futureNote, longNote, plannerNote, retireNote } from './notes-fixtures'
import { NoteBadge, agentSharedLabel } from './NoteBadge'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as styles, type NotesFrameArgs } from './notes-story-frame'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'

const agentLabel = 'Shared with the agent (people can see it too)'

function NoteBadgeStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  // Negative control: the host drops the newest note from the five-note case and the older shared
  // note from the mixed case, so the newest-preview and aggregate-sharing assertions must fail.
  const many = negativeControl ? allNotes.filter(note => note !== retireNote) : allNotes
  const mixed = negativeControl ? [retireNote] : [diffNote, retireNote]
  return <NotesFrame scheme={scheme} width={width} title="Note badge">
    {([
      ['single', 'One self note', [retireNote]],
      ['many', 'Five notes, passed out of order', many],
      ['agent', 'Newest shared with the agent', [agentNote, diffNote]],
      ['future', 'Future audience', [futureNote]],
      ['long', 'Long newest note', [longNote]],
      ['mixed', 'Older note shared, newest private', mixed],
      ['private', 'Only private notes', [plannerNote, retireNote]],
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
  argTypes: notesFrameArgTypes('Drop the newest note from the five-note case and the older shared note from the mixed case: the newest-preview and aggregate-sharing assertions must fail.'),
} satisfies Meta<typeof NoteBadgeStory>
export default meta
type Story = StoryObj<typeof meta>

const badgeIn = (canvasElement: HTMLElement, id: string) => within(canvasElement.querySelector<HTMLElement>(`[data-badge-case="${id}"]`)!).getByTestId('note-badge')

/** Sharing scope only: the badge must never claim the agent received, read or acknowledged a note. */
const deliveryClaim = /deliver|receiv|acknowledg|has read|was read|seen by|reads this/i

/** Aggregate sharing is its own signal; the newest note's audience is labelled separately beside the preview. */
const expectSharing = async (badge: HTMLElement, shared: boolean, newest: string) => {
  await expect(badge).toHaveAttribute('data-note-agent-shared', String(shared))
  await expect(within(badge).getByTestId('note-badge-newest-audience')).toHaveTextContent(newest)
  if (shared) await expectVisibleAgentMarker(within(badge).getByTestId('note-badge-agent-shared'))
  else {
    await expect(within(badge).queryByTestId('note-badge-agent-shared')).toBeNull()
    await expect(badge.querySelector('[data-note-agent-marker]')).toBeNull()
    await expect(badge.getAttribute('aria-label')).not.toContain(agentSharedLabel)
  }
  await expect(badge.getAttribute('aria-label')).not.toMatch(deliveryClaim)
}

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const single = badgeIn(canvasElement, 'single')
  await expect(single).toHaveAccessibleName(`1 note. Newest, Only me: ${retireNote.text}`)
  await expectSharing(single, false, 'Only me')

  // Five notes, newest private, older ones shared: the private preview never hides the sharing.
  const many = badgeIn(canvasElement, 'many')
  await expect(many).toHaveAttribute('data-note-count', '5')
  await expect(within(many).getByTestId('note-badge-preview')).toHaveTextContent(retireNote.text)
  await expectSharing(many, true, 'Only me')

  const agent = badgeIn(canvasElement, 'agent')
  await expect(agent).toHaveAccessibleName(`2 notes. ${agentSharedLabel}. Newest, ${agentLabel}: ${diffNote.text}`)
  await expect(agent).toHaveAttribute('data-note-audience', 'agent')
  await expectSharing(agent, true, 'Shared')

  // Mixed: the older shared note keeps the aggregate signal; the newest is labelled private.
  const mixed = badgeIn(canvasElement, 'mixed')
  await expect(mixed).toHaveAccessibleName(`2 notes. ${agentSharedLabel}. Newest, Only me: ${retireNote.text}`)
  await expect(mixed).toHaveAttribute('data-note-audience', 'self')
  await expectSharing(mixed, true, 'Only me')

  // Negative: several notes, none shared, show no sharing signal at all.
  const privateOnly = badgeIn(canvasElement, 'private')
  await expect(privateOnly).toHaveAccessibleName(`2 notes. Newest, Only me: ${retireNote.text}`)
  await expectSharing(privateOnly, false, 'Only me')

  // A future audience is neither private nor agent-shared, and never leaks its raw value.
  const future = badgeIn(canvasElement, 'future')
  await expect(future).toHaveAttribute('data-note-audience', 'other')
  await expect(future.getAttribute('aria-label')).toContain('Other audience')
  await expectSharing(future, false, 'Other audience')
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
  // Hover/focus disclosure spells out both signals in words, not only the glyph.
  await userEvent.tab()
  await expect(mixed).toHaveFocus()
  const mixedTip = await canvas.findByRole('tooltip', undefined, { timeout: 5000 })
  await expect(mixedTip).toHaveTextContent(`${agentSharedLabel}. Newest, Only me: ${retireNote.text}`)
  await expect(mixedTip.textContent).not.toMatch(deliveryClaim)
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('tooltip')).toBeNull(), { timeout: 5000 })

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }
