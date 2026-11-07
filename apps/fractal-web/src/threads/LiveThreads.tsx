import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { useAtom, useAtomValue } from '@effect/atom-react'
import { Button } from 'react-aria-components'
import type { ThreadListQuery } from '@fractal/client'
import type { Attention, ThreadMessageEvent, ThreadSummary, ThreadTimelineEvent } from '@fractal/client/schema/read'
import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { threadTitleFromAsk } from '../../../../clients/typescript/fractal-views/thread.ts'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useAgent, useDataSource, useNow } from '../data/react.tsx'
import { ThreadPanel } from './ThreadPanel.tsx'
import { waiting, type Feed } from '../data/source.ts'
import { ThreadEditor } from './ThreadEditor.tsx'
import type { ThreadObservation, ThreadSource } from './threadSource.ts'

type OpenThread = { readonly onOpenThread?: ((id: string) => void) | undefined }

export function LiveAgentThreads({ agentId, onOpenThread }: OpenThread & { readonly agentId: string }) {
  return <ThreadGroup query={{ agent_id: agentId }} label="Threads" onOpenThread={onOpenThread} />
}

export function LiveMissionThreads({ missionId, onOpenThread }: OpenThread & { readonly missionId: string }) {
  return <ThreadGroup query={{ mission_id: missionId }} label="Threads" onOpenThread={onOpenThread} />
}

export function LiveRecentlyClosedThreads({ onOpenThread }: OpenThread) {
  return <ThreadGroup query={{ recently_closed: true, limit: 10 }} label="Recently closed threads" onOpenThread={onOpenThread} />
}

function ThreadGroup({ query, label, onOpenThread }: OpenThread & { readonly query: ThreadListQuery; readonly label: string }) {
  const source = useDataSource().threads
  return source === undefined ? null : <ThreadGroupContent source={source} query={query} label={label} onOpenThread={onOpenThread} />
}

function ThreadGroupContent({ source, query, label, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly query: ThreadListQuery
  readonly label: string
}) {
  const { read } = useAtomValue(source.access)
  const list = useAtomValue(source.list(query))
  if (!read || (list._tag === 'Observed' && list.value.items.length === 0 && Option.isNone(list.value.next_cursor))) return null
  return <details {...stylex.props(styles.group)}>
    <summary>{label}</summary>
    <div {...stylex.props(styles.groupBody)}>
      <ThreadListPage source={source} query={query} onOpenThread={onOpenThread} />
    </div>
  </details>
}

function ThreadListPage({ source, query, onOpenThread, exclude }: OpenThread & {
  readonly source: ThreadSource
  readonly query: ThreadListQuery
  readonly exclude?: string | undefined
}) {
  const list = useAtomValue(source.list(query))
  const [showNext, setShowNext] = React.useState(false)
  if (list._tag !== 'Observed') return <ReadStatus state={list._tag} />
  const next = Option.getOrUndefined(list.value.next_cursor)
  return <>
    {list.value.items.filter(thread => thread.id !== exclude).map(thread => <LiveThread key={thread.id} source={source} thread={thread} onOpenThread={onOpenThread} />)}
    {next === undefined ? null : showNext
      ? <ThreadListPage source={source} query={{ ...query, cursor: next }} onOpenThread={onOpenThread} exclude={exclude} />
      : <Button onPress={() => setShowNext(true)}>Load more threads</Button>}
  </>
}

export function LiveAskThreads({ alert, onOpenThread }: OpenThread & { readonly alert: Attention }) {
  const source = useDataSource().threads
  const agent = alert.requester_id
  if (source === undefined || agent?.startsWith('agent/') !== true || alert.episode === undefined
    || !alert.source_id.startsWith('step-run/') || (alert.attention_kind !== 'agent-request' && alert.attention_kind !== 'person-step')) return null
  return <AskThreadsContent key={alert.id} source={source} agent={agent} ask={alert.source_id} episode={alert.episode}
    title={alert.title} canStart={alert.state === 'open' && alert.update === undefined} onOpenThread={onOpenThread} />
}

