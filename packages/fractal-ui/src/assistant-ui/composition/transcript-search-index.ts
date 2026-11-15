import { convertConversationItem, type ConversationItem } from '../embrace-converter'
import type { WorkLogCall } from '../taste/work-log'
import { completeStreamingTail } from './Markdown'
import { markdownSearchText, normalizeSearchText, normalizeQuery, searchOffsets } from './transcript-search-text'

export { normalizeQuery } from './transcript-search-text'

export interface SearchableTurn {
  readonly id: string
  readonly prompt?: ConversationItem
  readonly items: readonly ConversationItem[]
  readonly work: { readonly calls: readonly WorkLogCall[] }
}
export interface SearchEntry {
  readonly id: string
  /** The row's label, which shows without opening anything. */
  readonly label: string
  /** Text only an opened row shows: its summary, recorded input and full output. */
  readonly detail: string
}
/** One occurrence. Its key survives older pages prepending, so the current match never moves under the reader. */
export interface SearchMatch { readonly entryId: string; readonly occurrence: number; readonly deep: boolean }
export const searchMatchKey = (match: SearchMatch) => `${match.entryId}\u0000${match.occurrence}`

const itemTexts = new WeakMap<ConversationItem, string>()
/** The text the transcript shows for a non-tool item, from the same converter that renders it. */
const itemText = (item: ConversationItem): string => {
  const cached = itemTexts.get(item)
  if (cached !== undefined) return cached
  const content = convertConversationItem(item).content
  const raw = typeof content === 'string' ? content : content.flatMap(part => part.type === 'text' || part.type === 'reasoning' ? [part.text] : []).join('\n')
  // User prompts and semantic rows are literal text. Received prose, thoughts and mail bodies use Markdown.
  const text = item._tag === 'Text' && item.role === 'assistant' || item._tag === 'Reasoning'
    ? markdownSearchText(item.streaming ? completeStreamingTail(item.text) : item.text)
    : item._tag === 'Message'
      ? item.text === undefined && item.from === undefined && item.to === undefined && item.author === undefined
        ? markdownSearchText(raw)
        : normalizeSearchText([item.title ?? '', markdownSearchText(item.text ?? '')].join('\n'))
      : normalizeSearchText(raw)
  itemTexts.set(item, text)
  return text
}
/** Loaded source rows, mounted or not: the viewport's tail window never narrows what search covers. */
export const searchEntries = (turns: readonly SearchableTurn[]): readonly SearchEntry[] => turns.flatMap(turn => {
  const calls = new Map(turn.work.calls.map(call => [call.id, call]))
  return [...(turn.prompt === undefined ? [] : [turn.prompt]), ...turn.items].flatMap((item): readonly SearchEntry[] => {
    if (item._tag !== 'ToolCall') {
      const label = itemText(item)
      return label.trim() === '' ? [] : [{ id: item.id, label, detail: '' }]
    }
    const call = calls.get(item.id)
    if (call === undefined) return []
    const label = normalizeSearchText(call.argsSummary === undefined ? call.title : `${call.title} ${call.argsSummary}`)
    return [{ id: item.id, label, detail: normalizeSearchText([call.summary, call.rawInput, call.detail].filter(text => text !== undefined && text !== '').join('\n')) }]
  })
})
export const findMatches = (entries: readonly SearchEntry[], query: string): readonly SearchMatch[] => {
  const needle = normalizeQuery(query)
  if (needle === '') return []
  return entries.flatMap(entry => {
    const shallow = searchOffsets(normalizeSearchText(entry.label), needle).length, total = shallow + searchOffsets(normalizeSearchText(entry.detail), needle).length
    return Array.from({ length: total }, (_, occurrence) => ({ entryId: entry.id, occurrence, deep: occurrence >= shallow }))
  })
}
/** Wraps in both directions; nothing to step through stays at -1. */
export const stepMatch = (index: number, count: number, direction: 1 | -1) => count === 0 ? -1 : index === -1 ? (direction === 1 ? 0 : count - 1) : (index + direction + count) % count
/** Where a fresh query lands: the first match at or after the reader's row, else the latest one. */
export const nearestMatch = (matches: readonly SearchMatch[], rowIndex: ReadonlyMap<string, number>, origin: number) => {
  if (matches.length === 0) return -1
  const next = matches.findIndex(match => (rowIndex.get(match.entryId) ?? -1) >= origin)
  return next === -1 ? matches.length - 1 : next
}
