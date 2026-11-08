import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { decodeConversationChunk } from '@fractal/sdk/read'
import { assistantDarkTheme, liveComposerDarkTheme, liveAccentTheme, compositionLightTheme } from '@fractal/ui/assistant-ui'
import { colorVars as color, typeVars as type } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import wakes from '../../../../fixtures/clients/transcripts/wakes.json' with { type: 'json' }
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveTimeline } from '../conversation/fromTimeline.ts'
import { ConversationPane } from './ConversationPane.tsx'

// The shared `wakes` contract timeline through the real SDK decode, projection and pane: two wakes
// (one with `author`, one from a read without participants), agent mail, and a long agent message.
const agentRef = 'agent/example/quay'
const longBody = Array.from({ length: 12 }, (_, index) => `${index + 1}. Reader ${index + 1} moved to the new key.`).join('\n')
const records = [
  ...wakes,
  { id: 'timeline-entry/session-wake/3d3d3d3d3d3d3d3d-message', sequence: 9, revision: 1, timestamp: '2026-11-07T08:10:00.000Z', role: 'user', final: true, type: 'message', body: { message_id: 'message/mail04', author: 'person/ada', from: 'person/ada', to: agentRef, title: 'Status?' } },
  { id: 'timeline-entry/session-wake/3d3d3d3d3d3d3d3d-content', sequence: 10, revision: 1, timestamp: '2026-11-07T08:10:00.000Z', role: 'user', final: true, type: 'content', body: { media_type: 'text/plain', text: 'Skip the Windows runners for this one.' } },
  { id: 'timeline-entry/session-wake/native-11', sequence: 11, revision: 1, timestamp: '2026-11-07T08:11:00.000Z', role: 'assistant', final: true, type: 'message', body: { message_id: 'native-turn-11' } },
  { id: 'timeline-entry/session-wake/4e4e4e4e4e4e4e4e-message', sequence: 12, revision: 1, timestamp: '2026-11-07T08:11:00.000Z', role: 'assistant', final: true, type: 'message', body: { message_id: 'message/mail05', author: agentRef, from: agentRef, to: 'agent/example/docs-writer', title: 'Changelog wording' } },
  { id: 'timeline-entry/session-wake/4e4e4e4e4e4e4e4e-content', sequence: 13, revision: 1, timestamp: '2026-11-07T08:11:00.000Z', role: 'assistant', final: true, type: 'content', body: { media_type: 'text/plain', text: longBody } },
  { id: 'timeline-entry/session-wake/native-14', sequence: 14, revision: 1, timestamp: '2026-11-07T08:12:00.000Z', role: 'assistant', final: true, type: 'content', body: { media_type: 'text/plain', text: 'Done: **every reader** uses the new key, and the docs agent has the changelog wording.' } },
]
const timeline = new LiveTimeline()
timeline.apply(decodeConversationChunk({ kind: 'conversation', collection: 'conversation', id: 'follow/wakes', session_id: 'session/wakes', replace: true, has_more: false, items: records }))
const source = fixtureSource({ world: { ...fixtureProjections, conversations: { [agentRef]: { items: timeline.project().items, hasOlder: false } } } })

const EpisodeTranscript = ({ scheme }: { readonly scheme: 'dark' | 'light' }) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  return <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
    <DataSourceProvider source={source} registry={registry}><ConversationPane agentRef={agentRef} agentName="Quay" onOpenTool={() => undefined} /></DataSourceProvider>
  </main>
}
const meta = { excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Episode transcript', component: EpisodeTranscript,
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The shared wakes transcript fixture exercises the production ConversationPane through SDK decoding and the fixture data source.' } },
} satisfies Meta<typeof EpisodeTranscript>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Dark:contract-fixture',
  'Light:contract-fixture',
]
type Story = StoryObj<typeof meta>
const assertEpisodes: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  // Machine input is one quiet row each; only the person's own mail has the person's bar.
  await expect(await canvas.findByText('Woken by mission step · Review the key audit', {}, { timeout: 60000 })).toBeInTheDocument()
  await expect(canvas.getByText('Woken by restart handoff')).toBeInTheDocument()
  const people = canvas.getAllByTestId('user-message')
  await expect(people).toHaveLength(1)
  await expect(people[0]).toHaveTextContent('Skip the Windows runners for this one.')
  // A wake's whole body is behind Read message, as Markdown.
  const wake = canvas.getAllByTestId('harness-notice')[0]!
  await userEvent.click(within(wake).getByRole('button', { name: 'Read message' }))
  await expect(within(wake).getByTestId('harness-payload')).toHaveTextContent('Wake: automatic attempt 1 (ready)')
  // One object per message: sender to recipient, subject, then body; unresolved refs read as their last segment.
  const mails = canvas.getAllByTestId('mail-message')
  await expect(mails).toHaveLength(2)
  await expect(within(mails[0]!).getByTestId('mail-header')).toHaveTextContent('harbor → quay · Key audit')
  await expect(within(mails[0]!).getByTitle('agent/example/harbor')).toBeInTheDocument()
  await expect(mails[0]).toHaveAttribute('data-mail-look', 'between_others')
  // A long body collapses behind Read message.
  await expect(within(mails[1]!).getByRole('button', { name: 'Read message' })).toHaveAttribute('aria-expanded', 'false')
  // Trigger, work, outcome: one timestamp per episode, and a day line where the day changes.
  const turns = canvas.getAllByTestId('transcript-turn')
  await expect(turns).toHaveLength(4)
  for (const turn of turns) await expect(within(turn).getAllByTestId('turn-time')).toHaveLength(1)
  await expect(canvas.queryAllByTestId('message-time')).toHaveLength(0)
  await expect(canvas.getAllByTestId('transcript-day')).toHaveLength(1)
}
export const Dark: Story = { play: assertEpisodes }
export const Light: Story = { args: { scheme: 'light' }, play: assertEpisodes }
const styles = stylex.create({
  root: { height: '100vh', display: 'flex', flexDirection: 'column', backgroundColor: color.canvas, color: color.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading },
})