function AskThreadsContent({ source, agent, ask, episode, title, canStart, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly agent: string
  readonly ask: string
  readonly episode: string
  readonly title: string
  readonly canStart: boolean
}) {
  const { read, actions } = useAtomValue(source.access)
  const [starting, setStarting] = React.useState(false)
  const [focused, setFocused] = React.useState<string>()
  const threadTitle = threadTitleFromAsk(title)
  const startInput = React.useCallback((input: HTMLTextAreaElement | null) => { input?.focus() }, [])
  const draftKey = `ask:${ask}:${episode}`
  if (!read) return null
  return <div {...stylex.props(styles.groupBody)}>
    {focused === undefined ? null : <ThreadById source={source} threadId={focused} embedded focusComposer onOpenThread={onOpenThread} />}
    <ThreadListPage source={source} query={{ origin_ask: ask, origin_ask_episode: episode }} exclude={focused} onOpenThread={onOpenThread} />
    {!actions.has('thread.start') || !canStart ? null : !starting ? <Button onPress={() => setStarting(true)}>Start thread</Button>
      : <ThreadPanel thread={{ title: threadTitle, state: 'open' }} initiallyExpanded onHideDraft={() => setStarting(false)}>
        <ThreadEditor source={source} destination={draftKey} label={threadTitle} inputRef={startInput}
          send={content => source.start(draftKey, { agent_id: agent, origin_ask: ask, origin_ask_episode: episode, title: threadTitle, content })}
          onSent={id => { setFocused(id); setStarting(false) }} />
      </ThreadPanel>}
  </div>
}

export function LiveThreadPane({ threadId }: { readonly threadId: string }) {
  const source = useDataSource().threads
  return source === undefined ? <p>This connection does not support threads.</p>
    : <div {...stylex.props(styles.standalone)}><ThreadById source={source} threadId={threadId} /></div>
}

function ThreadById({ source, threadId, embedded = false, focusComposer = false, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly threadId: string
  readonly embedded?: boolean
  readonly focusComposer?: boolean
}) {
  const { read } = useAtomValue(source.access)
  const detail = useAtomValue(source.detail(threadId))
  if (!read) return <p>This connection does not support threads.</p>
  if (detail._tag !== 'Observed') return <ReadStatus state={detail._tag} />
  return <LiveThread source={source} thread={detail.value.summary} embedded={embedded} initiallyExpanded focusComposer={focusComposer} onOpenThread={onOpenThread} />
}

const idleDetail = Atom.make<Feed<ThreadObservation>>(waiting)

function LiveThread({ source, thread, embedded = true, initiallyExpanded = false, focusComposer = false, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly thread: ThreadSummary
  readonly embedded?: boolean
  readonly initiallyExpanded?: boolean
  readonly focusComposer?: boolean
}) {
  const { actions } = useAtomValue(source.access)
  const [expanded, setExpanded] = React.useState(!embedded || initiallyExpanded)
  const detail = useAtomValue(expanded ? source.detail(thread.id) : idleDetail)
  const observation = detail._tag === 'Observed' ? detail.value : undefined
  const summary = observation?.summary ?? thread
  return <ThreadPanel thread={summary} embedded={embedded} initiallyExpanded={initiallyExpanded} onExpansionChange={setExpanded}
    {...(summary.message_count === undefined ? {} : { messageCount: summary.message_count })}
    unread={summary.unread === true}
    {...(onOpenThread === undefined ? {} : { onOpenInTab: () => onOpenThread(thread.id) })}
    {...(observation === undefined || !actions.has('thread.rename') ? {} : {
      onRename: (title: string, revision: string) => source.rename(observation, title, revision),
    })}
    {...(observation === undefined || !actions.has(summary.state === 'open' ? 'thread.close' : 'thread.reopen') ? {} : {
      onChangeState: (state: ThreadSummary['state']) => source.changeState(observation, state),
    })}
  >
    <ThreadConversation source={source} thread={summary} editable={actions.has('thread.reply') && observation !== undefined} focusComposer={focusComposer} />
  </ThreadPanel>
}

function ThreadConversation({ source, thread, editable, focusComposer }: {
  readonly source: ThreadSource
  readonly thread: ThreadSummary
  readonly editable: boolean
  readonly focusComposer: boolean
}) {
  const [, setDraft] = useAtom(source.draft(thread.id))
  const input = React.useRef<HTMLTextAreaElement>(null)
  const didFocus = React.useRef(false)
  const attachInput = React.useCallback((element: HTMLTextAreaElement | null) => {
    input.current = element
    if (focusComposer && element !== null && !didFocus.current) {
      didFocus.current = true
      element.focus()
    }
  }, [focusComposer])
  const agent = useAgent(thread.agent_id)
  const agentName = agent._tag === 'Observed' ? agent.value?.name : undefined
  const quote = editable && thread.state === 'open' ? (event: ThreadMessageEvent, sender: string) => {
    setDraft(draft => ({ ...draft, quote: { id: event.id, sender, excerpt: event.content } }))
    input.current?.focus()
  } : undefined
  return <>
    <ThreadTimeline source={source} threadId={thread.id} agentId={thread.agent_id} agentName={agentName} onQuote={quote} />
    {thread.state === 'closed' ? <p {...stylex.props(styles.note)}>Reopen this thread to send a message.</p> : !editable ? null
      : <ThreadEditor source={source} destination={thread.id} label={thread.title} inputRef={attachInput}
        send={(content, quoteId) => source.reply({ target_id: thread.id, content, ...(quoteId === undefined ? {} : { thread_quote_id: quoteId }) })} />}
  </>
}

