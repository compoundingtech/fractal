import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { decodeConversationChunk } from '@fractal/sdk/effect'
import { assistantDarkTheme, liveComposerDarkTheme, liveAccentTheme, compositionLightTheme } from '@fractal/ui/assistant-ui'
import { colorVars as color, typeVars as type } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveTimeline } from '../conversation/fromTimeline.ts'
import { ConversationPane } from './ConversationPane.tsx'

// Synthetic native wire records cross the real SDK, projection, data-source and pane boundaries.
const agentRef = 'agent/build-host-a/workbench-shell'
const nativeBlock = (kind: string, fields: object = {}) => ({ id: `block/${kind}`, kind, source_type: 'omp', payload: { body_ref: true }, ...fields })
const records: unknown[] = []
const add = (type: string, body: object, role = 'assistant') => records.push({ id: `timeline-entry/native-proof/${records.length}`, sequence: records.length, revision: 1, final: true, timestamp: '2026-01-15T12:00:00Z', type, role, body })
const user = (text: string) => add('content', { text, media_type: 'text/plain' }, 'user')
const response = (text: string, model: string, total: number, cost: number) => add('content', { text, media_type: 'text/plain', blocks: [nativeBlock('text', { metadata: { model, usage: { total, cost_usd: cost } } })] })
user('Review two model responses in one turn.')
response('First response: checking the native records.', 'first-model', 42, 0.01)
response('Second response: the native records are readable.', 'last-model', 84, 0.025)
user('Preserve facts reported on a tool call.')
add('tool_call', { call_id: 'call/native-proof', name: 'bash', arguments: { command: 'printf ready' }, blocks: [nativeBlock('tool_call', { metadata: { model: 'tool-owner-model', usage: { total: 96, cost_usd: 0.012 } }, view: { type: 'bash', tool: 'bash', command: 'printf ready', env_keys: [] } })] })
add('tool_result', { call_id: 'call/native-proof', content: 'ready', media_type: 'text/plain', status: 'success', blocks: [nativeBlock('tool_output', { metadata: { exitCode: 0, wallTimeMs: 12 }, view: { type: 'bash', tool: 'bash', env_keys: [], exit_code: 0, wall_ms: 12 } })] }, 'tool')
add('content', { text: 'Tool facts remain attached to their response.', media_type: 'text/plain' })
user('Show a retryable error without a negotiated view.')
add('error', { message: 'Synthetic response interrupted', code: 'retry', details: {}, retryable: true, blocks: [nativeBlock('error', { metadata: { model: 'error-owner-model', usage: { total: 108, cost_usd: 0 } } })] })
user('Summarize every background job outcome.')
add('content', { text: 'Native job outcomes', media_type: 'text/plain', blocks: [nativeBlock('job', { view: { type: 'job', jobs: [{ id: 'job/completed', name: 'Completed check', state: 'completed' }, { id: 'job/unknown', name: 'Unreported check', state: 'unknown' }, { id: 'job/cancelled', name: 'Cancelled check', state: 'cancelled' }, { id: 'job/failed', name: 'Failed check', state: 'failed' }] } })] }, 'system')
add('content', { text: 'The failed job is visible without opening raw data.', media_type: 'text/plain' })
const timeline = new LiveTimeline()
timeline.apply(decodeConversationChunk({ kind: 'conversation', collection: 'conversation', id: 'follow/native-proof', session_id: 'session/native-proof', replace: true, has_more: false, items: records }))
const source = fixtureSource({ world: { ...fixtureProjections, conversations: { [agentRef]: { items: timeline.project().items, hasOlder: false } } } })

const NativeTranscript = ({ scheme }: { readonly scheme: 'dark' | 'light' }) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  return <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
    <DataSourceProvider source={source} registry={registry}><ConversationPane agentRef={agentRef} agentName="Native transcript review" onOpenTool={() => undefined} /></DataSourceProvider>
  </main>
}
const meta = { excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Native transcript', component: NativeTranscript,
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Synthetic native wire blocks exercise the production ConversationPane through SDK decoding and the fixture data source.' } },
} satisfies Meta<typeof NativeTranscript>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Dark:contract-fixture',
  'Light:contract-fixture',
]
type Story = StoryObj<typeof meta>
const assertNative: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const history = await canvas.findByTestId('transcript-history-controls', {}, { timeout: 60000 })
  const initialIds = canvas.getAllByTestId('transcript-turn').flatMap(turn => turn.dataset.sourceIds?.split(' ').filter(Boolean) ?? [])
  await expect(initialIds).toHaveLength(6)
  await expect(initialIds).toEqual(Array.from({ length: 6 }, (_, index) => `timeline-entry/native-proof/${records.length - 6 + index}`))
  await userEvent.click(canvas.getByRole('button', { name: 'Show all messages' }))
  await waitFor(() => expect(history).toHaveAttribute('data-state', 'complete'))
  await expect(await canvas.findByText('Last response: last-model · 84 tokens · $0.025', {}, { timeout: 60000 })).toBeInTheDocument()
  await expect(canvas.getByText('Last response: tool-owner-model · 96 tokens · $0.012')).toBeInTheDocument()
  await expect(canvas.getByText('Last response: error-owner-model · 108 tokens · $0')).toBeInTheDocument()
  await expect(canvas.getByText(/Synthetic response interrupted[\s\S]*Code: retry[\s\S]*Can be retried/)).toBeInTheDocument()
  await expect(canvas.getByText('4 background jobs · 1 failed, 1 cancelled, 1 status not reported, 1 completed')).toBeInTheDocument()
  await expect(canvas.getByText(/^Failed check\s+Cancelled check\s+Unreported check\s+Completed check$/)).toBeInTheDocument()
  await expect(canvas.queryByText('Last response: first-model · 42 tokens · $0.01')).toBeNull()
  await expect(canvas.queryByText(/body_ref/)).toBeNull()
  await expect(canvas.queryByTestId('live-work')).toBeNull()
}
export const Dark: Story = { play: assertNative }
export const Light: Story = { args: { scheme: 'light' }, play: assertNative }
const styles = stylex.create({
  root: { height: '100vh', display: 'flex', flexDirection: 'column', backgroundColor: color.canvas, color: color.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading },
})
