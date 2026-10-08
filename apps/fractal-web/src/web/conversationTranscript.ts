import type {
  ConversationRuntimeOptions,
  TranscriptAvailability,
  TranscriptEmptyState,
  TranscriptHistory,
  TranscriptTurn,
} from '@smalltalk/fractal-ui/assistant-ui'
import { workLogTurnFromItems, type WorkKind } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import type { SyncStatus } from '@smalltalk/fractal-ui/assistant-ui/sync'
import type { ConversationItem, RunStatus, SendState } from '../conversation/model.ts'
import type { FeedSyncObservation } from '../data/feedSync.ts'
import type { ConversationPage, Feed } from '../data/source.ts'
import { fieldsOf } from '../conversation/semantics.ts'

/** The explicit internal-event policy lives only here; every unlisted kind remains an Unknown row. */
type KitConversationItem = NonNullable<ConversationRuntimeOptions['messages']>[number]
type KitTextItem = Extract<KitConversationItem, { _tag: 'Text' }>

/** Send-state pass-through contract: the kit's accepted failure reasons must cover every app
 * reason. Today the kit takes free strings, so the assignment below is 1:1. When the kit closes
 * the enum (fractal's next head), this line is the compile error that flags any uncovered reason
 * instead of silently narrowing. */
const kitCoversAppFailureReasons: Extract<SendState, { _tag: 'Failed' }>['reason'] extends Extract<NonNullable<KitTextItem['sendState']>, { _tag: 'Failed' }>['reason'] ? true : never = true

const isPrompt = (item: KitConversationItem): item is KitTextItem & { readonly role: 'user' } =>
  item._tag === 'Text' && item.role === 'user'

// Native source preserves raw omp records (`external_sessions.rs` push_unrecognized); the
// converter decodes their JSON envelope (`fromTimeline.ts` unrecognizedOmpItem). `custom` is
// a bucket, not a kind: its raw.customType distinguishes redundant tool-start bookkeeping.
// Model/thinking changes are native status metadata (external_sessions.rs push_unrecognized).
const internalEventKinds: Record<string, true | undefined> = {
  credential_pin: true,
  title_change: true,
  model_change: true,
  thinking_level_change: true,
  'custom/tool_execution_start': true,
}

const isOmittedItem = (item: ConversationItem): boolean => {
  if (item._tag !== 'UnknownEvent') return false
  const raw = fieldsOf(fieldsOf(item.data)['raw'])
  const customType = raw['customType']
  const kind = item.eventType === 'custom' && typeof customType === 'string'
    ? `custom/${customType}` : item.eventType
  return Object.hasOwn(internalEventKinds, kind)
}

/** The host owns work-log classification: how a tool reads, runs or edits. */
const toolKindFor = (name: string): WorkKind => {
  const tool = name.toLowerCase()
  if (/(?:^|[/_.-])(?:bash|sh|shell|exec|execute|run|command)(?:$|[/_.-])/.test(tool)) return 'run'
  return /(?:edit|write|patch|apply)/.test(tool) ? 'edit' : 'read'
}

const activeRunStatuses: Record<RunStatus, boolean> = { queued: true, running: true, waiting: true, completed: false, failed: false, cancelled: false }

const lastStatusOf = (items: readonly KitConversationItem[]) => {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index]
    if (item?._tag === 'Status') return item
  }
  return undefined
}

const elapsedBetween = (start: string, end: string): number | undefined => {
  const from = Date.parse(start)
  const to = Date.parse(end)
  return Number.isFinite(from) && Number.isFinite(to) && to >= from ? to - from : undefined
}

/** Run lifecycle comes from the run's own status entries; an unanswered call stays in flight. */
const workFactsFor = (promptAt: string, items: readonly KitConversationItem[]) => {
  const status = lastStatusOf(items)
  const running = status !== undefined
    ? activeRunStatuses[status.status]
    : items.some(item => item._tag === 'ToolCall' && item.status === 'running')
  const settledAt = status !== undefined && !activeRunStatuses[status.status] ? status.at : undefined
  return {
    running,
    failed: status?.status === 'failed',
    interrupted: status?.status === 'cancelled',
    ...(status?.status === 'failed' && status.detail !== undefined ? { failureNote: status.detail } : {}),
    ...(settledAt !== undefined ? { durationMs: elapsedBetween(promptAt, settledAt) } : {}),
  }
}

export interface TranscriptTurnOptions {
  /** Roster display name for assistant captions; never invented when absent. */
  readonly agentName?: string
  /** A page with older history may have cut its first turn's earlier entries. */
  readonly firstTurnComplete: boolean
}

/** Prepared turns preserve the leading native tail without synthesizing a human prompt. */
export type PreparedTranscriptTurn = Omit<TranscriptTurn, 'prompt'> & {
  readonly prompt?: KitTextItem & { readonly role: 'user' }
}

