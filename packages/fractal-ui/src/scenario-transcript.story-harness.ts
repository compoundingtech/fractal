import { scenarioDomainFixture } from '@smalltalk/st3-scenarios/domain-fixture'
import type { WireSlice } from '@smalltalk/st3-scenarios/react'
import type { TranscriptHistory, TranscriptTurn } from './assistant-ui/composition/Transcript'
import type { Sender, TextItem, ToolCallItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'

export const scenarioDomainFixtureMarker: typeof scenarioDomainFixture = scenarioDomainFixture

type WireEntry = WireSlice<'conversation'>['state']['threads'][number]['items'][number]
type ToolResultEntry = Extract<WireEntry, { type: 'tool_result' }>
type Prompt = TextItem & { readonly role: 'user' }
export interface TranscriptSection { readonly id: string; readonly title: string; readonly turns: readonly TranscriptTurn[]; readonly history?: TranscriptHistory }
const operator: Sender = { kind: 'human', label: 'Operator' }
const assistant: Sender = { kind: 'agent', label: 'Assistant' }
/** Transcript defers turns beyond its tail plus one history batch; sections of at most this many turns mount whole. */
const sectionTurns = 6
/** The scenario factories and recordings currently emit shell/edit; reads remain read operations. */
const toolKinds: Readonly<Record<'shell' | 'edit' | 'read' | 'read_file', 'run' | 'edit' | 'read'>> = { shell: 'run', edit: 'edit', read: 'read', read_file: 'read' }
const kindFor = (name: string) => {
  switch (name) {
    case 'shell': case 'edit': case 'read': case 'read_file': return toolKinds[name]
    default: throw new Error(`Unclassified scenario tool: ${name}`)
  }
}
function conversationItem(entry: WireEntry, results: ReadonlyMap<string, ToolResultEntry>): TextItem | ToolCallItem | undefined {
  if (entry.type === 'content' && entry.role === 'assistant' && typeof entry.body.text === 'string') return { _tag: 'Text', id: entry.id, role: 'assistant', text: entry.body.text, attachments: [], streaming: !entry.final, at: entry.timestamp, sender: assistant }
  if (entry.type !== 'tool_call') return undefined
  const result = results.get(entry.body.call_id)
  return { _tag: 'ToolCall', id: entry.id, callId: entry.body.call_id, name: entry.body.name, input: entry.body.arguments, status: result === undefined ? 'running' : result.body.status, callSeen: true, at: entry.timestamp, sender: assistant, ...(result === undefined ? {} : { result: { content: result.body.content, mediaType: result.body.media_type, isError: result.body.status === 'error', at: result.timestamp } }) }
}
/** Story projection of wire entries: each user content opens a turn; assistant content and tool calls, joined with their results, follow it. */
function threadTurns(entries: readonly WireEntry[], hasMore: boolean): TranscriptTurn[] {
  const results = new Map(entries.flatMap(entry => entry.type === 'tool_result' ? [[entry.body.call_id, entry] as const] : []))
  const turns: TranscriptTurn[] = []
  let open: { id: string; prompt?: Prompt; items: (TextItem | ToolCallItem)[]; startedAt: string; lifecycle?: string } | undefined
  const close = () => {
    if (open === undefined) return
    const { id, prompt, items, startedAt, lifecycle } = open
    const running = lifecycle === undefined || lifecycle === 'running' || items.some(item => item._tag === 'ToolCall' ? item.status === 'running' : item.streaming)
    const failed = lifecycle === 'failed' || items.some(item => item._tag === 'ToolCall' && item.status === 'error')
    turns.push({ id, prompt, items, work: workLogTurnFromItems(prompt === undefined ? items : [prompt, ...items], { kindFor, running, failed, interrupted: lifecycle === 'interrupted', startedAt, completeHistory: !hasMore }) })
  }
  for (const entry of entries) {
    if (entry.type === 'status') {
      if (open !== undefined) open.lifecycle = entry.body.status
      continue
    }
    if (entry.type === 'content' && entry.role === 'user' && typeof entry.body.text === 'string') {
      close()
      open = { id: entry.id, prompt: { _tag: 'Text', id: entry.id, role: 'user', text: entry.body.text, attachments: [], streaming: false, at: entry.timestamp, sender: operator }, items: [], startedAt: entry.timestamp }
      continue
    }
    const item = conversationItem(entry, results)
    if (item === undefined) continue
    open ??= { id: item.id, items: [], startedAt: item.at }
    open.items.push(item)
  }
  close()
  return turns
}
/** The conversation as sections per thread; whole-thread hosts let Transcript apply its own initial mounted window. */
export const scenarioSections = ({ loading, state }: WireSlice<'conversation'>, grouping: 'bounded' | 'whole-thread' = 'bounded'): { readonly loading: boolean; readonly instant: number; readonly sections: readonly TranscriptSection[] } => ({
  loading,
  instant: loading ? 0 : state.threads.reduce((latest, thread) => thread.items.reduce((at, entry) => Math.max(at, Date.parse(entry.timestamp)), latest), 0),
  sections: loading ? [] : state.threads.flatMap(thread => {
    const incomplete = thread.has_more || (thread.history?.committed_from_sequence ?? 1) > 1
    const turns = threadTurns(thread.items, incomplete)
    const sectionSize = grouping === 'whole-thread' ? Math.max(1, turns.length) : sectionTurns
    const count = Math.max(1, Math.ceil(turns.length / sectionSize))
    return Array.from({ length: count }, (_, index): TranscriptSection => ({ id: `${thread.agent}/${index}`, title: count === 1 ? thread.agent : `${thread.agent} · part ${index + 1} of ${count}`, turns: turns.slice(index * sectionSize, (index + 1) * sectionSize), history: index === 0 && incomplete ? { _tag: 'HasOlder' } : { _tag: 'Complete' } }))
  }),
})
