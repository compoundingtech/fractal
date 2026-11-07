import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { useAtomValue } from '@effect/atom-react'
import { Button } from 'react-aria-components'
import type { ThreadSummary, ThreadTimelineEvent } from '@fractal/client/schema/read'
import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource, useNow } from '../data/react.tsx'
import { ThreadPanel } from './ThreadPanel.tsx'
import { waiting, type Feed } from '../data/source.ts'
import type { ThreadObservation, ThreadSource } from './threadSource.ts'

export function LiveAgentThreads({ agentId, onOpenThread }: {
  readonly agentId: string
  readonly onOpenThread?: ((id: string) => void) | undefined
}) {
  const source = useDataSource().threads
  return source === undefined ? null : <AgentThreads source={source} agentId={agentId} onOpenThread={onOpenThread} />
}

function AgentThreads({ source, agentId, onOpenThread }: {
  readonly source: ThreadSource
  readonly agentId: string
  readonly onOpenThread?: ((id: string) => void) | undefined
}) {
  const { read } = useAtomValue(source.access)
  const list = useAtomValue(source.list(agentId))
  if (!read || (list._tag === 'Observed' && list.value.items.length === 0)) return null
  return <details {...stylex.props(styles.group)}>
    <summary>Threads</summary>
    <div {...stylex.props(styles.groupBody)}>
      <ThreadListPage source={source} agentId={agentId} onOpenThread={onOpenThread} />
    </div>
  </details>
}

function ThreadListPage({ source, agentId, cursor, onOpenThread }: {
  readonly source: ThreadSource
  readonly agentId: string
  readonly cursor?: string
  readonly onOpenThread?: ((id: string) => void) | undefined
}) {
  const list = useAtomValue(source.list(agentId, cursor))
  const [showNext, setShowNext] = React.useState(false)
  if (list._tag !== 'Observed') return <ReadStatus state={list._tag} />
  const next = Option.getOrUndefined(list.value.next_cursor)
  return <>
    {list.value.items.map(thread => <LiveThread key={thread.id} source={source} thread={thread} onOpenThread={onOpenThread} />)}
    {typeof next !== 'string' ? null : showNext
      ? <ThreadListPage source={source} agentId={agentId} cursor={next} onOpenThread={onOpenThread} />
      : <Button onPress={() => setShowNext(true)}>Load more threads</Button>}
  </>
}

export function LiveThreadPane({ threadId }: { readonly threadId: string }) {
  const source = useDataSource().threads
  return source === undefined ? <p>This connection does not support threads.</p> : <StandaloneThread source={source} threadId={threadId} />
}

function StandaloneThread({ source, threadId }: { readonly source: ThreadSource; readonly threadId: string }) {
  const { read } = useAtomValue(source.access)
  const detail = useAtomValue(source.detail(threadId))
  if (!read) return <p>This connection does not support threads.</p>
  if (detail._tag !== 'Observed') return <ReadStatus state={detail._tag} />
  return <div {...stylex.props(styles.standalone)}><LiveThread source={source} thread={detail.value.summary} embedded={false} /></div>
}

const idleDetail = Atom.make<Feed<ThreadObservation>>(waiting)

function LiveThread({ source, thread, embedded = true, onOpenThread }: {
  readonly source: ThreadSource
  readonly thread: ThreadSummary
  readonly embedded?: boolean
  readonly onOpenThread?: ((id: string) => void) | undefined
}) {
  const { edit } = useAtomValue(source.access)
  const [expanded, setExpanded] = React.useState(!embedded)
  const detail = useAtomValue(expanded ? source.detail(thread.id) : idleDetail)
  const observation = detail._tag === 'Observed' ? detail.value : undefined
  return <ThreadPanel thread={observation?.summary ?? thread} embedded={embedded} onExpansionChange={setExpanded}
    {...(onOpenThread === undefined ? {} : { onOpenInTab: () => onOpenThread(thread.id) })}
    {...(!edit || observation === undefined ? {} : {
      onRename: (title: string, revision: string) => source.rename(observation, title, revision),
      onChangeState: (state: ThreadSummary['state']) => source.changeState(observation, state),
    })}
  >
    <ThreadTimeline source={source} threadId={thread.id} />
  </ThreadPanel>
}

function ThreadTimeline({ source, threadId, cursor }: {
  readonly source: ThreadSource
  readonly threadId: string
  readonly cursor?: string
}) {
  const feed = useAtomValue(source.timeline(threadId, cursor))
  const now = useNow()
  const [showOlder, setShowOlder] = React.useState(false)
  if (feed._tag !== 'Observed') return <ReadStatus state={feed._tag} />
  const next = Option.getOrUndefined(feed.value.next_cursor)
  return <>
    {typeof next !== 'string' ? null : showOlder
      ? <ThreadTimeline source={source} threadId={threadId} cursor={next} />
      : <Button onPress={() => setShowOlder(true)}>Load earlier events</Button>}
    <ol {...stylex.props(styles.timeline)}>
      {feed.value.items.toReversed().map(event => <li key={event.id} {...stylex.props(styles.event)}>
        <span>{event.actor_id}</span>
        <EventText event={event} />
        <time dateTime={new Date(event.accepted_at_unix_ms).toISOString()}>{ago(new Date(event.accepted_at_unix_ms).toISOString(), now)} ago</time>
      </li>)}
    </ol>
  </>
}

function EventText({ event }: { readonly event: ThreadTimelineEvent }) {
  switch (event.kind) {
    case 'thread.renamed': return <span>renamed “{event.previous_title}” to “{event.title}”</span>
    case 'thread.closed': return <span>closed the thread</span>
    case 'thread.reopened': return <span>reopened the thread</span>
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
  event: { display: 'flex', flexWrap: 'wrap', gap: s.sm, color: c.fgMuted, fontSize: t.metaSize },
})
