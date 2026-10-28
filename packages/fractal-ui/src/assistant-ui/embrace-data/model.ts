/**
 * Type-only port of conversation/model.ts's Schema.Type contract. The source app owns decoding;
 * the isolated workshop does not import Effect or the generated SDK just to render synthetic data.
 * All source tags, readonly fields, optionality and unknown payloads are preserved here.
 */
export type Role = 'user' | 'assistant' | 'system'
export type Sender = {
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}
export type Attachment = {
  readonly id: string
  readonly mediaType: string
  readonly name?: string
  readonly size?: number
  /** Native message carrier, not the transcript row's presentation identity. */
  readonly messageId?: string
}
export type ToolCallStatus = 'running' | 'success' | 'error' | 'interrupted'
export type ToolResult = {
  readonly content: unknown
  readonly metadata?: Readonly<Record<string, unknown>>
  readonly view?: Readonly<Record<string, unknown>>
  readonly mediaType?: string
  readonly isError: boolean
  readonly at: string
}
export type RunStatus = 'queued' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled'
export type NoticeKind = 'error' | 'redaction' | 'truncation' | 'event' | 'unknown'

/** Typed harness facts; app-owned recognition never runs in the kit. */
export type HarnessNotice =
  | { readonly _tag: 'SubagentResult'; readonly taskId: string; readonly agent: string; readonly status: string; readonly duration: string; readonly payload: string; readonly raw: string; readonly lines?: number; readonly size?: string; readonly fullOutput?: string; readonly abortReason?: string; readonly error?: string; readonly mergeSummary?: string; readonly followUp?: string }
  | { readonly _tag: 'Irc'; readonly from: string; readonly to?: string; readonly replyTo?: string; readonly payload: string; readonly raw: string }
  | { readonly _tag: 'BackgroundJob'; readonly jobId: string; readonly label?: string; readonly state: BackgroundJobState; readonly duration?: string; readonly payload: string; readonly raw: string }
  /** Several jobs reported in one fact; `outcomes` is already in reader order (see backgroundJobOutcomeCounts). */
  | { readonly _tag: 'BackgroundJobGroup'; readonly outcomes: readonly BackgroundJobOutcomeCount[]; readonly payload: string; readonly raw: string }
  | { readonly _tag: 'Compaction'; readonly payload: string; readonly raw: string }
  | { readonly _tag: 'Interjection'; readonly from: string; readonly to?: string; readonly payload: string; readonly raw: string }
  | { readonly _tag: 'SkillInvocation'; readonly skill: string; readonly by: string; readonly payload: string; readonly raw: string }
  | { readonly _tag: 'Delivery'; readonly from: string; readonly to: string; readonly subject?: string; readonly payload: string; readonly raw: string }

export type BackgroundJobState = 'running' | 'completed' | 'failed' | 'cancelled' | 'unknown'
export type BackgroundJobOutcomeCount = { readonly state: BackgroundJobState; readonly count: number }
/** The one reader-facing wording of a job outcome; `unknown` stays internal. A new state fails typecheck here. */
export const backgroundJobStateLabel = (state: BackgroundJobState): string => {
  switch (state) {
    case 'running': return 'running'
    case 'completed': return 'completed'
    case 'failed': return 'failed'
    case 'cancelled': return 'cancelled'
    case 'unknown': return 'status not reported'
  }
}

/** Reader rank for grouped job outcomes: failures first, success last, never arrival order. A new state fails typecheck here. */
const outcomeRank: Record<BackgroundJobState, number> = { failed: 0, running: 1, cancelled: 2, unknown: 3, completed: 4 }
const compareBackgroundJobStates = (a: BackgroundJobState, b: BackgroundJobState): number => outcomeRank[a] - outcomeRank[b]
/** Stable reader order without changing the producer's arrival list. */
export const orderBackgroundJobs = <Job>(jobs: readonly Job[], stateOf: (job: Job) => BackgroundJobState): readonly Job[] =>
  [...jobs].sort((a, b) => compareBackgroundJobStates(stateOf(a), stateOf(b)))
export const backgroundJobOutcomeCounts = (states: Iterable<BackgroundJobState>): readonly BackgroundJobOutcomeCount[] => {
  const counts = new Map<BackgroundJobState, number>()
  for (const state of states) counts.set(state, (counts.get(state) ?? 0) + 1)
  return [...counts].sort(([a], [b]) => compareBackgroundJobStates(a, b)).map(([state, count]) => ({ state, count }))
}
export const backgroundJobOutcomeSummary = (outcomes: readonly BackgroundJobOutcomeCount[]): string =>
  outcomes.map(({ state, count }) => `${count} ${backgroundJobStateLabel(state)}`).join(', ')
/** A group reads as its most urgent member; an empty group reports no status. */
export const backgroundJobGroupLead = (outcomes: readonly BackgroundJobOutcomeCount[]): BackgroundJobState => outcomes[0]?.state ?? 'unknown'

/** One outcome language for subagents and jobs: only failure alarms, only completion earns a check, the rest stays neutral. */
export type HarnessOutcomeTone = 'failed' | 'completed' | 'neutral'
export const harnessOutcomeTone = (notice: HarnessNotice): HarnessOutcomeTone | undefined => {
  if (notice._tag === 'SubagentResult') return notice.status === 'completed' ? 'completed' : /^(?:merge )?failed(?: \(exit -?\d+\))?$/.test(notice.status) ? 'failed' : 'neutral'
  const state = notice._tag === 'BackgroundJob' ? notice.state : notice._tag === 'BackgroundJobGroup' ? backgroundJobGroupLead(notice.outcomes) : undefined
  return state === undefined ? undefined : state === 'failed' ? 'failed' : state === 'completed' ? 'completed' : 'neutral'
}

