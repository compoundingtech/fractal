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
import { Button as ActionButton } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import type { RetainedThreadStart, ThreadObservation, ThreadSource } from './threadSource.ts'

type OpenThread = { readonly onOpenThread?: ((id: string) => void) | undefined }

const upgradeThreadEndpoint = 'Upgrade the connected Fractal to use threads. Your browser pairing is kept.'

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
  const { read, upgradeRequired } = useAtomValue(source.access)
  const list = useAtomValue(source.list(query))
  if (upgradeRequired) return <ReadStatus state="Unavailable" detail={upgradeThreadEndpoint} />
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
  readonly exclude?: readonly string[] | undefined
}) {
  const list = useAtomValue(source.list(query))
  const [showNext, setShowNext] = React.useState(false)
  if (list._tag !== 'Observed') return <ReadStatus state={list._tag} />
  const next = Option.getOrUndefined(list.value.next_cursor)
  return <>
    {list.value.items.filter(thread => !exclude?.includes(thread.id)).map(thread => <LiveThread key={thread.id} source={source} thread={thread} onOpenThread={onOpenThread} />)}
    {next === undefined ? null : showNext
      ? <ThreadListPage source={source} query={{ ...query, cursor: next }} onOpenThread={onOpenThread} exclude={exclude} />
      : <ActionButton onPress={() => setShowNext(true)}>Load more threads</ActionButton>}
  </>
}

export function LiveInboxThreads({ alert, onOpenThread }: OpenThread & { readonly alert: Attention }) {
  const source = useDataSource().threads
  if (source === undefined || alert.episode === undefined) return null
  const agent = alert.conversation_id?.startsWith('agent/') === true ? alert.conversation_id : undefined
  return <InboxThreadsContent key={JSON.stringify([alert.source_id, alert.episode])} source={source} agent={agent}
    originSource={alert.source_id} episode={alert.episode} title={alert.title}
    canStart={alert.state === 'open'} onOpenThread={onOpenThread} />
}

function InboxThreadsContent({ source, agent, originSource, episode, title, canStart, onOpenThread }: OpenThread & {
  readonly source: ThreadSource
  readonly agent: string | undefined
  readonly originSource: string
  readonly episode: string
  readonly title: string
  readonly canStart: boolean
}) {
  const { read, actions, inboxOrigins, upgradeRequired } = useAtomValue(source.access)
  const retained = useAtomValue(source.pendingStarts?.(originSource) ?? noPendingStarts)
    .filter(start => start.parameters.origin.episode === episode)
  const [starting, setStarting] = React.useState(false)
  const [focused, setFocused] = React.useState<string>()
  const threadTitle = threadTitleFromAsk(title)
  const startInput = React.useCallback((input: HTMLTextAreaElement | null) => { input?.focus() }, [])
  const draftKey = `inbox:${JSON.stringify([originSource, episode])}`
  if (upgradeRequired || (read && !inboxOrigins)) return <ReadStatus state="Unavailable" detail={upgradeThreadEndpoint} />
  if (!read) return null
  return <div {...stylex.props(styles.groupBody)}>
    {focused === undefined ? null : <ThreadById source={source} threadId={focused} embedded focusComposer onOpenThread={onOpenThread} />}
    <ThreadListPage source={source} query={{ origin_source: originSource, origin_episode: episode }} exclude={[...(focused === undefined ? [] : [focused]), ...retained.map(start => start.targetId)]} onOpenThread={onOpenThread} />
    {retained.map(start => <PendingStartPanel key={start.draftKey} source={source} start={start} onSent={setFocused} />)}
    {retained.length > 0 || !actions.has('thread.start') || !canStart ? null : agent === undefined
      ? <p {...stylex.props(styles.note)}>No responsible agent is configured for this Inbox item.</p>
      : !starting ? <ActionButton onPress={() => setStarting(true)}>Start thread</ActionButton>
      : <ThreadPanel thread={{ title: threadTitle, state: 'open' }} initiallyExpanded onHideDraft={() => setStarting(false)}>
        <ThreadEditor source={source} destination={draftKey} label={threadTitle} inputRef={startInput}
          send={content => source.start(draftKey, { agent_id: agent, origin: { source_id: originSource, episode }, title: threadTitle, content })}
          onSent={id => { setFocused(id); setStarting(false) }} />
      </ThreadPanel>}
  </div>
}

/** Pending first sends survive the Inbox source that originally hosted their editor. */
export function LivePendingInboxStarts({ items, onOpenThread }: OpenThread & { readonly items: readonly Attention[] }) {
  const source = useDataSource().threads
  return source === undefined ? null : <PendingInboxStarts source={source} items={items} onOpenThread={onOpenThread} />
}

