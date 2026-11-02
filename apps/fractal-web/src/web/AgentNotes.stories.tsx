import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { SubjectNote } from '@fractal/client/schema'
import { Button } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls'
import { greenFindings } from '../../../../packages/fractal-ui/src/assistant-ui/composition/no-green-assertion'
import { NotesFrame, notesFrameArgs, notesFrameArgTypes, expectNoAudiencePresentation, type NotesFrameArgs } from '../../../../packages/fractal-ui/src/notes/notes-story-frame'
import { target, plannerTarget, retireNote, publishedNote, viewer } from '../../../../packages/fractal-ui/src/notes/notes-fixtures'
import { fixtureSource } from '../data/fixtureSource'
import { fixtureProjections } from '../fixtures/projections'
import { DataSourceProvider } from '../data/react'
import { observed, type Feed } from '../data/source'
import { AgentNotes, AgentNoteQuickAdd } from './AgentNotes'

function WebNotes({ scheme, width, negativeControl }: NotesFrameArgs) {
  const [selected, select] = React.useState(target)
  const [fixture] = React.useState(() => {
    const registry = AtomRegistry.make()
    const splitRegistry = AtomRegistry.make()
    const notes = Atom.make<Feed<readonly SubjectNote[]>>(observed({ value: [retireNote] })).pipe(Atom.keepAlive)
    let refusals = 1
    const source = { ...fixtureSource({ world: fixtureProjections }), notes, notesActor: Atom.make<string | undefined>(viewer), noteActions: {
      create: async (to: string, text: string) => {
        if (refusals-- > 0) throw new Error('Synthetic gateway refusal')
        const previous = registry.get(notes)
        registry.set(notes, observed({ value: [publishedNote(text, to), ...(previous._tag === 'Observed' ? previous.value : [])] }))
      },
      archive: async () => undefined,
    } }
    return { source, registry, splitRegistry }
  })
  return <NotesFrame scheme={scheme} width={width} title="Web Notes: detail and selected row">
    <DataSourceProvider source={fixture.source} registry={fixture.registry}>
      <nav aria-label="Agents">
        {[target, plannerTarget].map(ref => <Button key={ref} variant="outline" onPress={() => select(ref)} aria-pressed={selected === ref}>{ref}</Button>)}
        {/* Negative control: a separate host registry breaks the production shared-draft contract. */}
        {negativeControl
          ? <DataSourceProvider source={fixture.source} registry={fixture.splitRegistry}><AgentNoteQuickAdd key={selected} agentRef={selected} /></DataSourceProvider>
          : <AgentNoteQuickAdd key={selected} agentRef={selected} />}
      </nav>
      <AgentNotes agentRef={selected} />
    </DataSourceProvider>
  </NotesFrame>
}
const meta = {
  title: 'Fractal/Plugins/Notes/Web entry points', component: WebNotes,
  excludeStories: ['scenarioExemptions'],
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Real web note adapters share the data-source registry over synthetic note publication and a deterministic refused create.' } },
  args: notesFrameArgs,
  argTypes: notesFrameArgTypes('Split quick-add and detail registries: the shared-draft assertion must fail.'),
} satisfies Meta<typeof WebNotes>
export default meta
type Story = StoryObj<typeof meta>
export const scenarioExemptions: readonly string[] = ['AllStates:contract-fixture', 'Dark1440:contract-fixture', 'Light1440:contract-fixture', 'Light440:contract-fixture']

const play: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const page = within(canvasElement.ownerDocument.body)
  const detail = () => within(canvas.getByTestId('agent-notes')).getByTestId('note-composer-input')
  const trigger = () => canvas.getByRole('button', { name: `Add note to ${target}` })
  const dialog = () => page.getByRole('dialog', { name: `Add note to ${target}` })
  await userEvent.click(detail())
  await userEvent.type(detail(), 'Keep this draft')
  await userEvent.keyboard('n')
  await expect(page.queryByRole('dialog')).toBeNull()
  await expect(detail()).toHaveValue('Keep this draftn')
  trigger().dispatchEvent(new KeyboardEvent('keydown', { key: 'n', bubbles: true, cancelable: true, isComposing: true }))
  await expect(page.queryByRole('dialog')).toBeNull()
  await userEvent.click(trigger())
  await expect(within(dialog()).getByTestId('note-composer-input')).toHaveValue('Keep this draftn')
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(trigger()).toHaveFocus())
  await userEvent.keyboard('n')
  await expect(within(dialog()).getByTestId('note-composer-input')).toHaveValue('Keep this draftn')
  await userEvent.click(within(dialog()).getByRole('button', { name: 'Add note' }))
  await expect(await within(dialog()).findByRole('alert')).toHaveTextContent('Your text and target are kept')
  await expect(within(dialog()).getByTestId('note-composer-input')).toHaveValue('Keep this draftn')
  await userEvent.click(within(dialog()).getByRole('button', { name: 'Retry' }))
  await waitFor(() => expect(page.queryByRole('dialog')).toBeNull())
  await waitFor(() => expect(detail()).toHaveValue(''))
  await userEvent.click(canvasElement.querySelector('summary')!)
  await expect(canvas.getByTestId('note-session')).toHaveTextContent('This session')
  await expect(canvas.getByTestId('note-list')).toHaveTextContent('Keep this draftn')
  await userEvent.click(detail())
  await userEvent.type(detail(), 'Reviewer draft')
  await userEvent.click(canvas.getByRole('button', { name: plannerTarget }))
  await expect(detail()).toHaveValue('')
  await userEvent.click(detail())
  await userEvent.type(detail(), 'Planner draft')
  await userEvent.click(canvas.getByRole('button', { name: target }))
  await expect(detail()).toHaveValue('Reviewer draft')
  await userEvent.click(trigger())
  await expect(within(dialog()).getByTestId('note-composer-input')).toHaveValue('Reviewer draft')
  await userEvent.click(within(dialog()).getByRole('switch'))
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(trigger()).toHaveFocus())
  await userEvent.keyboard('n')
  await expect(page.queryByRole('dialog')).toBeNull()
  await expectNoAudiencePresentation(canvasElement)
  await expect(greenFindings(canvasElement)).toEqual([])
}
export const AllStates: Story = { play }
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play }
