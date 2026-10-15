import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { NoteComposer, noteTextLimitBytes } from './NoteComposer'
import { NotesFrame, expectNoAudiencePresentation, expectVisibleAgentMarker, notesFrameArgTypes, notesFrameArgs, notesStoryStyles as styles, type NotesFrameArgs } from './notes-story-frame'
import { greenFindings } from '../assistant-ui/composition/no-green-assertion'

const disabledReason = 'Notes are unavailable while st reconnects.'

function NoteComposerStory({ scheme, width, negativeControl }: NotesFrameArgs) {
  const [submitted, setSubmitted] = React.useState<readonly string[]>([])
  // Negative control: a host that remounts the composer after a failure loses the draft.
  const [failingKey, setFailingKey] = React.useState(0)
  const refused = React.useRef(false)
  const record = async (text: string) => {
    const { promise, resolve } = Promise.withResolvers<void>()
    setTimeout(resolve, 150)
    await promise
    setSubmitted(current => [...current, text])
  }
  const refuse = async () => {
    const { promise, resolve } = Promise.withResolvers<void>()
    setTimeout(resolve, 150)
    await promise
    if (!refused.current) {
      refused.current = true
      if (negativeControl) setFailingKey(key => key + 1)
      throw new Error('synthetic create refusal')
    }
  }
  return <NotesFrame scheme={scheme} width={width} title="Note composer">
    <section aria-label="Person composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Person</h2>
      <NoteComposer onSubmit={record} autoFocus />
      <ul aria-label="Submitted notes" data-testid="submitted">
        {submitted.map((text, index) => <li key={index}>{text}</li>)}
      </ul>
    </section>
    <section aria-label="Failing composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Create refused</h2>
      <NoteComposer key={failingKey} onSubmit={refuse} target="mission/example" />
    </section>
    <section aria-label="Agent target composer" {...stylex.props(styles.section)}>
      <h2 {...stylex.props(styles.heading)}>Agent target</h2>
      <NoteComposer onSubmit={record} target="agent/example/reviewer" />
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
    composer: scope.getByTestId('note-composer'),
  }
}

const assertStates: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)

  // Plain text only: no audience choice, and the copy says every note reaches the agent.
  const personRegion = canvas.getByRole('region', { name: 'Person composer' })
  const person = parts(personRegion)
  await waitFor(() => expect(person.input).toHaveFocus())
  await expect(person.submit).toBeDisabled()
  await expect(within(personRegion).getByTestId('note-composer-limit')).toHaveTextContent('0 of 1024 bytes')
  const reach = within(personRegion).getByTestId('note-composer-reach')
  await expect(reach).toHaveTextContent('Notes reach the agent as context. People with access can read them too.')
  await expect(person.input).toHaveAccessibleDescription(expect.stringContaining('Notes reach the agent as context.'))
  await expectVisibleAgentMarker(reach)
  await expect(within(personRegion).queryAllByRole('radio')).toHaveLength(0)
  await expect(personRegion.querySelectorAll('[aria-pressed]')).toHaveLength(0)
  await userEvent.type(person.input, 'Wrap up and hand off')
  await userEvent.keyboard('{Shift>}{Enter}{/Shift}this session retires soon')
  await userEvent.paste('\nPasted paragraph\nSecond line')
  const exactText = 'Wrap up and hand off\nthis session retires soon\nPasted paragraph\nSecond line'
  await expect(person.input).toHaveValue(exactText)
  await expect(within(personRegion).getByTestId('note-composer-limit')).toHaveTextContent(`${new TextEncoder().encode(exactText).length} of 1024 bytes`)
  // Tab leaves the field for the submit action; there is no audience stop in between.
  await userEvent.tab()
  await expect(person.submit).toHaveFocus()
  await userEvent.click(person.input)
  await userEvent.keyboard('{Enter}')
  await expect(within(personRegion).getByRole('status')).toHaveTextContent('Adding note…')
  await expect(person.composer).toHaveAttribute('aria-busy', 'true')
  const submitted = canvas.getByTestId('submitted')
  await waitFor(() => expect(submitted.querySelectorAll('li')).toHaveLength(1))
  await expect(submitted.querySelector('li')?.textContent).toBe(exactText)
  await waitFor(() => expect(person.input).toHaveValue(''))
  await expect(within(personRegion).getByRole('status')).toHaveTextContent('Note added.')
  await expect(person.input).toHaveFocus()

  // Refused create: text and focus survive, and the error is announced.
  const failing = parts(canvas.getByRole('region', { name: 'Failing composer' }))
  await userEvent.click(failing.input)
  await userEvent.paste('  keep this draft\nexactly  ')
  await userEvent.click(failing.submit)
  const failingRegion = within(canvas.getByRole('region', { name: 'Failing composer' }))
  await expect(await failingRegion.findByRole('alert')).toHaveTextContent("Couldn't add the note. Your text and target are kept.")
  await expect(failingRegion.getByTestId('note-composer-error-detail')).toHaveTextContent('synthetic create refusal')
  const afterFailure = parts(canvas.getByRole('region', { name: 'Failing composer' }))
  await expect(afterFailure.input).toHaveValue('  keep this draft\nexactly  ')
  await expect(afterFailure.input).toHaveFocus()
  await expect(afterFailure.submit).toBeEnabled()
  await expect(failingRegion.getByTestId('note-composer-reach')).toHaveTextContent('Notes reach agents currently working on this mission. People with access can read them too.')
  await expect(afterFailure.submit).toHaveAccessibleName('Retry')
  await userEvent.click(afterFailure.submit)
  await waitFor(() => expect(afterFailure.input).toHaveValue(''))
  await expect(failingRegion.getByRole('status')).toHaveTextContent('Note added.')
  await userEvent.click(afterFailure.input)
  await userEvent.type(afterFailure.input, 'Explicit discard only')
  await userEvent.keyboard('{Escape}')
  await expect(afterFailure.input).toHaveValue('Explicit discard only')
  await userEvent.click(failingRegion.getByRole('button', { name: 'Discard draft' }))
  await expect(afterFailure.input).toHaveValue('')
  await expect(failingRegion.getByRole('status')).toHaveTextContent('Draft discarded.')

  // Agent target: the same plain composer, its target named, no audience presentation anywhere.
  const agentRegion = canvas.getByRole('region', { name: 'Agent target composer' })
  await expect(within(agentRegion).getByText('Target: agent/example/reviewer.')).toBeVisible()
  await expect(within(agentRegion).getByTestId('note-composer-reach')).toHaveTextContent('Notes reach the agent as context.')
  await expectNoAudiencePresentation(canvasElement)

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
