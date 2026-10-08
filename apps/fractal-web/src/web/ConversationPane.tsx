import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { observeSyncStatus, syncLine, type SyncObservation, type SyncStatus as ViewSyncStatus } from '@smalltalk/fractal-ui/assistant-ui/sync-status'
import type { SyncStatus } from '@st3/sdk/effect'
import type { ConversationItem } from '../conversation/model.ts'
import { useConversation, useConversationSync, useNow } from '../data/react.tsx'

/**
 * The SDK verdict in the view tier's flattened vocabulary: server code and message, local kind
 * and detail. `Stale{Unknown}` has no view vocabulary yet, so it renders no line at all rather
 * than a made-up one.
 */
const viewStatus = (status: SyncStatus): ViewSyncStatus | undefined => {
  switch (status._tag) {
    case 'Connecting':
    case 'Requested':
    case 'Progress':
    case 'Live':
      return status
    case 'Stale': {
      if (status.reason._tag === 'Unknown') return undefined
      const lastLiveAt = status.lastLiveAt
      return { _tag: 'Stale', reason: status.reason, ...(lastLiveAt === undefined ? {} : { lastLiveAt }) }
    }
    case 'Failed':
      if (status.cause._tag === 'Server') return { _tag: 'Failed', code: status.cause.code, message: status.cause.message }
      if (status.cause._tag === 'Local')
        return { _tag: 'Failed', code: status.cause.kind, message: status.cause.detail?.message ?? status.cause.kind }
      return { _tag: 'Failed', code: 'unknown', message: 'Unknown failure' }
  }
}

/** Sender label for one row: provenance when the wire names it, otherwise the model role. */
const senderOf = (item: ConversationItem): string => {
  if ('sender' in item && item.sender !== undefined) return item.sender.label
  if (item._tag === 'Text') return item.role
  if (item._tag === 'Reasoning') return 'reasoning'
  return 'system'
}

/** The text of one tool-result content part, whatever envelope the harness chose. */
const textOf = (part: unknown): string | undefined => {
  if (typeof part === 'string') return part
  if (typeof part === 'object' && part !== null && 'text' in part && typeof part.text === 'string') return part.text
  return undefined
}

const toolResultText = (result: { readonly content: unknown } | undefined): string | undefined => {
  if (result === undefined) return undefined
  const content = result.content
  if (typeof content === 'string') return content
  if (!Array.isArray(content)) return undefined
  const text = content.map(textOf).filter((part): part is string => part !== undefined).join('\n')
  return text === '' ? undefined : text
}

/** One folded conversation row; every case of the shared renderer contract has a real presentation. */
const ConversationRow = ({ item }: { readonly item: ConversationItem }) => {
  switch (item._tag) {
    case 'Text':
      return (
        <article data-testid="conversation-item" data-kind="text" data-streaming={item.streaming} {...stylex.props(styles.item, styles.turn)}>
          <header {...stylex.props(styles.sender)}>{senderOf(item)}</header>
          <p {...stylex.props(styles.prose)}>{item.text}</p>
        </article>
      )
    case 'Message':
      return (
        <article data-testid="conversation-item" data-kind="message" {...stylex.props(styles.item)}>
          <header {...stylex.props(styles.sender)}>{item.title ?? 'message'}</header>
          <p {...stylex.props(styles.prose)}>{senderOf(item)}{item.from === undefined ? '' : ` · from ${item.from}`}</p>
        </article>
      )
    case 'Reasoning':
      return (
        <details data-testid="conversation-item" data-kind="reasoning" {...stylex.props(styles.item)}>
          <summary {...stylex.props(styles.sender)}>Reasoning · {senderOf(item)}</summary>
          <p {...stylex.props(styles.prose)}>{item.text}</p>
        </details>
      )
    case 'ToolCall':
      return (
        <article data-testid="conversation-item" data-kind="tool-call" data-status={item.status} {...stylex.props(styles.item)}>
          <header {...stylex.props(styles.sender)}>{item.name} · {item.status}</header>
          {item.result === undefined ? null : (
            <pre {...stylex.props(styles.output)}>{toolResultText(item.result) ?? JSON.stringify(item.result.content)}</pre>
          )}
        </article>
      )
    case 'Status':
      return (
        <p data-testid="conversation-item" data-kind="status" {...stylex.props(styles.item, styles.meta)}>{item.status}{item.detail === undefined ? '' : ` · ${item.detail}`}</p>
      )
    case 'Usage':
      return (
        <p data-testid="conversation-item" data-kind="usage" {...stylex.props(styles.item, styles.meta)}>
          {item.semantics}{item.model === undefined ? '' : ` · ${item.model}`}{item.outputTokens === undefined ? '' : ` · ${item.outputTokens} out`}
        </p>
      )
    case 'Notice':
      return (
        <p data-testid="conversation-item" data-kind="notice" {...stylex.props(styles.item, styles.meta)}>{item.text}</p>
      )
    case 'Event':
      return (
        <article data-testid="conversation-item" data-kind="event" {...stylex.props(styles.item)}>
          <header {...stylex.props(styles.sender)}>{item.title}</header>
          <p {...stylex.props(styles.prose)}>{item.text}</p>
        </article>
      )
    case 'UnknownEvent':
      return (
        <p data-testid="conversation-item" data-kind="unknown-event" {...stylex.props(styles.item, styles.meta)}>{item.eventType}</p>
      )
  }
}