type Quote = ((event: ThreadMessageEvent, sender: string) => void) | undefined

function ThreadTimeline({ source, threadId, cursor, agentId, agentName, onQuote }: {
  readonly source: ThreadSource
  readonly threadId: string
  readonly cursor?: string
  readonly agentId: string
  readonly agentName: string | undefined
  readonly onQuote: Quote
}) {
  const feed = useAtomValue(source.timeline(threadId, cursor))
  const { actor } = useAtomValue(source.access)
  const now = useNow()
  const [showOlder, setShowOlder] = React.useState(false)
  if (feed._tag !== 'Observed') return <ReadStatus state={feed._tag} />
  const next = Option.getOrUndefined(feed.value.next_cursor)
  return <>
    {next === undefined ? null : showOlder
      ? <ThreadTimeline source={source} threadId={threadId} cursor={next} agentId={agentId} agentName={agentName} onQuote={onQuote} />
      : <Button onPress={() => setShowOlder(true)}>Load earlier messages</Button>}
    {feed.freshness === 'stale' ? <p role="status" {...stylex.props(styles.note)}>Updates paused</p> : null}
    <ol {...stylex.props(styles.timeline)}>
      {feed.value.items.toReversed().map(event => {
        const sender = event.actor_id === actor || actor?.startsWith(`${event.actor_id}/session/`) ? 'You' : event.actor_id === agentId && agentName !== undefined ? agentName : event.actor_id.split('/').at(-1) ?? event.actor_id
        const date = new Date(event.accepted_at_unix_ms)
        const iso = Number.isFinite(date.getTime()) ? date.toISOString() : undefined
        const quotedId = event.kind === 'message' ? Option.getOrUndefined(event.thread_quote_id) : undefined
        const quoted = quotedId === undefined ? undefined : feed.value.items.find(
          (candidate): candidate is ThreadMessageEvent => candidate.kind === 'message' && candidate.id === quotedId,
        )
        return <li key={event.id} {...stylex.props(stylex.defaultMarker(), styles.event)}>
          <div {...stylex.props(styles.eventHeader)}>
            <strong>{sender}</strong>
            {iso === undefined ? null : <time dateTime={iso}>{ago(iso, now)} ago</time>}
            {event.kind !== 'message' || onQuote === undefined ? null : <Button aria-label={`Reply to ${sender}`} onPress={() => onQuote(event, sender)} {...stylex.props(styles.reply)}>Reply</Button>}
          </div>
          <EventText event={event} excerpt={quoted?.content.slice(0, 160)} />
        </li>
      })}
    </ol>
  </>
}

function EventText({ event, excerpt }: { readonly event: ThreadTimelineEvent; readonly excerpt: string | undefined }) {
  switch (event.kind) {
    case 'message': return <>
      {Option.isNone(event.thread_quote_id) ? null : <span title={event.thread_quote_id.value}>{excerpt ?? 'Reply to an earlier message'}</span>}
      <div {...stylex.props(styles.message)}>{event.content}</div>
    </>
    case 'thread.renamed': return <span>Renamed “{event.previous_title}” to “{event.title}”</span>
    case 'thread.closed': return <span>Closed the thread</span>
    case 'thread.reopened': return <span>Reopened the thread</span>
  }
}

function ReadStatus({ state }: { readonly state: 'Waiting' | 'Unavailable' }) {
  return state === 'Waiting' ? <p role="status">Loading threads…</p> : <p role="alert">Threads could not be loaded.</p>
}

const styles = stylex.create({
  group: { flexShrink: 0, paddingInline: s.md, fontSize: t.metaSize },
  groupBody: { display: 'flex', flexDirection: 'column', gap: s.sm, maxHeight: '35vh', overflowY: 'auto', paddingBlock: s.sm },
  standalone: { position: 'absolute', inset: 0, overflowY: 'auto', backgroundColor: c.canvas, padding: s.md },
  timeline: { display: 'flex', flexDirection: 'column', gap: s.sm, padding: 0, margin: 0, listStyle: 'none' },
  event: { display: 'flex', flexDirection: 'column', gap: s.xs, color: c.fgMuted, fontSize: t.metaSize },
  eventHeader: { display: 'flex', alignItems: 'baseline', gap: s.sm },
  reply: { opacity: { default: 0, ':focus-visible': 1, [stylex.when.ancestor(':hover')]: 1, '@media (hover: none)': 1 } },
  message: { color: c.fg, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  note: { margin: 0, color: c.fgMuted, fontSize: t.metaSize },
})
