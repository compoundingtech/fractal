import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button as AriaButton } from 'react-aria-components'
import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { NoteBadge, NoteComposer, NoteList, defaultNoteAudience, type NoteAudienceChoice, type NoteDraft } from './index'
import { agentNote, diffNote, plannerNote, plannerTarget, publishedNote, target, viewer } from './notes-fixtures'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as frame, type NotesFrameArgs } from './notes-story-frame'
import { Button } from '../assistant-ui/composition/Controls'
import { Icon } from '../assistant-ui/composition/Icons'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'

const targets = [target, plannerTarget] as const
type Target = typeof targets[number]
const emptyDraft: NoteDraft = { text: '', audience: defaultNoteAudience }
/** The planner refuses its first two creates, so failure, target switches and Retry stay reachable. */
const plannerRefusals = 2

/**
 * UX variant A — inline quick composer. The detail pane keeps its notes in place; Add note opens
 * the composer under the list, Enter adds, and focus returns to Add note. Each agent keeps its
 * own draft, audience and open state, so switching agents never moves text to another target.
 * Escape closes and keeps the draft; only Discard draft clears it.
 */
function InlineQuickComposerStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  const [notes, setNotes] = React.useState<Readonly<Record<Target, readonly SubjectNote[]>>>({ [target]: [diffNote, agentNote], [plannerTarget]: [plannerNote] })
  const [selected, setSelected] = React.useState<Target>(target)
  const [open, setOpen] = React.useState<Readonly<Partial<Record<Target, boolean>>>>({})
  const [drafts, setDrafts] = React.useState<Readonly<Partial<Record<Target, NoteDraft>>>>({})
  // Opening focuses the composer and closing focuses Add note; switching agents keeps focus on the switcher.
  const [focusOnMount, setFocusOnMount] = React.useState(false)
  // Persistent announcement: the composer's own success status unmounts when it closes.
  const [announcement, setAnnouncement] = React.useState('')
  const refusals = React.useRef(plannerRefusals)
  const composing = open[selected] === true
  const closeOn = (to: Target) => {
    setFocusOnMount(true)
    setOpen(current => ({ ...current, [to]: false }))
  }
  const submitTo = (to: Target) => async (text: string, audience: NoteAudienceChoice) => {
    await Promise.resolve()
    if (to === plannerTarget && refusals.current > 0) {
      refusals.current -= 1
      throw new Error('synthetic create refusal')
    }
    // Negative control: the host never publishes the created note.
    if (!negativeControl) setNotes(current => ({ ...current, [to]: [publishedNote(text, audience, to), ...current[to]] }))
    setDrafts(current => ({ ...current, [to]: emptyDraft }))
    setAnnouncement(`Note added to ${to}.`)
    closeOn(to)
  }
  return <NotesFrame scheme={scheme} width={width} title="Inline quick composer">
    <ul aria-label="Agents" {...stylex.props(styles.agents)}>
      {targets.map(ref => <li key={ref}>
        <AriaButton aria-pressed={ref === selected} data-testid={`inline-agent-${ref}`} onPress={() => { setFocusOnMount(false); setSelected(ref) }} {...stylex.props(styles.agent, ref === selected && styles.agentSelected)}>
          {ref}{(drafts[ref]?.text ?? '') !== '' && <span {...stylex.props(styles.draftCue)}> · draft</span>}
        </AriaButton>
      </li>)}
    </ul>
    <article aria-label={`Agent detail: ${selected}`} {...stylex.props(styles.pane)}>
      <header {...stylex.props(styles.header)}>
        <span {...stylex.props(styles.name)}>{selected}</span>
        <NoteBadge notes={notes[selected]} />
      </header>
      <section aria-label="Notes" {...stylex.props(frame.section)}>
        <h2 {...stylex.props(frame.heading)}>Notes</h2>
        <NoteList notes={notes[selected]} author={viewer} />
        {composing
          ? <NoteComposer
            key={selected}
            target={selected}
            draft={drafts[selected] ?? emptyDraft}
            onDraftChange={draft => setDrafts(current => ({ ...current, [selected]: draft }))}
            onSubmit={submitTo(selected)}
            onClose={() => closeOn(selected)}
            autoFocus={focusOnMount}
          />
          : <Button variant="outline" size="md" autoFocus={focusOnMount} onPress={() => { setFocusOnMount(true); setOpen(current => ({ ...current, [selected]: true })) }} data-testid="inline-add-note"><Icon name="plus" />Add note</Button>}
        <p role="status" aria-live="polite" aria-atomic="true" data-testid="inline-status" {...stylex.props(frame.heading)}>{announcement}</p>
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

