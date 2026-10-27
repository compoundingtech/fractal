import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { Transcript, transcriptMountedWindow, type TranscriptHistory } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { SyncStatus } from './assistant-ui/sync-presentation/sync-status'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { scenarioSections, type TranscriptSection } from './scenario-transcript.story-harness'
import { expect, userEvent, waitFor } from 'storybook/test'
import { PressStory, initialTurns, ready, readerScrollsUp, settleFrames } from './transcript-press.story-harness'

type PressArgs = React.ComponentProps<typeof ScenarioConversationStory> & { conversation?: string }
const meta = { title: 'Fractal UI/Transcript press', component: ScenarioConversationStory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<PressArgs>
export default meta
type Story = StoryObj<PressArgs>

function SectionTranscript({ section, status, instant }: { section: TranscriptSection; status: SyncStatus; instant: number }) {
  const options = React.useMemo(() => ({ messages: section.turns.flatMap(entry => entry.prompt === undefined ? entry.items : [entry.prompt, ...entry.items]), isRunning: section.turns.some(turn => turn.work.running), onNew: async () => {} }), [section.turns])
  return <EmbraceRuntimeProvider options={options}>
    <div {...stylex.props(styles.transcript)}><Transcript title={section.title} turns={section.turns} history={section.history} sync={status} now={instant} observedAt={instant - 8000} /></div>
  </EmbraceRuntimeProvider>
}
const noSection: TranscriptSection = { id: 'conversation', title: 'Conversation', turns: [] }
/** The scenario conversation, whole, on the press surface: every thread in sections that mount without deferral. */
function ScenarioConversationStory({ scheme }: { scheme: Scheme }) {
  const { loading, instant, sections } = useScenarioSlice('conversation', scenarioSections)
  const status: SyncStatus = loading ? { _tag: 'Connecting', attempt: 1, since: instant } : { _tag: 'Live', since: instant - 5000 }
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {(sections.length === 0 ? [noSection] : sections).map(section => <SectionTranscript key={section.id} section={section} status={status} instant={instant} />)}
  </main>
}
export const ScenarioConversation: Story = { parameters: { scenario: { _tag: 'slices', slices: ['conversation'] } }, argTypes: scenarioArgTypes(['conversation']), render: args => <ScenarioConversationStory scheme={args.scheme} /> }
export const ScenarioConversationLight: Story = { ...ScenarioConversation, args: { scheme: 'light' } }

/**
 * Keep the newest 41 turns of the pinned single-thread world; Transcript still owns
 * its production mounted window. Scenario data has no failed local-send outbox.
 */
function ScenarioPressStory({ scheme }: { scheme: Scheme }) {
  const { loading, instant, sections } = useScenarioSlice('conversation', slice => scenarioSections(slice, 'whole-thread'))
  if (loading) return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)} />
  const projected = sections.flatMap(section => section.turns)
  const turns = projected.slice(-initialTurns.length)
  const history: TranscriptHistory = turns.length < projected.length ? { _tag: 'HasOlder' } : sections[0]?.history ?? { _tag: 'Complete' }
  return <PressStory scheme={scheme} initial={{ turns, history, now: instant }} revisions={false} />
}
const scenarioPress = { parameters: { scenario: { _tag: 'slices', slices: ['conversation'], world: 'ci-failing-fix', window: transcriptMountedWindow } }, args: { conversation: 'long' }, argTypes: { conversation: { control: false } }, render: args => <ScenarioPressStory scheme={args.scheme} /> } satisfies Story

/** A scrolled-up reader is not told about rows that were already there. */
export const SameRowsScrolledUp: Story = { ...scenarioPress, play: async ({ canvasElement, abortSignal }) => {
  const { viewport, canvas, jump } = await ready(canvasElement, abortSignal)
  await readerScrollsUp(viewport)
  await userEvent.click(canvas.getByRole('button', { name: 'Republish snapshot' }))
  await settleFrames()
  await expect(jump, 'same rows marked unread').not.toBeVisible()
  // Positive control: a new row still marks unread.
  await userEvent.click(canvas.getByRole('button', { name: 'Append agent reply' }))
  await waitFor(() => expect(jump).toBeVisible())
} }
export const SameRowsScrolledUpLight: Story = { ...SameRowsScrolledUp, args: { ...scenarioPress.args, scheme: 'light' } }

const styles = stylex.create({
  root: { height: '100vh', width: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  transcript: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' },
})