function PendingInboxStarts({ source, items, onOpenThread }: OpenThread & { readonly source: ThreadSource; readonly items: readonly Attention[] }) {
  const starts = useAtomValue(source.pendingStarts?.() ?? noPendingStarts).filter(start => !items.some(item =>
    item.source_id === start.parameters.origin.source_id && item.episode === start.parameters.origin.episode))
  const [focused, setFocused] = React.useState<string>()
  if (starts.length === 0 && focused === undefined) return null
  return <section aria-label="Thread recovery" {...stylex.props(styles.group)}>
    <h3 {...stylex.props(styles.note)}>Thread recovery</h3>
    <div {...stylex.props(styles.groupBody)}>
      {focused === undefined ? null : <ThreadById source={source} threadId={focused} embedded focusComposer onOpenThread={onOpenThread} />}
      {starts.map(start => <PendingStartPanel key={start.draftKey} source={source} start={start} onSent={setFocused} />)}
    </div>
  </section>
}

function PendingStartPanel({ source, start, onSent }: {
  readonly source: ThreadSource
  readonly start: RetainedThreadStart
  readonly onSent: (id: string) => void
}) {
  const agent = useAgent(start.parameters.agent_id)
  const name = agent._tag === 'Observed' ? agent.value?.name : undefined
  const retry = source.retryStart
  return <ThreadPanel thread={{ title: start.parameters.title, state: 'open' }} initiallyExpanded
    pendingChange="thread start" retryPending={start.state === 'sending'}
    {...(retry === undefined ? {} : { onRetryChange: async () => {
      const result = await retry(start.draftKey)
      if (result._tag === 'Refused') return result.detail
      onSent(result.threadId)
      return undefined
    } })}>
    <p {...stylex.props(styles.note)}>To {name ?? start.parameters.agent_id}</p>
    <div {...stylex.props(styles.message)}>{start.parameters.content}</div>
  </ThreadPanel>
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
  const { read, upgradeRequired } = useAtomValue(source.access)
  const detail = useAtomValue(source.detail(threadId))
  if (upgradeRequired) return <ReadStatus state="Unavailable" detail={upgradeThreadEndpoint} />
  if (!read) return <p>This connection does not support threads.</p>
  if (detail._tag !== 'Observed') return <ReadStatus state={detail._tag} />
  return <LiveThread source={source} thread={detail.value.summary} embedded={embedded} initiallyExpanded focusComposer={focusComposer} onOpenThread={onOpenThread} />
}

const noPendingChange = Atom.make<string | undefined>(undefined)
const idleDetail = Atom.make<Feed<ThreadObservation>>(waiting)
const noPendingStarts = Atom.make<readonly RetainedThreadStart[]>([])

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
  const retryChange = source.retryChange
  const pendingChange = useAtomValue(source.pendingChange?.(thread.id) ?? noPendingChange)
  const markRead = summary.unread === true && actions.has('thread.read') ? source.markRead : undefined
  const viewport = useThreadViewport(expanded && markRead !== undefined)
  return <ThreadPanel thread={summary} pendingChange={pendingChange}
    {...(retryChange === undefined ? {} : { onRetryChange: () => retryChange(thread.id) })}
    embedded={embedded} initiallyExpanded={initiallyExpanded} onExpansionChange={setExpanded}
    {...(summary.message_count === undefined ? {} : { messageCount: summary.message_count })}
    unread={summary.unread === true}
    {...(onOpenThread === undefined ? {} : { onOpenInTab: () => onOpenThread(thread.id) })}
    {...(observation === undefined || !actions.has('thread.rename') ? {} : {
      onRename: (title: string, revision: string) => source.rename(observation, title, revision),
    })}
    {...(observation === undefined || !actions.has(summary.state === 'open' ? 'thread.close' : 'thread.reopen') ? {} : {
      onChangeState: (state: ThreadSummary['state']) => source.changeState(observation, state),
    })}
    {...(observation === undefined || markRead === undefined ? {} : {
      canMarkRead: viewport.canMark,
      onMarkRead: () => {
        const seen = viewport.capture()
        const anchor = seen.at(-1)
        return anchor === undefined
          ? Promise.resolve('Scroll to a visible reply before marking it read.')
          : markRead(observation, anchor, seen)
      },
    })}
  >
    <ThreadConversation source={source} thread={summary} editable={actions.has('thread.reply') && observation !== undefined} focusComposer={focusComposer} timelineRef={viewport.timeline} />
  </ThreadPanel>
}

