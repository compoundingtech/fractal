import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { NoteComposer, noteTextLimitBytes } from './NoteComposer'
import { audiencePresentation, type NoteAudienceChoice } from './audience'
import { NotesFrame, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as styles, type NotesFrameArgs } from './notes-story-frame'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'

const disabledReason = 'Notes are unavailable while st reconnects.'

function NoteComposerStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  const [submitted, setSubmitted] = React.useState<readonly (readonly [string, NoteAudienceChoice])[]>([])
  // Negative control: a host that remounts the composer after a failure loses the draft.
  const [failingKey, setFailingKey] = React.useState(0)
  const record = async (text: string, audience: NoteAudienceChoice) => {
    await Promise.resolve()
    setSubmitted(current => [...current, [text, audience]])
  }
  const refuse = async () => {
    await Promise.resolve()
    if (negativeControl) setFailingKey(key => key + 1)
    throw new Error('synthetic create refusal')
  }
  return <NotesFrame scheme={scheme} width={width} title="Note composer">
    <section aria-label="Person composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Person · default only me</h2>
      <NoteComposer onSubmit={record} autoFocus />
      <ul aria-label="Submitted notes" data-testid="submitted">
        {submitted.map(([text, audience], index) => <li key={index} data-audience={audience}>{audiencePresentation(audience).label}: {text}</li>)}
      </ul>
    </section>
    <section aria-label="Failing composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Create refused</h2>
      <NoteComposer key={failingKey} onSubmit={refuse} />
    </section>
    <section aria-label="Agent composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Agent author</h2>
      <NoteComposer onSubmit={record} agentAuthor />
    </section>
    <section aria-label="Unavailable composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Unavailable</h2>
      <NoteComposer onSubmit={record} disabledReason={disabledReason} />
    </section>
    <section aria-label="Long composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Over the byte limit</h2>
      <NoteComposer onSubmit={record} />
    </section>
  </NotesFrame>
}

const meta = {
  title: 'Fractal/Plugins/Notes/Note composer', component: NoteComposerStory, parameters: { layout: 'fullscreen' },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Remount the refused composer after its failure: the preserved-draft assertion must fail.'),
} satisfies Meta<typeof NoteComposerStory>
export default meta
type Story = StoryObj<typeof meta>

