import { createRoot } from 'react-dom/client'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as Atom from 'effect/reactivity/Atom'
import { initialFeedSync, type FeedSync } from '../data/feedSync.ts'
import type { DataSource, ConversationPage, TerminalScreen } from '../data/source.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import './app.css'

// Synthetic public-feed fixture for scripts/split-header-geometry-proof.mjs; no gateway.
const now = Date.parse('2026-10-19T12:00:00.000Z')
const agents = ['Metric history', 'Review metric aggregation', 'Workspace layout'].map((name, index) => decodeUnknownSync(Agent)({
  kind: 'agent', id: `agent/split-${index}`, name, host_id: `host/build-${index === 2 ? 2 : 1}`, runtime_ids: [],
  reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: new Date(now).toISOString(),
}))
const agent = agents[0]!
const base = fixtureSource({ world: {
  now, agents, missions: [], attention: [], events: [], terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  conversations: { [agent.id]: { items: [
    { _tag: 'Text', id: 'message/prompt', role: 'user', text: 'Keep the latest 20 metric samples in the history view, and make the total match the visible window.', attachments: [], streaming: false, at: new Date(now).toISOString() },
    { _tag: 'ToolCall', id: 'tool/edit', callId: 'call/edit', name: 'Edit', status: 'success', callSeen: true, at: new Date(now).toISOString(), input: { file_path: 'src/metrics/history.ts', old_string: 'export const history = rows.slice(0, 20)\nexport const total = rows.length', new_string: 'export const history = rows.slice(-20)\nexport const total = history.length' }, result: { content: 'Edit applied', isError: false, at: new Date(now).toISOString() } },
    { _tag: 'Text', id: 'message/answer', role: 'assistant', text: 'The history now keeps the **latest 20 samples** and derives its total from the same window.\n\n- Updated `src/metrics/history.ts`.\n- Older samples stay outside the displayed history window.\n- The captured patch is available in Changes.', attachments: [], streaming: false, at: new Date(now).toISOString() },
  ], hasOlder: false } },
} })
const failed = new URLSearchParams(location.search).has('sync-failed')
const reason = 'The subscription limit has been reached while loading retained conversation history. Close unused follows before retrying this conversation; the previously loaded thread and captured changes remain available.'
const sync = <T,>(): Atom.Atom<FeedSync<T>> => Atom.make({ ...initialFeedSync<T>(now), sync: { observedAt: now, status: failed ? { _tag: 'Failed', cause: { _tag: 'Server', code: 'subscription-limit', message: reason } } : { _tag: 'Live', since: now } } })
const conversationSync = sync<ConversationPage>()
const terminalSync = sync<TerminalScreen>()
const source: DataSource = { ...base, retryConversation: () => {}, sync: {
  gateway: Atom.make({ observedAt: now, status: { _tag: 'Live', since: now } }),
  agents: sync(), missions: sync(), attention: sync(), conversation: () => conversationSync, terminal: () => terminalSync,
} }
history.replaceState(null, '', '/w/' + encodeURIComponent(agent.id))
createRoot(document.getElementById('root')!).render(<DataSourceProvider source={source} registry={AtomRegistry.make()}><LiveAgentWorkspace /></DataSourceProvider>)