/** Visual weight of a harness event; the transcript default is one line in HarnessNotice. */
export type HarnessWeight = 'row' | 'card'


export type UnsupportedReason =
  | 'unknown-entry-type'
  | 'unknown-status'
  | 'undecodable-entry'
  | 'unknown-block-kind'
  | 'unrecognized-record'
  | 'undecodable-block'
  | 'missing-view'
export type InspectPreview = {
  readonly text: string
  readonly bytes: number
  readonly truncated: boolean
}
export type ConversationItem =
  | {
      readonly _tag: 'Text'
      readonly id: string
      readonly role: Role
      readonly text: string
      readonly attachments: readonly Attachment[]
      readonly streaming: boolean
      /** Host-observed send lifecycle for user prompts; absent means not reported. */
      readonly sendState?: SendState
      readonly model?: string
      readonly at: string
      readonly sender?: Sender
    }
  | {
      readonly _tag: 'Message'
      readonly id: string
      readonly messageId: string
      readonly from?: string
      readonly to?: string
      readonly title?: string
      readonly replyTo?: string
      readonly at: string
      readonly sender?: Sender
    }
  | {
      readonly _tag: 'Reasoning'
      readonly id: string
      readonly text: string
      readonly durationMs?: number
      readonly streaming: boolean
      readonly at: string
      readonly sender?: Sender
    }
  | {
      readonly _tag: 'ToolCall'
      readonly id: string
      readonly callId: string
      readonly name: string
      readonly server?: string
      readonly input: unknown
      readonly view?: Readonly<Record<string, unknown>>
      readonly status: ToolCallStatus
      readonly skipped?: boolean
      readonly result?: ToolResult
      readonly callSeen: boolean
      readonly at: string
      readonly sender?: Sender
    }
  | {
      readonly _tag: 'Status'
      readonly id: string
      readonly status: RunStatus
      readonly detail?: string
      readonly at: string
    }
  | {
      readonly _tag: 'Usage'
      readonly id: string
      readonly semantics: 'context_occupancy' | 'session_cumulative' | 'response'
      readonly model?: string
      readonly inputTokens?: number
      readonly outputTokens?: number
      readonly cachedTokens?: number
      readonly cost?: number
      readonly currency?: string
      readonly contextUsedPercent?: number
      readonly at: string
    }
  | {
      readonly _tag: 'Notice'
      readonly id: string
      readonly kind: NoticeKind
      readonly text: string
      readonly detail?: string
      readonly retryable?: boolean
      readonly at?: string
      readonly sender?: Sender
    }
  | {
      readonly _tag: 'Event'
      readonly id: string
      readonly kind: 'resource-observed' | 'harness-message' | 'subagent-result' | 'irc' | 'background-job' | 'compaction' | 'interjection' | 'skill-invocation' | 'delivery'
      readonly title: string
      readonly text: string
      readonly sender: Sender
      readonly data: unknown
      readonly notice?: HarnessNotice
      readonly at?: string
    }
  | {
      /** Only a redacted, capped preview crosses the rendering boundary; never the raw record. */
      readonly _tag: 'Unsupported'
      readonly id: string
      readonly harness: string
      readonly recordKind: string
      readonly reason: UnsupportedReason
      readonly inspect: InspectPreview
      readonly at?: string
    }
  | {
      readonly _tag: 'UnknownEvent'
      readonly id: string
      readonly eventType: string
      readonly data: unknown
      /** A message containing both visible content and internal records; body inspection is opt-in. */
      readonly mixedContent?: boolean
      readonly sender?: Sender
      readonly at?: string
    }

export type TextItem = Extract<ConversationItem, { _tag: 'Text' }>
export type MessageItem = Extract<ConversationItem, { _tag: 'Message' }>
export type ReasoningItem = Extract<ConversationItem, { _tag: 'Reasoning' }>
export type ToolCallItem = Extract<ConversationItem, { _tag: 'ToolCall' }>
export type StatusItem = Extract<ConversationItem, { _tag: 'Status' }>
export type UsageItem = Extract<ConversationItem, { _tag: 'Usage' }>
export type NoticeItem = Extract<ConversationItem, { _tag: 'Notice' }>
export type EventItem = Extract<ConversationItem, { _tag: 'Event' }>
export type UnknownEventItem = Extract<ConversationItem, { _tag: 'UnknownEvent' }>
export type UnsupportedItem = Extract<ConversationItem, { _tag: 'Unsupported' }>

/** Delivery lifecycle for a user prompt, from the host's own observations. */
export type SendState =
  | { readonly _tag: 'Sent' }
  | { readonly _tag: 'Pending' }
  | {
      readonly _tag: 'Failed'
      readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }
      readonly detail?: string
      /** Full native error code, preserved for diagnosis. */
      readonly code?: string
      /** Source-controlled retry policy; when present it decides whether Retry appears. */
      readonly retryable?: boolean
    }
  /** Fractal may have taken the send and has not confirmed it yet; sending it again is safe. */
  | { readonly _tag: 'Unconfirmed'; readonly detail?: string }
