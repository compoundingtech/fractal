import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice, type WireSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { Transcript } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { SyncStatus } from './assistant-ui/st3-views/sync-status'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { scenarioSections, type TranscriptSection } from './scenario-transcript.story-harness'

type Family = 'tool-call' | 'tool-result' | 'reasoning' | 'streaming' | 'failed' | 'cancelled'
type Entry = WireSlice<'conversation'>['state']['threads'][number]['items'][number]
const matches = (entry: Entry, family: Family): boolean => {
  switch (family) {
    case 'tool-call': return entry.type === 'tool_call'
    case 'tool-result': return entry.type === 'tool_result'
    case 'reasoning': return entry.type === 'content' && (entry.body.blocks?.some(block => block.kind === 'reasoning' && block.visibility !== 'internal' && block.visibility !== 'hidden-by-harness') ?? false)
    case 'streaming': return entry.type === 'content' && (!entry.final || entry.revision > 1)
    case 'failed': return entry.type === 'error' || entry.type === 'status' && entry.body.status === 'failed'
    case 'cancelled': return entry.type === 'status' && entry.body.status === 'cancelled'
  }
}

function SectionTranscript({ section, sync, instant }: { section: TranscriptSection; sync: SyncStatus; instant: number }) {
  const options = React.useMemo(() => ({ messages: section.turns.flatMap(turn => turn.prompt === undefined ? turn.items : [turn.prompt, ...turn.items]), isRunning: section.turns.some(turn => turn.work.running), onNew: async () => {} }), [section.turns])
  return <EmbraceRuntimeProvider options={options}>
    <div {...stylex.props(styles.transcript)}><Transcript title={section.title} turns={section.turns} history={section.history} sync={sync} now={instant} observedAt={instant - 8000} /></div>
  </EmbraceRuntimeProvider>
}
const noSection: TranscriptSection = { id: 'conversation', title: 'Conversation', turns: [] }
/** Every story retains the whole conversation; the evidence disclosure spotlights its named family. */
function ScenarioTranscript({ scheme, family }: { scheme: Scheme; family: Family }) {
  const slice = useScenarioSlice('conversation')
  const { loading, instant, sections } = React.useMemo(() => scenarioSections(slice), [slice])
  const entries = loading ? [] : slice.state.threads.flatMap(thread => thread.items)
  const evidence = entries.filter(entry => matches(entry, family))
  // Native reasoning prose comes from body.text in the production projection. Keep the
  // separately retained block payload inspectable too, without relabelling it as prose.
  const reasoning = family === 'reasoning' ? [] : entries.filter(entry => matches(entry, 'reasoning'))
  const sync: SyncStatus = loading ? { _tag: 'Connecting', attempt: 1, since: instant } : { _tag: 'Live', since: instant - 5000 }
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <details open><summary>Native {family} evidence</summary>{evidence.length === 0 ? <p>No {family} entries in this conversation.</p> : evidence.map(entry => <pre key={entry.id} {...stylex.props(styles.evidence)}>{JSON.stringify(entry, null, 2)}</pre>)}</details>
    {reasoning.length > 0 && <details open><summary>Native reasoning blocks</summary>{reasoning.map(entry => <pre key={entry.id} {...stylex.props(styles.evidence)}>{JSON.stringify(entry, null, 2)}</pre>)}</details>}
    {(sections.length === 0 ? [noSection] : sections).map(section => <SectionTranscript key={section.id} section={section} sync={sync} instant={instant} />)}
  </main>
}

const meta = { title: 'Fractal UI/Transcript', component: ScenarioTranscript, parameters: { layout: 'fullscreen', scenario: { _tag: 'slices', slices: ['conversation'], world: 'transcript-families' } }, args: { scheme: 'dark' }, argTypes: { ...scenarioArgTypes(['conversation']), scheme: { options: ['dark', 'light'], control: 'radio' }, family: { control: false } } } satisfies Meta<typeof ScenarioTranscript>
export default meta
type Story = StoryObj<typeof meta>

export const ScenarioToolCalls: Story = { args: { family: 'tool-call' } }
export const ScenarioToolCallsLight: Story = { ...ScenarioToolCalls, args: { ...ScenarioToolCalls.args, scheme: 'light' } }
export const ScenarioToolResults: Story = { args: { family: 'tool-result' } }
export const ScenarioToolResultsLight: Story = { ...ScenarioToolResults, args: { ...ScenarioToolResults.args, scheme: 'light' } }
export const ScenarioReasoning: Story = { args: { family: 'reasoning' } }
export const ScenarioReasoningLight: Story = { ...ScenarioReasoning, args: { ...ScenarioReasoning.args, scheme: 'light' } }
export const ScenarioStreamingRevisions: Story = { args: { family: 'streaming' } }
export const ScenarioStreamingRevisionsLight: Story = { ...ScenarioStreamingRevisions, args: { ...ScenarioStreamingRevisions.args, scheme: 'light' } }
export const ScenarioFailed: Story = { args: { family: 'failed' } }
export const ScenarioFailedLight: Story = { ...ScenarioFailed, args: { ...ScenarioFailed.args, scheme: 'light' } }
export const ScenarioCancelled: Story = { args: { family: 'cancelled' } }
export const ScenarioCancelledLight: Story = { ...ScenarioCancelled, args: { ...ScenarioCancelled.args, scheme: 'light' } }

const styles = stylex.create({
  root: { minHeight: '100vh', width: '100%', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  transcript: { height: 720, display: 'flex', flexDirection: 'column' },
  evidence: { whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono },
})