const agentLabel = 'Shared with the agent (people can see it too)'

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const input = () => canvas.getByTestId('note-composer-input')
  const composer = () => canvas.getByTestId('note-composer')
  const addNote = () => canvas.getByTestId('inline-add-note')
  const badge = canvas.getByTestId('note-badge')
  await expectVisibleAgentMarker(badge)
  await expectVisibleAgentMarker(canvas.getAllByTestId('note-row')[0]!)
  await expect(badge).toHaveAttribute('data-note-count', '2')

  // One click, the text, one confirm; the host status outlives the composer.
  await userEvent.click(addNote())
  await waitFor(() => expect(input()).toHaveFocus())
  await expect(composer()).toHaveTextContent(`Target: ${target}.`)
  await userEvent.keyboard('Hand off before the evening run{Enter}')
  await waitFor(() => expect(canvas.queryByTestId('note-composer')).toBeNull())
  const rows = canvas.getAllByTestId('note-row')
  await expect(rows).toHaveLength(3)
  await expect(within(rows[0]!).getByTestId('note-text')).toHaveTextContent('Hand off before the evening run')
  await expect(within(rows[0]!).getByTestId('note-audience')).toHaveTextContent('Only me')
  await expect(canvas.getByTestId('note-badge')).toHaveAttribute('data-note-count', '3')
  await expect(canvas.getByTestId('inline-status')).toHaveTextContent(`Note added to ${target}.`)
  await waitFor(() => expect(addNote()).toHaveFocus())

  // Multiline: Shift+Enter and pasted newlines survive. Tab reaches the audience control without changing it.
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(input()).toHaveFocus())
  await userEvent.keyboard('Context:{Shift>}{Enter}{/Shift}')
  await userEvent.paste('first line\nsecond line')
  const multiline = 'Context:\nfirst line\nsecond line'
  await expect(input()).toHaveValue(multiline)
  await userEvent.tab()
  await waitFor(() => expect(canvas.getByTestId('note-audience-self')).toHaveFocus())
  await expect(composer()).toHaveAttribute('data-note-audience', 'self')
  await userEvent.keyboard('{ArrowRight}')
  await waitFor(() => expect(composer()).toHaveAttribute('data-note-audience', 'agent'))
  await expect(canvas.getByTestId('note-composer-status')).toHaveTextContent(`Audience: ${agentLabel}.`)
  await userEvent.keyboard(' ')
  await expect(composer()).toHaveAttribute('data-note-audience', 'agent')

  // Escape closes without sending or discarding; reopening restores text and audience.
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByTestId('note-composer')).toBeNull())
  await waitFor(() => expect(addNote()).toHaveFocus())
  await expect(canvas.getAllByTestId('note-row')).toHaveLength(3)
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(input()).toHaveFocus())
  await expect(input()).toHaveValue(multiline)
  await expect(composer()).toHaveAttribute('data-note-audience', 'agent')

  // Another agent starts its own private draft; a refusal keeps text, audience and target.
  await userEvent.click(canvas.getByTestId(`inline-agent-${plannerTarget}`))
  await expect(canvas.queryByTestId('note-composer')).toBeNull()
  await userEvent.click(addNote())
  await waitFor(() => expect(input()).toHaveFocus())
  await expect(input()).toHaveValue('')
  await expect(composer()).toHaveAttribute('data-note-audience', 'self')
  await expect(composer()).toHaveTextContent(`Target: ${plannerTarget}.`)
  await userEvent.keyboard('Budget{Enter}')
  await expect(await canvas.findByRole('alert')).toHaveTextContent("Couldn't add the note. Your text, audience and target are kept.")
  await expect(input()).toHaveValue('Budget')

  // Switching back and forth keeps each agent's open composer and draft on its own target.
  await userEvent.click(canvas.getByTestId(`inline-agent-${target}`))
  await expect(input()).toHaveValue(multiline)
  await expect(composer()).toHaveAttribute('data-note-audience', 'agent')
  await expect(composer()).toHaveTextContent(`Target: ${target}.`)
  await userEvent.click(canvas.getByTestId(`inline-agent-${plannerTarget}`))
  await expect(input()).toHaveValue('Budget')
  await expect(composer()).toHaveTextContent(`Target: ${plannerTarget}.`)

  // A second refusal offers Retry, which adds the note and returns focus to Add note.
  await userEvent.click(input())
  await userEvent.keyboard('{Enter}')
  await canvas.findByRole('alert')
  await userEvent.click(canvas.getByRole('button', { name: 'Retry' }))
  await waitFor(() => expect(canvas.queryByTestId('note-composer')).toBeNull())
  await expect(within(canvas.getAllByTestId('note-row')[0]!).getByTestId('note-text')).toHaveTextContent('Budget')
  await expect(canvas.getByTestId('note-badge')).toHaveAttribute('data-note-count', '2')
  await expect(canvas.getByTestId('inline-status')).toHaveTextContent(`Note added to ${plannerTarget}.`)
  await waitFor(() => expect(addNote()).toHaveFocus())

  // The reviewer's shared multiline draft is still there and adds with its newlines and audience.
  await userEvent.click(canvas.getByTestId(`inline-agent-${target}`))
  await userEvent.click(input())
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(canvas.queryByTestId('note-composer')).toBeNull())
  const shared = canvas.getAllByTestId('note-row')[0]!
  await expect(within(shared).getByTestId('note-text').textContent).toBe(multiline)
  await expect(within(shared).getByTestId('note-audience')).toHaveTextContent(agentLabel)
  await waitFor(() => expect(addNote()).toHaveFocus())

  // Only Discard draft clears a draft; the next one starts private.
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(input()).toHaveFocus())
  await expect(input()).toHaveValue('')
  await expect(composer()).toHaveAttribute('data-note-audience', 'self')
  await userEvent.keyboard('scrap{Escape}')
  await waitFor(() => expect(addNote()).toHaveFocus())
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(input()).toHaveValue('scrap'))
  await userEvent.click(canvas.getByTestId('note-composer-discard'))
  await expect(input()).toHaveValue('')
  await expect(canvas.getByTestId('note-composer-status')).toHaveTextContent('Draft discarded.')
  await userEvent.click(input())
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(addNote()).toHaveFocus())

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }

const styles = stylex.create({
  agents: { listStyle: 'none', margin: s.zero, padding: s.zero, display: 'flex', flexWrap: 'wrap', gap: s.xs, minWidth: 0 },
  agent: {
    minHeight: g.controlMd, boxSizing: 'border-box', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border,
    borderRadius: r.control, backgroundColor: surface.transparent, color: text.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading,
    overflowWrap: 'anywhere', textAlign: 'start', cursor: 'pointer',
    outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  agentSelected: { backgroundColor: surface.rowActive, borderColor: border.borderStrong, color: text.fg },
  draftCue: { color: text.fgMuted },
  pane: {
    display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0, padding: s.lg,
    backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.base,
  },
  header: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0, flexWrap: 'wrap' },
  name: { fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold, color: text.fg, minWidth: 0, overflowWrap: 'anywhere' },
})