function useThreadViewport(enabled: boolean) {
  const timeline = React.useRef<HTMLDivElement>(null)
  const [canMark, setCanMark] = React.useState(false)
  const capture = React.useRef<() => string[]>(() => [])
  React.useLayoutEffect(() => {
    setCanMark(false)
    const container = timeline.current
    if (!enabled || container === null || typeof IntersectionObserver === 'undefined') return
    const tracked = new Set<Element>()
    const visible = new Set<Element>()
    let active = true
    const track = (entries: readonly IntersectionObserverEntry[]) => {
      if (!active) return
      for (const entry of entries) {
        if (tracked.has(entry.target) && entry.isIntersecting && entry.intersectionRatio >= 0.001) visible.add(entry.target)
        else visible.delete(entry.target)
      }
      setCanMark(visible.size > 0)
    }
    // Viewport-rooted observation also clips against the pane’s ancestor scroll containers.
    const observer = new IntersectionObserver(track, { threshold: 0.001 })
    const rows = () => [...container.querySelectorAll('[data-thread-message]')]
    const syncRows = () => {
      const current = new Set(rows())
      for (const row of tracked) {
        if (current.has(row)) continue
        observer.unobserve(row)
        visible.delete(row)
        tracked.delete(row)
      }
      for (const row of current) {
        if (tracked.has(row)) continue
        tracked.add(row)
        observer.observe(row)
      }
      setCanMark(visible.size > 0)
    }
    const changes = new MutationObserver(syncRows)
    changes.observe(container, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-thread-message'] })
    syncRows()
    capture.current = () => {
      track(observer.takeRecords())
      // The sticky control can overlap a row that still intersects the viewport.
      // Recheck current scroll clipping at the moment the person presses it.
      const bounds = threadViewportBounds(container)
      // Timeline DOM order is chronological, including earlier history pages.
      const seen = rows().flatMap(row => {
        if (!visible.has(row)) return []
        const id = row.getAttribute('data-thread-message')
        const rect = row.getBoundingClientRect()
        return id !== null && rect.top < bounds.bottom && rect.bottom > bounds.top
          && rect.left < bounds.right && rect.right > bounds.left ? [id] : []
      })
      return [...new Set(seen)].slice(-50)
    }
    return () => {
      active = false
      changes.disconnect()
      observer.disconnect()
      capture.current = () => []
    }
  }, [enabled])
  return { timeline, canMark, capture: () => capture.current() }
}

function threadViewportBounds(container: HTMLElement) {
  const bounds = { top: 0, bottom: window.innerHeight, left: 0, right: window.innerWidth }
  const clips = (overflow: string) => overflow !== '' && overflow !== 'visible'
  for (let element = container.parentElement; element !== null; element = element.parentElement) {
    const style = getComputedStyle(element)
    if (!clips(style.overflowX) && !clips(style.overflowY)) continue
    const rect = element.getBoundingClientRect()
    if (clips(style.overflowX)) {
      bounds.left = Math.max(bounds.left, rect.left + element.clientLeft)
      bounds.right = Math.min(bounds.right, rect.left + element.clientLeft + element.clientWidth)
    }
    if (clips(style.overflowY)) {
      bounds.top = Math.max(bounds.top, rect.top + element.clientTop)
      bounds.bottom = Math.min(bounds.bottom, rect.top + element.clientTop + element.clientHeight)
    }
  }
  const heading = container.closest('[data-thread-panel]')?.querySelector('[data-thread-read-heading]')
  if (heading) bounds.top = Math.max(bounds.top, heading.getBoundingClientRect().bottom)
  return bounds
}

function ThreadConversation({ source, thread, editable, focusComposer, timelineRef }: {
  readonly source: ThreadSource
  readonly thread: ThreadSummary
  readonly editable: boolean
  readonly focusComposer: boolean
  readonly timelineRef: React.Ref<HTMLDivElement>
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
  const quote = editable ? (event: ThreadMessageEvent, sender: string) => {
    setDraft(draft => ({ ...draft, quote: { id: event.id, sender, excerpt: event.content } }))
    input.current?.focus()
  } : undefined
  return <>
    <div ref={timelineRef}><ThreadTimeline source={source} threadId={thread.id} agentId={thread.agent_id} agentName={agentName} onQuote={quote} /></div>
    {!editable ? null
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
      : <ActionButton onPress={() => setShowOlder(true)}>Load earlier messages</ActionButton>}
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
        return <li key={event.id} data-thread-message={event.kind === 'message' && event.actor_id === agentId && feed.freshness === 'live' ? event.id : undefined} {...stylex.props(stylex.defaultMarker(), styles.event)}>
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

function ReadStatus({ state, detail }: { readonly state: 'Waiting' | 'Unavailable'; readonly detail?: string }) {
  return state === 'Waiting' ? <p role="status">Loading threads…</p> : <p role="alert">{detail ?? 'Threads could not be loaded.'}</p>
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