const parts = (region: HTMLElement) => {
  const scope = within(region)
  return {
    input: scope.getByTestId('note-composer-input') as HTMLTextAreaElement,
    submit: scope.getByTestId('note-composer-submit'),
    toggle: scope.getByTestId('note-audience-toggle'),
    selected: () => scope.getByTestId('note-audience-toggle').querySelector('[data-selected]')?.textContent,
    composer: scope.getByTestId('note-composer'),
  }
}

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)

  // Person: default self, Alt+A toggles, Enter adds once and resets the draft.
  const person = parts(canvas.getByRole('region', { name: 'Person composer' }))
  await waitFor(() => expect(person.input).toHaveFocus())
  await expect(person.selected()).toBe('Only me')
  await expect(person.submit).toBeDisabled()
  await expect(within(canvas.getByRole('region', { name: 'Person composer' })).getByTestId('note-composer-help')).toHaveTextContent(/switches audience/)
  await userEvent.type(person.input, 'Wrap up and hand off')
  await userEvent.keyboard('{Shift>}{Enter}{/Shift}this session retires soon')
  await userEvent.keyboard('{Alt>}a{/Alt}')
  await expect(person.selected()).toBe('Shared with agent')
  await userEvent.keyboard('{Alt>}a{/Alt}')
  await expect(person.selected()).toBe('Only me')
  await expectVisibleAgentMarker(person.toggle)
  const agentOption = within(person.toggle).getByTestId('note-audience-agent')
  // Selecting the agent option also focuses it; focus opens the kit Tooltip (hover is unchanged
  // kit-wide behavior, not asserted here).
  await userEvent.click(agentOption)
  const audienceTip = await canvas.findByRole('tooltip', undefined, { timeout: 5000 })
  await expect(audienceTip).toHaveTextContent('The agent reads this note at its next turn.')
  await expect(audienceTip).toHaveTextContent('switches audience.')
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(canvas.queryByRole('tooltip')).toBeNull(), { timeout: 5000 })
  await expect(person.selected()).toBe('Shared with agent')
  await expect(person.input).toHaveValue('Wrap up and hand off\nthis session retires soon')
  await userEvent.keyboard('{Enter}')
  const submitted = canvas.getByTestId('submitted')
  await waitFor(() => expect(submitted.querySelectorAll('li')).toHaveLength(1))
  await expect(submitted.querySelector('li')).toHaveAttribute('data-audience', 'agent')
  await expect(submitted.querySelector('li')!.textContent).toBe('Shared with agent: Wrap up and hand off\nthis session retires soon')
  await waitFor(() => expect(person.input).toHaveValue(''))
  await expect(person.selected()).toBe('Only me')
  await expect(person.input).toHaveFocus()

  // Refused create: text, audience and focus survive, and the error is announced.
  const failing = parts(canvas.getByRole('region', { name: 'Failing composer' }))
  await userEvent.click(failing.input)
  await userEvent.type(failing.input, 'keep this draft')
  await userEvent.keyboard('{Alt>}a{/Alt}')
  await userEvent.click(failing.submit)
  const failingRegion = within(canvas.getByRole('region', { name: 'Failing composer' }))
  await expect(await failingRegion.findByRole('alert')).toHaveTextContent("Couldn't add the note. Your text is kept; try again.")
  await expect(failingRegion.getByTestId('note-composer-error-detail')).toHaveTextContent('synthetic create refusal')
  const afterFailure = parts(canvas.getByRole('region', { name: 'Failing composer' }))
  await expect(afterFailure.input).toHaveValue('keep this draft')
  await expect(afterFailure.selected()).toBe('Shared with agent')
  await expect(afterFailure.input).toHaveFocus()
  await expect(afterFailure.submit).toBeEnabled()

  // Agent authors: agent audience fixed and explained; Alt+A is inert.
  const agentRegion = canvas.getByRole('region', { name: 'Agent composer' })
  const agent = parts(agentRegion)
  await expect(agent.selected()).toBe('Shared with agent')
  for (const button of agent.toggle.querySelectorAll('button')) await expect(button).toBeDisabled()
  await expect(within(agentRegion).getByTestId('note-composer-agent-only')).toHaveTextContent('Agents write notes for agents only')
  await userEvent.click(agent.input)
  await userEvent.keyboard('{Alt>}a{/Alt}')
  await expect(agent.selected()).toBe('Shared with agent')
  await expect(agent.composer).toHaveAttribute('data-note-audience', 'agent')

  // Unavailable: reason visible and linked, nothing submits.
  const unavailableRegion = canvas.getByRole('region', { name: 'Unavailable composer' })
  const unavailable = parts(unavailableRegion)
  await expect(within(unavailableRegion).getByTestId('note-composer-disabled-reason')).toHaveTextContent(disabledReason)
  await expect(unavailable.input).toHaveAccessibleDescription(expect.stringContaining(disabledReason))
  await expect(unavailable.input).toHaveAttribute('readonly')
  await expect(unavailable.submit).toBeDisabled()

  // Over the server byte limit: counted in UTF-8 bytes, explained, not submittable.
  const longRegion = canvas.getByRole('region', { name: 'Long composer' })
  const long = parts(longRegion)
  await userEvent.click(long.input)
  await userEvent.paste('é'.repeat(520))
  await expect(within(longRegion).getByTestId('note-composer-limit')).toHaveTextContent(`Too long by ${520 * 2 - noteTextLimitBytes} bytes`)
  await expect(long.submit).toBeDisabled()
  await expect(long.input).toHaveAttribute('aria-invalid', 'true')

  await expect(greenFindings(canvasElement)).toEqual([])
}

export const AllStates: Story = { play: assertStates }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: assertStates }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: assertStates }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: assertStates }