export const prepareTranscriptTurns = (
  items: readonly ConversationItem[],
  options: TranscriptTurnOptions,
): readonly PreparedTranscriptTurn[] => {
  const turns: PreparedTranscriptTurn[] = []
  let prompt: (KitTextItem & { readonly role: 'user' }) | undefined
  let turnItems: KitConversationItem[] = []
  const flush = () => {
    const first = prompt ?? turnItems[0]
    if (first === undefined) return
    const senderCaptions: Record<string, string | undefined> = {}
    for (const item of turnItems) {
      if (item._tag !== 'Text' || item.role !== 'assistant') continue
      const caption = item.sender?.label ?? options.agentName
      if (caption !== undefined) senderCaptions[item.id] = caption
    }
    turns.push({
      id: first.id,
      ...(prompt === undefined ? {} : { prompt }),
      items: turnItems,
      work: workLogTurnFromItems(prompt === undefined ? turnItems : [prompt, ...turnItems], {
        kindFor: toolKindFor,
        ...workFactsFor(first.at ?? '', turnItems),
        ...(first.at === undefined ? {} : { startedAt: first.at }),
        ...(turns.length === 0 && !options.firstTurnComplete ? { completeHistory: false } : {}),
      }),
      ...(Object.keys(senderCaptions).length === 0 ? {} : { senderCaptions }),
    })
    turnItems = []
  }
  for (const item of items) {
    if (isOmittedItem(item)) continue
    if (isPrompt(item)) {
      flush()
      prompt = item
      continue
    }
    turnItems.push(item)
  }
  flush()
  return turns
}

// Kit A currently requires prompt. On its optional-prompt bump, change this predicate to `true`
// and enable ConversationPane.composition.integration.test.tsx's skipped pointer test.
const kitSupportsTurn = (turn: PreparedTranscriptTurn): turn is PreparedTranscriptTurn & TranscriptTurn =>
  turn.prompt !== undefined

export const transcriptTurnsForItems = (
  items: readonly ConversationItem[],
  options: TranscriptTurnOptions,
): readonly TranscriptTurn[] => prepareTranscriptTurns(items, options).filter(kitSupportsTurn)

export type ConversationTranscriptState =
  | { readonly _tag: 'Waiting' }
  | { readonly _tag: 'Unavailable'; readonly availability: TranscriptAvailability }
  | {
      readonly _tag: 'Observed'
      readonly turns: readonly TranscriptTurn[]
      /** Runtime messages: exactly the prompts and kept items the transcript may render. */
      readonly items: readonly KitConversationItem[]
      readonly history: TranscriptHistory
      readonly filteredEmpty: boolean
      readonly emptyState: TranscriptEmptyState | undefined
      readonly isRunning: boolean
    }

const availabilityFor = (reason: 'ungranted' | 'unsupported' | 'failed', detail: string): TranscriptAvailability => ({
  _tag: 'Unavailable',
  reason: reason === 'ungranted' ? 'Conversation access not granted'
    : reason === 'unsupported' ? 'Conversation not supported'
      : 'Conversation unavailable',
  detail,
})

export const mapConversationFeed = (
  feed: Feed<ConversationPage>,
  options: { readonly agentName?: string },
): ConversationTranscriptState => {
  switch (feed._tag) {
    case 'Waiting':
      return { _tag: 'Waiting' }
    case 'Unavailable':
      return { _tag: 'Unavailable', availability: availabilityFor(feed.reason, feed.detail) }
    case 'Observed': {
      const hasOlder = feed.value.hasOlder
      const turns = transcriptTurnsForItems(feed.value.items, { agentName: options.agentName, firstTurnComplete: !hasOlder })
      // Native page provenance decides emptiness: a non-empty page whose rows are all omitted
      // (or that arrived with none) is filtered, not an empty conversation.
      const filteredEmpty = feed.value.observation?.empty === false
        && (feed.value.items.length === 0 || feed.value.items.every(isOmittedItem))
      return {
        _tag: 'Observed',
        turns,
        items: turns.flatMap(turn => [turn.prompt, ...turn.items]),
        history: hasOlder ? { _tag: 'HasOlder' } : { _tag: 'Complete' },
        filteredEmpty,
        emptyState: filteredEmpty ? { title: 'This page contains no displayable conversation entries.' } : undefined,
        isRunning: turns.some(turn => turn.work.running),
      }
    }
  }
}

/**
 * The SDK status stream owns every Live verdict. A source that reports no sync stream (fixtures)
 * declares freshness on its feed instead; retained rows never manufacture Live.
 */
export const transcriptSyncStatus = (
  observation: FeedSyncObservation | undefined,
  feed: Feed<ConversationPage>,
  now: number,
): SyncStatus =>
  observation !== undefined ? observation.status
    : feed._tag === 'Observed' && feed.freshness === 'live' ? { _tag: 'Live', since: now }
      : feed._tag === 'Observed' ? { _tag: 'Stale', reason: { _tag: 'Unknown' } }
        : { _tag: 'Connecting', attempt: 1, since: now }

export const transcriptObservedAt = (observation: FeedSyncObservation | undefined, now: number): number =>
  observation?.observedAt ?? now

/** Read-only transcript capabilities: no send/edit/retry transport is claimed by this layer. */
export const transcriptRuntimeOptions = (
  items: readonly KitConversationItem[] = [],
  isRunning = false,
): ConversationRuntimeOptions => ({
  messages: items,
  isRunning,
  isDisabled: true,
  onNew: async () => { throw new Error('This conversation view is read-only') },
})
