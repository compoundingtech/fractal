import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, within } from 'storybook/test'
import { QuestionForm, QuestionSummary, NeedsYouBar, NeedsYouInbox } from './assistant-ui/QuestionForm'
import { EmbraceToolItem } from './assistant-ui/EmbraceToolCall'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { darkTheme } from './assistant-ui/embrace-theme'
import { tokens } from './assistant-ui/embrace-tokens.stylex'
import { questionKinds, questionRequest, questionStates } from './question-fixtures'

const meta = { title: 'Fractal UI/Questions', component: QuestionForm, parameters: { layout: 'fullscreen' }, args: { request: questionRequest, onOpenTerminal: fn(), onDecline: fn(), onSubmit: fn(), onDraftChange: fn(), composerTyping: true }, decorators: [(Story, context) => <main data-scheme={context.parameters.scheme ?? 'dark'} {...stylex.props(styles.root, ...baselineTheme, context.parameters.scheme === 'light' ? lightTheme : darkTheme, context.parameters.width === 1440 ? styles.wide : styles.narrow)}><Story /></main>] } satisfies Meta<typeof QuestionForm>
export default meta
type Story = StoryObj<typeof meta>
export const ReadOnly: Story = { play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.queryByRole('button', { name: 'Submit' })).toBeNull()
  await expect(canvas.getByText('delivered after the question is answered')).toBeVisible()
  await userEvent.click(canvas.getByRole('button', { name: 'Open terminal' }))
  await expect(args.onOpenTerminal).toHaveBeenCalledTimes(1)
  await expect(args.onDecline).not.toHaveBeenCalled()
} }
export const Keyboard: Story = { args: { request: { ...questionRequest, answerable: true }, draft: { single: { selected: ['thread'], note: 'Keep this draft' } } }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvas.getByRole('option', { name: /This thread/ }))
  await userEvent.keyboard('{Escape}')
  await expect(args.onDecline).not.toHaveBeenCalled()
  await expect(canvas.getByRole('option', { name: /This thread/ })).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByRole('button', { name: 'Submit' })).toBeEnabled()
  await expect(canvas.getByRole('button', { name: 'Skip · ends the turn' })).toBeVisible()
  await userEvent.keyboard('2')
  await expect(args.onDraftChange).toHaveBeenCalledWith({ single: { selected: ['inbox'], note: 'Keep this draft', other: undefined } })
  await userEvent.keyboard('{Enter}')
  await expect(args.onDecline).not.toHaveBeenCalled()
} }
export const AllStates: Story = { render: () => <>
  {questionKinds.map(kind => <section key={kind.id}><h2>{kind.id}</h2><QuestionForm request={{ ...questionRequest, id: `readonly-${kind.id}`, questions: [kind] }} onOpenTerminal={fn()} />{questionStates.map((state, index) => <QuestionForm key={index} request={{ ...questionRequest, id: `${kind.id}-${index}`, questions: [kind], answerable: true, state }} draft={{ [kind.id]: { selected: [kind.options[0]?.id ?? ''], other: kind.kind === 'text' ? 'Release review' : undefined } }} onOpenTerminal={fn()} onSubmit={fn()} onDraftChange={fn()} onDecline={fn()} onRetry={fn()} />)}</section>)}
  <QuestionForm request={{ ...questionRequest, questions: questionKinds, answerable: true }} onDraftChange={fn()} onSubmit={fn()} onDecline={fn()} />
  <QuestionSummary request={{ ...questionRequest, questions: [questionKinds[0]!, questionKinds[1]!], state: { _tag: 'answered', answers: { multi: { selected: [] } }, by: 'Johannes', where: 'terminal', at: '15:41 UTC' } }} />
  <NeedsYouBar requests={[questionRequest, { ...questionRequest, id: 'second', agentLabel: 'atlas' }, { ...questionRequest, id: 'third', agentLabel: 'lyra' }]} expanded={false} onToggle={fn()} />
  <NeedsYouBar requests={[questionRequest]} expanded onToggle={fn()}><QuestionForm request={questionRequest} variant="pinned" onOpenTerminal={fn()} /></NeedsYouBar>
  <NeedsYouInbox requests={[questionRequest, { ...questionRequest, id: 'second', agentLabel: 'atlas' }, { ...questionRequest, id: 'third', agentLabel: 'lyra' }]} onShowInThread={fn()} onOpenTerminal={fn()} />
</> }
export const NegativeControls: Story = { render: () => <>
  <EmbraceToolItem item={{ _tag: 'ToolCall', id: 'decision-1', callId: 'decision-1', name: 'decision', input: { description: 'Review recorded' }, status: 'success', callSeen: true, at: '2026-10-11T15:40:00Z' }} variant="cards" />
  <NeedsYouBar requests={[]} expanded={false} onToggle={fn()} />
  <NeedsYouInbox requests={[]} onShowInThread={fn()} onOpenTerminal={fn()} />
  <NeedsYouBar requests={[questionRequest]} transcriptVisibility={0.6} expanded={false} onToggle={fn()} />
  <QuestionSummary request={{ ...questionRequest, questions: [questionKinds[0]!, questionKinds[1]!], state: { _tag: 'answered', answers: { multi: { selected: [] } }, by: 'Johannes', at: '15:41 UTC' } }} />
</>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.queryByRole('button', { name: 'Open terminal' })).toBeNull()
  await expect(canvas.queryByRole('complementary', { name: 'Needs you' })).toBeNull()
  await expect(canvas.getByText('none')).toBeVisible()
  await expect(canvas.getByText('Left open')).toBeVisible()
  await expect(canvas.getByRole('region', { name: 'Needs you inbox' }).childElementCount).toBe(0)
} }
export const AllStatesLight440: Story = { ...AllStates, parameters: { scheme: 'light', width: 440 } }
export const AllStatesDark1440: Story = { ...AllStates, parameters: { scheme: 'dark', width: 1440 } }
export const AllStatesLight1440: Story = { ...AllStates, parameters: { scheme: 'light', width: 1440 } }
export const ReadOnlyLight440: Story = { ...ReadOnly, parameters: { scheme: 'light', width: 440 } }
export const ReadOnlyDark1440: Story = { ...ReadOnly, parameters: { scheme: 'dark', width: 1440 } }
export const ReadOnlyLight1440: Story = { ...ReadOnly, parameters: { scheme: 'light', width: 1440 } }
export const KeyboardLight440: Story = { ...Keyboard, parameters: { scheme: 'light', width: 440 } }
export const KeyboardDark1440: Story = { ...Keyboard, parameters: { scheme: 'dark', width: 1440 } }
export const KeyboardLight1440: Story = { ...Keyboard, parameters: { scheme: 'light', width: 1440 } }
export const NegativeControlsLight440: Story = { ...NegativeControls, parameters: { scheme: 'light', width: 440 } }
export const NegativeControlsDark1440: Story = { ...NegativeControls, parameters: { scheme: 'dark', width: 1440 } }
export const Pending: Story = { args: { request: { ...questionRequest, answerable: true } } }
export const PendingTimeout: Story = { args: { request: { ...questionRequest, answerable: true, state: questionStates[1]! } } }
export const Submitting: Story = { args: { request: { ...questionRequest, answerable: true, state: { _tag: 'submitting' } }, draft: { single: { selected: ['thread'] } } } }
export const Failed: Story = { args: { request: { ...questionRequest, answerable: true, state: questionStates[3]! }, draft: { single: { selected: ['thread'], note: 'Retain this note on retry' } }, onRetry: fn() }, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('option', { name: /This thread/ })).toHaveAttribute('aria-selected', 'true')
  await expect(canvas.getByRole('textbox', { name: 'Note to the agent' })).toHaveValue('Retain this note on retry')
  await userEvent.click(canvas.getByRole('button', { name: 'Retry' }))
  await expect(args.onRetry).toHaveBeenCalledTimes(1)
  await expect(args.onDecline).not.toHaveBeenCalled()
} }
export const Answered: Story = { args: { request: { ...questionRequest, state: questionStates[4]! } } }
export const AnsweredElsewhere: Story = { args: { request: { ...questionRequest, state: questionStates[5]! }, draft: { single: { selected: ['thread'], other: 'Unsent draft must not survive' } } }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText(/your unsent web draft was discarded/)).toBeVisible()
  await expect(canvas.queryByText(/Unsent draft must not survive/)).toBeNull()
  await expect(canvas.queryByRole('button', { name: 'Submit' })).toBeNull()
} }
export const PartiallyAnswered: Story = { args: { request: { ...questionRequest, questions: [questionKinds[0]!, questionKinds[1]!], state: { _tag: 'answered', answers: { single: { selected: ['thread'] } }, by: 'Johannes', at: '15:41 UTC' } } } }
export const TimedOutAuto: Story = { args: { request: { ...questionRequest, state: questionStates[6]! } } }
export const TimedOutNone: Story = { args: { request: { ...questionRequest, state: questionStates[7]! } } }
export const Cancelled: Story = { args: { request: { ...questionRequest, state: questionStates[8]! } } }
export const ChatRedirected: Story = { args: { request: { ...questionRequest, state: questionStates[9]! } } }
export const NegativeControlsLight1440: Story = { ...NegativeControls, parameters: { scheme: 'light', width: 1440 } }
const styles = stylex.create({ root: { minHeight: '100vh', boxSizing: 'border-box', padding: 16, display: 'flex', flexDirection: 'column', gap: 16, backgroundColor: tokens.canvas, color: tokens.ink, fontFamily: 'sans-serif' }, narrow: { width: 440, maxWidth: '100%' }, wide: { width: 1440, maxWidth: '100%' } })
