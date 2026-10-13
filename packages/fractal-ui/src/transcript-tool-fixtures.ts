import type { ConversationItem } from './assistant-ui/embrace-data/model'
import type { WorkKind } from './assistant-ui/taste/work-log'

export const toolFixtureAt = '2026-01-15T12:00:00Z'
export const toolArrivalInitial = [{ _tag: 'Text', id: 'arrival/answer', role: 'assistant', text: 'Reviewing the row selection.', attachments: [], streaming: false, at: toolFixtureAt }] as const satisfies readonly ConversationItem[]
export const arrivingSuccess: Extract<ConversationItem, { _tag: 'ToolCall' }> = { _tag: 'ToolCall', id: 'arrival/run', callId: 'arrival/run', name: 'bash', input: { i: 'Checking selected rows', command: 'pnpm test rows' }, status: 'success', callSeen: true, at: toolFixtureAt, result: { content: 'Selection retained. Three checks passed.', isError: false, at: toolFixtureAt } }

export const scopedToolCases = [
  { kind: 'run', name: 'bash', input: { i: 'Checking selected rows', command: 'pnpm test rows', cwd: 'packages/rows' }, target: 'pnpm test rows' },
  { kind: 'read', name: 'read', input: { i: 'Reading row projection', path: 'src/rows.ts' }, target: 'src/rows.ts' },
  { kind: 'change', name: 'edit', input: { i: 'Updating selection', path: 'src/selection.ts' }, target: 'src/selection.ts' },
  { kind: 'web', name: 'web_fetch', input: { i: 'Reading row guide', url: 'https://example.com/rows' }, target: 'https://example.com/rows' },
  { kind: 'search', name: 'grep', input: { i: 'Finding row selectors', pattern: 'visibleRows', path: 'src/' }, target: 'visibleRows in src/' },
  { kind: 'subagent', name: 'task', input: { i: 'Delegating row review', tasks: [{ name: 'ReviewRows', task: 'Review the row selection.' }] }, target: 'ReviewRows' },
] as const

/** Fixture-host classification; the live host's classifier is exercised separately by the app tests. */
export const fixtureToolKind = (name: string): WorkKind => scopedToolCases.find(example => example.name === name)?.kind ?? 'unknown'
export const scopedToolItems: readonly Extract<ConversationItem, { _tag: 'ToolCall' }>[] = scopedToolCases.map(example => ({ _tag: 'ToolCall', id: `scoped/${example.kind}`, callId: `scoped/${example.kind}`, name: example.name, input: example.input, status: 'success', callSeen: true, at: toolFixtureAt, result: { content: `Observed ${example.kind} result.`, isError: false, at: toolFixtureAt } }))
