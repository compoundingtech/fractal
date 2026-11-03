import { TimelineEntry, decodeUnknownSync } from '@fractal/client/schema'
import { timelineItems } from '../../../apps/fractal-web/src/conversation/fromTimeline.ts'
import { scenarioDomainFixture } from '@fractal/scenarios/domain-fixture'
import type { WireSlice } from '@fractal/scenarios/react'
import { transcriptInitialSourceRowCount, type TranscriptHistory, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import type { ConversationItem, Sender, StatusItem, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'

export const scenarioDomainFixtureMarker: typeof scenarioDomainFixture = scenarioDomainFixture

type WireEntry = WireSlice<'conversation'>['state']['threads'][number]['items'][number]
const decodeEntry = decodeUnknownSync(TimelineEntry)
type Prompt = TextItem & { readonly role: 'user' }
export interface TranscriptSection { readonly id: string; readonly title: string; readonly turns: readonly TranscriptTurn[]; readonly history?: TranscriptHistory }
const operator: Sender = { kind: 'human', label: 'Operator' }
const assistant: Sender = { kind: 'agent', label: 'Assistant' }
const sourceRows = (turn: TranscriptTurn) => turn.prompt === undefined ? turn.items.length : turn.items.length + 1
/**
 * Bounded sections mount whole: each holds at most Transcript's initial source rows (prompts and items). A turn with
 * more rows continues in the next section without its prompt, as a host shows history that starts mid-turn; only its
 * last part carries the run's live, failed or interrupted state.
 */
function sourceRowParts(turn: TranscriptTurn): TranscriptTurn[] {
  if (sourceRows(turn) <= transcriptInitialSourceRowCount) return [turn]
  const promptRows = turn.prompt === undefined ? 0 : 1
  const parts = Math.ceil(sourceRows(turn) / transcriptInitialSourceRowCount)
  return Array.from({ length: parts }, (_, part) => {
    const items = turn.items.slice(Math.max(0, part * transcriptInitialSourceRowCount - promptRows), (part + 1) * transcriptInitialSourceRowCount - promptRows)
    const ids = new Set(items.map(item => item.id))
    const last = part === parts - 1
    return {
      ...turn, id: part === 0 ? turn.id : `${turn.id}/part-${part + 1}`, prompt: part === 0 ? turn.prompt : undefined, items,
      work: { ...turn.work, calls: turn.work.calls.filter(call => ids.has(call.id)), running: last && turn.work.running, failed: last && turn.work.failed, interrupted: last && turn.work.interrupted },
    }
  })
}
/** Greedily packs whole turn parts into sections of at most Transcript's initial source rows. */
function boundedGroups(turns: readonly TranscriptTurn[]): TranscriptTurn[][] {
  const groups: TranscriptTurn[][] = []
  let rows = transcriptInitialSourceRowCount
  for (const part of turns.flatMap(sourceRowParts)) {
    if (rows + sourceRows(part) > transcriptInitialSourceRowCount) {
      groups.push([])
      rows = 0
    }
    groups.at(-1)!.push(part)
    rows += sourceRows(part)
  }
  return groups
}
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
    const groups = grouping === 'whole-thread' || turns.length === 0 ? [turns] : boundedGroups(turns)
    return groups.map((group, index): TranscriptSection => ({ id: `${thread.agent}/${index}`, title: groups.length === 1 ? thread.agent : `${thread.agent} · part ${index + 1} of ${groups.length}`, turns: group, history: index === 0 && incomplete ? { _tag: 'HasOlder' } : { _tag: 'Complete' } }))
  }),
})
