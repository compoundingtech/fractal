import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import axe from 'axe-core'
import { QuestionForm, QuestionSummary, NeedsYouBar, NeedsYouInbox } from './assistant-ui/QuestionForm'
import type { Answers, QuestionFormProps } from './assistant-ui/QuestionForm'
import { Composer } from './assistant-ui/composition/Composer'
import { EmbraceRuntimeProvider, type ConversationRuntimeOptions } from './assistant-ui/EmbraceRuntime'
import { EmbraceToolItem } from './assistant-ui/EmbraceToolCall'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t, spaceVars as s, geometryVars as g } from './assistant-ui/composition-tokens.stylex'
import { questionKinds, questionRequest, questionStates } from './question-fixtures'

function ComposerRoutingExample({ routing, storyId }: { readonly routing: NonNullable<QuestionFormProps['composerRouting']>; readonly storyId: string }) {
  const [sent, setSent] = React.useState<readonly string[]>([])
  const options = React.useMemo<ConversationRuntimeOptions>(() => ({ messages: [], onNew: async message => setSent(previous => [...previous, message.content.filter(part => part.type === 'text').map(part => part.text).join('\n')]) }), [])
  return <EmbraceRuntimeProvider options={options}>
    <QuestionForm request={questionRequest} composerTyping composerRouting={routing} onOpenTerminal={fn()} />
    <Composer agent={questionRequest.agentLabel} draftKey={`question-routing.${storyId}.${routing._tag}`} onSend={text => setSent(previous => [...previous, text])} />
    <output aria-label="Composer fixture submissions">{sent.join('\n')}</output>
  </EmbraceRuntimeProvider>
}

function SharedQuestionDraft({ scrollDriven = false }: { readonly scrollDriven?: boolean }) {
  const [drafts, setDrafts] = React.useState<Readonly<Record<string, Answers>>>({})
  const [expanded, setExpanded] = React.useState(true)
  const [ratio, setRatio] = React.useState(0)
  const scroll = React.useRef<HTMLDivElement>(null)
  const observe = React.useCallback((node: HTMLDivElement | null) => {
    if (node === null || !scrollDriven) return
    const observer = new IntersectionObserver(entries => setRatio(entries[0]?.intersectionRatio ?? 0), { root: scroll.current, threshold: [0, 0.6, 1] })
    observer.observe(node)
    return () => observer.disconnect()
  }, [scrollDriven])
  const request = { ...questionRequest, answerable: true }
  const shared = { draft: drafts[request.id], onDraftChange: (draft: Answers) => setDrafts(previous => ({ ...previous, [request.id]: draft })) }
  return <>
    <div ref={scroll} data-question-scroll {...stylex.props(styles.scroll)}>
      <div ref={observe}><QuestionForm request={request} {...shared} /></div>
      {scrollDriven ? <div {...stylex.props(styles.scrollSpace)}>Earlier conversation</div> : null}
    </div>
    <NeedsYouBar requests={[request]} expanded={expanded} onToggle={() => setExpanded(value => !value)} transcriptVisibility={scrollDriven ? ratio : 0}><QuestionForm request={request} variant="pinned" {...shared} /></NeedsYouBar>
    {!scrollDriven ? <NeedsYouInbox requests={[request]} onShowInThread={fn()} onOpenTerminal={fn()} formProps={() => shared} /> : null}
  </>
}

const meta = {
  title: 'Fractal UI/Questions',
  component: QuestionForm,
  parameters: { layout: 'fullscreen' },
  args: { request: questionRequest, onOpenTerminal: fn(), onDecline: fn(), onSubmit: fn(), onDraftChange: fn(), composerTyping: true },
  render: function ControlledQuestion(args) {
    const [draft, setDraft] = React.useState(args.draft)
    return <QuestionForm {...args} draft={draft} onDraftChange={next => { setDraft(next); args.onDraftChange?.(next) }} />
  },
  decorators: [(Story, context) => <main data-scheme={context.parameters.scheme ?? 'dark'} {...stylex.props(styles.root, ...baselineTheme, context.parameters.scheme === 'light' && lightTheme, context.parameters.width === 1440 ? styles.wide : styles.narrow)}><Story /></main>],
} satisfies Meta<typeof QuestionForm>
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
  await userEvent.click(canvas.getByRole('radio', { name: /This thread/ }))
  await userEvent.keyboard('{Escape}')
  await expect(args.onDecline).not.toHaveBeenCalled()
  await expect(canvas.getByRole('radio', { name: /This thread/ })).toBeChecked()
  const contrast = await axe.run(canvasElement, { runOnly: { type: 'rule', values: ['color-contrast'] } })
  await expect(contrast.violations).toEqual([])
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
  <QuestionSummary request={{ ...questionRequest, questions: [questionKinds[0]!, questionKinds[1]!], state: { _tag: 'answered', answers: { multi: { selected: [] } }, by: 'Reviewer', where: 'terminal', at: '15:41 UTC' } }} />
  <NeedsYouBar requests={[questionRequest, { ...questionRequest, id: 'second', agentLabel: 'atlas' }, { ...questionRequest, id: 'third', agentLabel: 'lyra' }]} expanded={false} onToggle={fn()} />
  <NeedsYouBar requests={[questionRequest]} expanded onToggle={fn()}><QuestionForm request={questionRequest} variant="pinned" onOpenTerminal={fn()} /></NeedsYouBar>
  <NeedsYouInbox requests={[questionRequest, { ...questionRequest, id: 'second', agentLabel: 'atlas' }, { ...questionRequest, id: 'third', agentLabel: 'lyra' }]} onShowInThread={fn()} onOpenTerminal={fn()} />
