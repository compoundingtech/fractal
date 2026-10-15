import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { ToolCallItem } from '../conversation/model.ts'
import { undeclared } from '../monitor/source.ts'
import { fixtureSource } from './fixtureSource.ts'
import { observed, type ConversationPage, type DataSource, type Feed } from './source.ts'

export const inspectorFixtureRef = 'agent/row-review'
const at = '2026-01-15T12:00:00.000Z'
const endedAt = '2026-01-15T12:00:01.250Z'
export const inspectorItems: readonly ToolCallItem[] = [
  { _tag: 'ToolCall', id: 'inspector/run', callId: 'run', name: 'run', input: { i: 'Checking row selection', command: 'pnpm test rows', cwd: 'sample-app' }, status: 'success', callSeen: true, at, result: { content: 'Three row checks passed.', isError: false, at: endedAt } },
  { _tag: 'ToolCall', id: 'inspector/read', callId: 'read', name: 'read', input: { i: 'Reading row keys', path: 'src/rows.ts' }, status: 'success', callSeen: true, at, result: { content: 'export const rowKey = (row: Row) => row.id', mediaType: 'text/typescript', isError: false, at: endedAt } },
  { _tag: 'ToolCall', id: 'inspector/long-read', callId: 'long-read', name: 'read', input: { i: 'Reading the full row catalog', path: 'src/catalog.ts' }, status: 'success', callSeen: true, at, result: { content: Array.from({ length: 40 }, (_, index) => `export const row${index + 1} = "row-${index + 1}"`).join('\n'), mediaType: 'text/typescript', isError: false, at: endedAt } },
  { _tag: 'ToolCall', id: 'inspector/change', callId: 'change', name: 'edit', input: { i: 'Preserving stable selection', path: 'src/rows.ts', oldString: 'return row.index', newString: 'return row.id' }, status: 'success', callSeen: true, at, result: { content: 'Applied the selection change.', isError: false, at: endedAt } },
  { _tag: 'ToolCall', id: 'inspector/write', callId: 'write', name: 'write', input: { i: 'Writing row keys', path: 'src/keys.ts', content: 'export const key = \"row-a\"' }, status: 'success', callSeen: true, at, result: { content: 'Wrote src/keys.ts.', isError: false, at: endedAt } },
  { _tag: 'ToolCall', id: 'inspector/raw', callId: 'raw', name: 'catalog.inspect', input: { i: 'Inspecting catalog record', key: 'row-state' }, status: 'success', callSeen: true, at, result: { content: { revision: 4, keys: ['row-a', 'row-b'] }, isError: false, at: endedAt } },
  { _tag: 'ToolCall', id: 'inspector/running', callId: 'running', name: 'run', input: { i: 'Waiting for row checks', command: 'pnpm test rows --watch' }, status: 'running', callSeen: true, at },
  { _tag: 'ToolCall', id: 'inspector/failed', callId: 'failed', name: 'run', input: { i: 'Checking an invalid selection', command: 'pnpm test missing-row' }, status: 'error', callSeen: true, at, result: { content: 'Error: selected row was not found.', isError: true, at: endedAt } },
]
export interface ToolInspectorFixture {
  readonly source: DataSource
  readonly registry: AtomRegistry.AtomRegistry
  readonly setPage: (page: ConversationPage) => void
}
/** One observed conversation page of recorded tool calls; the harness replaces it to model arriving results. */
export const createToolInspectorFixture = (): ToolInspectorFixture => {
  const registry = AtomRegistry.make()
  const conversation = Atom.make<Feed<ConversationPage>>(observed({ value: { items: inspectorItems, hasOlder: false } }))
  const source: DataSource = {
    ...fixtureSource({ world: { now: Date.parse(at), agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: undeclared } }),
    conversation: () => conversation,
  }
  return { source, registry, setPage: page => registry.set(conversation, observed({ value: page })) }
}
