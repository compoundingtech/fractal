import { TimelineEntry, decodeUnknownSync } from '@fractal/client/schema'
import { timelineItems } from '../../../apps/fractal-web/src/conversation/fromTimeline.ts'
import { scenarioDomainFixture } from '@fractal/scenarios/domain-fixture'
import type { WireSlice } from '@fractal/scenarios/react'
import type { TranscriptHistory, TranscriptTurn } from './assistant-ui/composition/Transcript'
import type { ConversationItem, Sender, StatusItem, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'

export const scenarioDomainFixtureMarker: typeof scenarioDomainFixture = scenarioDomainFixture

type WireEntry = WireSlice<'conversation'>['state']['threads'][number]['items'][number]
const decodeEntry = decodeUnknownSync(TimelineEntry)
type Prompt = TextItem & { readonly role: 'user' }
export interface TranscriptSection { readonly id: string; readonly title: string; readonly turns: readonly TranscriptTurn[]; readonly history?: TranscriptHistory }
const operator: Sender = { kind: 'human', label: 'Operator' }
const assistant: Sender = { kind: 'agent', label: 'Assistant' }
/** Transcript defers turns beyond its tail plus one history batch; sections of at most this many turns mount whole. */
const sectionTurns = 6
/** Match the host's tool families, retaining unknown tools as inspectable read operations. */
const kindFor = (name: string) => {
  const tool = name.toLowerCase()
  if (/(?:^|[/_.-])(?:bash|sh|shell|exec|execute|run|command|eval|task|infra)(?:$|[/_.-])/.test(tool)) return 'run'
  return /(?:edit|write|patch|apply)/.test(tool) ? 'edit' : 'read'
}
/** Use the live native-block projection; each user content opens a turn, retaining semantic rows. */
function threadTurns(entries: readonly WireEntry[], hasMore: boolean): TranscriptTurn[] {
  const projected = timelineItems([{ entries: entries.map(entry => decodeEntry(entry)), replace: true, hasMore }])
  const turns: TranscriptTurn[] = []
  let open: { id: string; prompt?: Prompt; items: ConversationItem[]; startedAt?: string } | undefined
  const close = () => {
    if (open === undefined) return
    const { id, prompt, startedAt } = open
    const lifecycle = open.items.findLast((item): item is StatusItem => item._tag === 'Status')
    const terminal = lifecycle !== undefined && ['completed', 'failed', 'cancelled'].includes(lifecycle.status)
    const items = terminal ? open.items.map(item => item._tag === 'ToolCall' && item.status === 'running' ? { ...item, status: 'interrupted' as const } : item) : open.items
    const running = !terminal && (lifecycle === undefined || lifecycle.status === 'running' || items.some(item => item._tag === 'ToolCall' ? item.status === 'running' : (item._tag === 'Text' || item._tag === 'Reasoning') && item.streaming))
    const failed = lifecycle?.status === 'failed' || items.some(item => item._tag === 'ToolCall' && item.status === 'error')
    turns.push({ id, prompt, items, work: workLogTurnFromItems(prompt === undefined ? items : [prompt, ...items], { kindFor, running, failed, interrupted: lifecycle?.status === 'cancelled', failureNote: lifecycle?.status === 'failed' ? lifecycle.detail : undefined, startedAt, completeHistory: !hasMore }) })
  }
  for (const source of projected) {
    // Wire entries cannot contain local send state. Native visibility still owns hidden rows.
    if (source._tag === 'UnknownEvent' && source.mixedContent !== true) continue
    const item: ConversationItem = source._tag === 'Text' ? { ...source, sendState: undefined, sender: source.role === 'user' ? operator : source.role === 'assistant' ? assistant : source.sender } : source
    if (item._tag === 'Text' && item.role === 'user') {
      close()
      open = { id: item.id, prompt: { ...item, role: 'user' }, items: [], startedAt: item.at }
      continue
    }
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