</> }
export const NegativeControls: Story = { render: () => <>
  <EmbraceToolItem item={{ _tag: 'ToolCall', id: 'decision-1', callId: 'decision-1', name: 'decision', input: { description: 'Review recorded' }, status: 'success', callSeen: true, at: '2026-10-11T15:40:00Z' }} variant="cards" />
  <NeedsYouBar requests={[]} expanded={false} onToggle={fn()} />
  <NeedsYouInbox requests={[]} onShowInThread={fn()} onOpenTerminal={fn()} />
  <NeedsYouBar requests={[questionRequest]} transcriptVisibility={0.6} expanded={false} onToggle={fn()} />
  <QuestionSummary request={{ ...questionRequest, questions: [questionKinds[0]!, questionKinds[1]!], state: { _tag: 'answered', answers: { multi: { selected: [] } }, by: 'Reviewer', at: '15:41 UTC' } }} />
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
  await expect(canvas.getByRole('radio', { name: /This thread/ })).toBeChecked()
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
export const PartiallyAnswered: Story = { args: { request: { ...questionRequest, questions: [questionKinds[0]!, questionKinds[1]!], state: { _tag: 'answered', answers: { single: { selected: ['thread'] } }, by: 'Reviewer', at: '15:41 UTC' } } } }
export const TimedOutAuto: Story = { args: { request: { ...questionRequest, state: questionStates[6]! } } }
export const TimedOutNone: Story = { args: { request: { ...questionRequest, state: questionStates[7]! } } }
export const Cancelled: Story = { args: { request: { ...questionRequest, state: questionStates[8]! } } }
export const ChatRedirected: Story = { args: { request: { ...questionRequest, state: questionStates[9]! } } }
export const KeyboardWalk: Story = {
  args: { request: { ...questionRequest, answerable: true, questions: [questionKinds[0]!, questionKinds[1]!, questionKinds[2]!] }, autoFocus: true, draft: { single: { selected: ['thread'] } } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('radiogroup')).toHaveAttribute('aria-required', 'true')
    await userEvent.click(canvas.getByRole('radio', { name: /This thread/ }))
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(canvas.getByRole('checkbox', { name: 'Unit tests' })).toHaveFocus())
    await userEvent.keyboard('n')
    await waitFor(() => expect(canvas.getByRole('textbox', { name: 'Note to the agent' })).toHaveFocus())
    await userEvent.keyboard('{Escape}')
    await expect(canvas.getByRole('checkbox', { name: 'Unit tests' })).toHaveFocus()
    await userEvent.keyboard('3')
    await waitFor(() => expect(canvas.getByRole('textbox', { name: 'Your answer' })).toHaveFocus())
    await userEvent.type(canvas.getByRole('textbox', { name: 'Your answer' }), 'Accessibility audit')
    await userEvent.keyboard('{Escape}1')
    await expect(canvas.getByRole('textbox', { name: 'Your answer' })).toHaveValue('Accessibility audit')
    await expect(canvas.getByRole('checkbox', { name: 'Unit tests' })).toBeChecked()
    await userEvent.click(canvas.getByRole('button', { name: 'Next' }))
    await waitFor(() => expect(canvas.getByRole('textbox', { name: 'Your answer' })).toHaveFocus())
    await userEvent.type(canvas.getByRole('textbox', { name: 'Your answer' }), 'A release ready for review.')
    await userEvent.click(canvas.getByRole('button', { name: 'Next' }))
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Submit' })).toHaveFocus())
    await expect(canvas.getByRole('button', { name: 'Review' })).toHaveAttribute('aria-current', 'step')
    await expect(args.onDecline).not.toHaveBeenCalled()
    await expect(args.onSubmit).not.toHaveBeenCalled()
  },
}
export const ToolQuestions: Story = {
  render: args => <>
    <EmbraceToolItem item={{ _tag: 'ToolCall', id: 'singular', callId: 'singular', name: 'ask_user', input: { question: 'Choose a sample size.', options: ['Four rows', 'Eight rows'] }, status: 'running', callSeen: true, at: '2026-10-11T15:40:00Z' }} onOpenTerminal={args.onOpenTerminal} />
    <EmbraceToolItem item={{ _tag: 'ToolCall', id: 'settled-batch', callId: 'settled-batch', name: 'ask', input: { questions: [{ id: 'db', question: 'Which database?', options: [{ label: 'SQLite' }, { label: 'Postgres' }] }, { id: 'checks', question: 'Which checks?', options: [{ label: 'Unit' }, { label: 'Browser' }], multi: true }] }, result: { content: { details: { results: [{ id: 'db', selectedOptions: ['SQLite'], note: 'Keep it local' }, { id: 'checks', selectedOptions: ['Browser'], customInput: 'Accessibility', timedOut: true }] } }, isError: false, at: '2026-10-11T15:41:00Z' }, status: 'success', callSeen: true, at: '2026-10-11T15:40:00Z' }} />
    <EmbraceToolItem item={{ _tag: 'ToolCall', id: 'interrupted-singular', callId: 'interrupted-singular', name: 'ask_user', input: { question: 'Choose a sample size.', options: ['Four rows', 'Eight rows'] }, status: 'interrupted', callSeen: true, at: '2026-10-11T15:40:00Z' }} />
    <EmbraceToolItem item={{ _tag: 'ToolCall', id: 'settled-unrecorded', callId: 'settled-unrecorded', name: 'ask_user', input: { question: 'Choose an archived sample size.', options: ['Four rows', 'Eight rows'] }, status: 'success', callSeen: true, at: '2026-10-11T15:40:00Z' }} />
  </>,
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('radio', { name: 'Four rows' })).toBeVisible()
    await expect(canvas.getByRole('radio', { name: 'Eight rows' })).toBeVisible()
    await expect(canvas.queryByRole('button', { name: 'Submit' })).toBeNull()
    await expect(canvas.getAllByText('SQLite')[0]).toBeVisible()
    await expect(canvas.getByText(/Accessibility/)).toBeVisible()
    await expect(canvas.getByText(/auto-selected/)).toBeVisible()
    await expect(canvas.getByText('Interrupted')).toBeVisible()
    await expect(canvas.queryByText('Answer details were not recorded.')).toBeNull()
    await expect(canvas.queryByText('Question closed')).toBeNull()
    const original = within(canvas.getAllByRole('region', { name: 'Original question' })[1]!)
    await expect(original.getByText('Four rows')).toBeVisible()
    await expect(original.getByText('Eight rows')).toBeVisible()
    await expect(original.queryByText('Left open')).toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: 'Open terminal' }))
    await expect(args.onOpenTerminal).toHaveBeenCalledTimes(1)
    await expect(args.onSubmit).not.toHaveBeenCalled()
  },
}
export const KeyboardWalkLight440: Story = { ...KeyboardWalk, parameters: { scheme: 'light', width: 440 } }
export const KeyboardWalkDark1440: Story = { ...KeyboardWalk, parameters: { scheme: 'dark', width: 1440 } }
export const KeyboardWalkLight1440: Story = { ...KeyboardWalk, parameters: { scheme: 'light', width: 1440 } }
export const ToolQuestionsLight440: Story = { ...ToolQuestions, parameters: { scheme: 'light', width: 440 } }
export const ToolQuestionsDark1440: Story = { ...ToolQuestions, parameters: { scheme: 'dark', width: 1440 } }
export const ToolQuestionsLight1440: Story = { ...ToolQuestions, parameters: { scheme: 'light', width: 1440 } }
export const NegativeControlsLight1440: Story = { ...NegativeControls, parameters: { scheme: 'light', width: 1440 } }
export const SettledWithoutAnswerFacts: Story = {
  args: { request: { ...questionRequest, state: { _tag: 'unrecorded', reason: '' } } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('This thread')).toBeVisible()
    await expect(canvas.getByText('Team inbox')).toBeVisible()
    await expect(canvas.queryByText('Answer details were not recorded.')).toBeNull()
    await expect(canvas.queryByText('Question closed')).toBeNull()
    await expect(canvas.queryByText('Left open')).toBeNull()
  },
}
export const SettledWithoutAnswerFactsLight440: Story = { ...SettledWithoutAnswerFacts, parameters: { scheme: 'light', width: 440 } }
export const SettledWithoutAnswerFactsDark1440: Story = { ...SettledWithoutAnswerFacts, parameters: { scheme: 'dark', width: 1440 } }
export const SettledWithoutAnswerFactsLight1440: Story = { ...SettledWithoutAnswerFacts, parameters: { scheme: 'light', width: 1440 } }
export const ComposerRouting: Story = { render: (_args, context) => <><ComposerRoutingExample storyId={context.id} routing={{ _tag: 'HeldUntilAnswered' }} /><ComposerRoutingExample storyId={context.id} routing={{ _tag: 'ChatAboutQuestion' }} /></>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('delivered after the question is answered')).toBeVisible()
  await expect(canvas.getByText('Your message goes to the agent as a note about this question')).toBeVisible()
  const composers = canvas.getAllByRole('textbox').filter(element => element.closest('form') !== null)
  await expect(composers).toHaveLength(2)
  for (const composer of composers) await userEvent.clear(composer)
  await userEvent.type(composers[0]!, 'Please clarify the scope')
  await expect(composers[0]).toHaveValue('Please clarify the scope')
  await expect(composers[1]).not.toHaveValue('Please clarify the scope')
} }
export const SharedDraft: Story = { render: () => <SharedQuestionDraft />, play: async ({ canvasElement }) => {
  const cards = canvasElement.querySelectorAll('[data-question-variant]')
  await userEvent.click(within(cards[0] as HTMLElement).getByRole('radio', { name: /Team inbox/ }))
  await expect(within(cards[1] as HTMLElement).getByRole('radio', { name: /Team inbox/ })).toBeChecked()
  await expect(within(cards[1] as HTMLElement).getByRole('radio', { name: /This thread/ })).not.toBeChecked()
  const inbox = within(canvasElement).getByRole('region', { name: 'Needs you inbox' })
  await userEvent.click(inbox.querySelector('summary')!)
  await expect(within(inbox).getByRole('radio', { name: /Team inbox/ })).toBeChecked()
} }
export const ScrollAttention: Story = { render: () => <SharedQuestionDraft scrollDriven />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const scroll = canvasElement.querySelector<HTMLElement>('[data-question-scroll]')!
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: 'Needs you' })).toBeNull())
  scroll.scrollTo({ top: scroll.scrollHeight })
  await waitFor(() => expect(canvas.getByRole('complementary', { name: 'Needs you' })).toBeVisible())
  scroll.scrollTo({ top: 0 })
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: 'Needs you' })).toBeNull())
} }
export const ComposerRoutingLight440: Story = { ...ComposerRouting, parameters: { scheme: 'light', width: 440 } }
export const ComposerRoutingDark1440: Story = { ...ComposerRouting, parameters: { scheme: 'dark', width: 1440 } }
export const ComposerRoutingLight1440: Story = { ...ComposerRouting, parameters: { scheme: 'light', width: 1440 } }
export const SharedDraftLight440: Story = { ...SharedDraft, parameters: { scheme: 'light', width: 440 } }
export const SharedDraftDark1440: Story = { ...SharedDraft, parameters: { scheme: 'dark', width: 1440 } }
export const SharedDraftLight1440: Story = { ...SharedDraft, parameters: { scheme: 'light', width: 1440 } }
export const ScrollAttentionLight440: Story = { ...ScrollAttention, parameters: { scheme: 'light', width: 440 } }
export const ScrollAttentionDark1440: Story = { ...ScrollAttention, parameters: { scheme: 'dark', width: 1440 } }
export const ScrollAttentionLight1440: Story = { ...ScrollAttention, parameters: { scheme: 'light', width: 1440 } }
export const RelativeAge: Story = { args: { ageLabel: '2 minutes ago' }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText(/2 minutes ago/)).toBeVisible()
  await expect(canvas.queryByText(questionRequest.askedAt, { exact: false })).toBeNull()
} }
export const RelativeAgeLight440: Story = { ...RelativeAge, parameters: { scheme: 'light', width: 440 } }
export const RelativeAgeDark1440: Story = { ...RelativeAge, parameters: { scheme: 'dark', width: 1440 } }
export const RelativeAgeLight1440: Story = { ...RelativeAge, parameters: { scheme: 'light', width: 1440 } }
const styles = stylex.create({ root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.xl, display: 'flex', flexDirection: 'column', gap: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading }, narrow: { width: g.specimenNarrow, maxWidth: '100%' }, wide: { width: g.specimenWide, maxWidth: '100%' }, scroll: { maxHeight: g.transcriptFrameHeight, overflowY: 'auto', maxWidth: g.lane }, scrollSpace: { height: '200vh' } })