/** The selected agent's thread: folded timeline rows plus the SDK's own per-follow sync verdict. */
export const ConversationPane = ({ ref }: { readonly ref: string }) => {
  const feed = useConversation(ref)
  const sync = useConversationSync(ref)
  const now = useNow()
  const [observation, setObservation] = React.useState<SyncObservation | undefined>(undefined)
  const status = sync === undefined ? undefined : viewStatus(sync.status)
  React.useEffect(() => {
    if (status === undefined || sync === undefined) return
    setObservation((previous) => observeSyncStatus(previous, status, sync.observedAt))
  }, [status, sync])
  const line = status === undefined || observation === undefined
    ? undefined
    : syncLine({ status, label: 'conversation', now, observedAt: observation.observedAt })
  return (
    <div data-testid="conversation-pane" {...stylex.props(styles.pane)}>
      {line === undefined ? null : (
        <p role="status" data-testid="conversation-sync" data-tone={line.tone} {...stylex.props(styles.sync, line.tone === 'warning' && styles.syncWarning, line.tone === 'error' && styles.syncError)}>
          {line.text}
        </p>
      )}
      {feed._tag === 'Waiting' ? (
        <p role="status" {...stylex.props(styles.meta)}>Waiting for the conversation.</p>
      ) : feed._tag === 'Unavailable' ? (
        <p role="status" {...stylex.props(styles.meta)}>Conversation unavailable: {feed.detail}</p>
      ) : (
        <div role="log" aria-label="Conversation" data-testid="conversation-thread" {...stylex.props(styles.thread)}>
          {feed.value.items.map((item) => <ConversationRow key={item.id} item={item} />)}
          {feed.value.items.length === 0 ? <p role="status" {...stylex.props(styles.meta)}>No conversation entries yet.</p> : null}
        </div>
      )}
    </div>
  )
}

const styles = stylex.create({
  pane: { display: 'flex', flexDirection: 'column', flexGrow: 1, minHeight: 0, overflow: 'hidden' },
  thread: { display: 'flex', flexDirection: 'column', gap: 8, overflowY: 'auto', padding: 16, minHeight: 0, flexGrow: 1 },
  item: { padding: 8, borderRadius: 8, backgroundColor: 'var(--recess)', display: 'flex', flexDirection: 'column', gap: 4 },
  turn: { borderWidth: 0 },
  sender: { fontSize: 11, color: 'var(--muted)', fontWeight: 600 },
  prose: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  output: { margin: 0, padding: 8, borderRadius: 6, backgroundColor: 'var(--canvas)', fontFamily: 'var(--mono)', fontSize: 11, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', maxHeight: 220, overflowY: 'auto' },
  meta: { color: 'var(--muted)', fontSize: 11, padding: 8 },
  sync: { padding: '6px 16px', fontSize: 11, color: 'var(--muted)', flexShrink: 0 },
  syncWarning: { color: 'var(--warning)' },
  syncError: { color: 'var(--danger)' },
})
