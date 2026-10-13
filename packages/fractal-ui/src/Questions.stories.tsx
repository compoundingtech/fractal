import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { QuestionDock, QuestionDockGroup } from '../../../apps/fractal-web/src/web/questions/QuestionDock.tsx'
import { questionScenarios, viewedAt, submittedAt } from './assistant-ui/questions/fixtures'
import type { QuestionRequestView } from './assistant-ui/questions/model'
import { EmbraceComposer } from './assistant-ui/EmbraceComposer'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { Transcript } from './assistant-ui/composition/Transcript'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { TerminalDrawer } from './assistant-ui/composition/Shell'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars, textVars, typeVars } from './assistant-ui/composition-tokens.stylex'

type Phase = 'pending' | 'arrival' | 'readonly' | 'answered'
interface Args { scheme: Scheme; phase: Phase; multiple: boolean; drawer: boolean }
const requestFor = (phase: Phase): QuestionRequestView => ({
  ...questionScenarios.multi,
  ...(phase === 'readonly' ? { answerable: false, readOnlyReason: 'Answer in the agent terminal. Web answering is not available on this connection.' } : {}),
  ...(phase === 'answered' ? { state: { _tag: 'answered', by: 'You', at: submittedAt, answers: { checks: { selected: ['typecheck'], note: 'Run the narrow checks first.' } } } } : {}),
})
/** Real dock, transcript, runtime and composer. Settlement is explicitly story-only, not a live transport. */
function QuestionsStory({ scheme, phase, multiple, drawer }: Args) {
  const [request, setRequest] = React.useState(() => requestFor(phase))
  const [arrived, setArrived] = React.useState(phase !== 'arrival')
  const options = React.useMemo(() => ({ messages: [], isRunning: false, onNew: async () => {} }), [])
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div data-testid="question-conversation-pane" style={{ flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
    <EmbraceRuntimeProvider options={options}>
      <div {...stylex.props(styles.history)}><Transcript title="Sample agent" turns={[]} sync={{ _tag: 'Live', since: viewedAt - 1000 }} now={viewedAt} observedAt={viewedAt} /></div>
      {arrived && <QuestionDockGroup>
        <QuestionDock request={request} now={viewedAt} onSubmit={answers => setRequest(current => ({ ...current, state: { _tag: 'answered', by: 'You', at: submittedAt, answers } }))} />
        {multiple && <QuestionDock request={{ ...requestFor('readonly'), id: 'second-ask' }} now={viewedAt} />}
      </QuestionDockGroup>}
      <div data-testid="conversation-composer-dock" onInputCapture={() => setArrived(true)} {...stylex.props(styles.composer)}><EmbraceComposer variant="C1" readingColumn /></div>
    </EmbraceRuntimeProvider>
    </div>
    <TerminalDrawer open={drawer} height={440} onHeight={() => {}} lines={['$ pnpm typecheck', 'Typecheck completed. The agent is waiting for your answer.']} />
  </main>
}
const meta = { title: 'Fractal/Plugins/Questions', component: QuestionsStory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', phase: 'pending', multiple: false, drawer: false }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, phase: { options: ['pending', 'arrival', 'readonly', 'answered'], control: 'radio' } } } satisfies Meta<typeof QuestionsStory>
export default meta
type Story = StoryObj<typeof meta>
export const Pending: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('checkbox', { name: /Typecheck/ })).toBeVisible()
  await expect(canvas.queryByText('Open question')).toBeNull()
  const dock = canvas.getByTestId('question-dock').getBoundingClientRect()
  await expect(dock.top).toBeGreaterThanOrEqual(0)
  await expect(dock.bottom).toBeLessThanOrEqual(window.innerHeight)
} }
export const Submit: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvas.getByRole('checkbox', { name: /Typecheck/ }))
  await userEvent.click(canvas.getByRole('button', { name: /^Submit$/ }))
  await expect(canvas.getByText('Submitted by You.')).toBeVisible()
  await expect(canvas.queryByRole('checkbox')).toBeNull()
} }
export const ComposerFocus: Story = { args: { phase: 'arrival' }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const composer = canvas.getByRole('textbox')
  await userEvent.click(composer)
  await userEvent.type(composer, 'Keep this draft while the question arrives')
  await expect(composer).toHaveFocus()
  await expect(canvas.getByRole('checkbox', { name: /Typecheck/ })).toBeVisible()
} }
export const ReadOnly: Story = { args: { phase: 'readonly' }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('Read only', { exact: true })).toBeVisible()
  await expect(canvas.queryByRole('textbox', { name: 'Note' })).toBeNull()
  await expect(canvas.queryByRole('button', { name: /^Submit$/ })).toBeNull()
  await expect(canvas.getByRole('checkbox', { name: /Typecheck/ })).toBeVisible()
} }
export const Answered: Story = { args: { phase: 'answered' }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('Submitted by You.')).toBeVisible()
  await expect(canvas.queryByRole('checkbox')).toBeNull()
  await expect(canvas.getByText(/Run the narrow checks first/)).toBeVisible()
} }
export const MultipleWithTerminalDrawer: Story = { args: { multiple: true, drawer: true }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const pane = canvas.getByTestId('question-conversation-pane').getBoundingClientRect()
  const group = canvas.getByTestId('question-dock-group').getBoundingClientRect()
  const composer = canvas.getByTestId('conversation-composer-dock').getBoundingClientRect()
  await expect(canvas.getAllByTestId('question-dock')).toHaveLength(2)
  await expect(group.height).toBeLessThanOrEqual(pane.height * 0.45 + 1)
  await expect(composer.bottom).toBeLessThanOrEqual(pane.bottom + 1)
  await expect(canvas.getByRole('textbox', { name: 'Message' })).toBeVisible()
} }
export const AllStates: Story = { render: args => <div>{(['pending', 'readonly', 'answered'] as const).map(phase => <section key={phase} aria-label={phase}><QuestionsStory {...args} phase={phase} /></section>)}</div> }
const styles = stylex.create({
  root: { height: '100vh', display: 'flex', flexDirection: 'column', minWidth: 0, backgroundColor: surfaceVars.canvas, color: textVars.fg, fontFamily: typeVars.fontSans },
  history: { flex: '1 1 0', minHeight: 0, overflow: 'hidden' },
  composer: { flexShrink: 0, paddingBottom: 16 },
})
