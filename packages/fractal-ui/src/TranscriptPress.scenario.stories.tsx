import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice } from '@smalltalk/st3-scenarios/react'
import { scenarioArgTypes } from '@smalltalk/st3-scenarios/storybook'
import { Transcript } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { SyncStatus } from './assistant-ui/st3-views/sync-status'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { scenarioSections, type TranscriptSection } from './scenario-transcript.story-harness'

const meta = { title: 'Fractal UI/Transcript press', component: ScenarioConversationStory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof ScenarioConversationStory>
export default meta
type Story = StoryObj<typeof meta>

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
const styles = stylex.create({
  root: { height: '100vh', width: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  transcript: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' },
})
