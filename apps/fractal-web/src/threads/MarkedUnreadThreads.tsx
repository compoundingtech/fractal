import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria-components'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import * as Atom from 'effect/reactivity/Atom'
import { useAtomValue } from '@effect/atom-react'
import { Option } from 'effect'
import type { ThreadSummary } from '@fractal/client/schema/read'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { ThreadPanel } from './ThreadPanel.tsx'
import { useDataSource } from '../data/react.tsx'
import { markedUnreadErrorMessage, readMarkedUnreadPages, type MarkedUnreadRead } from './markedUnreadPages.ts'
import { upgradeThreadEndpoint, type ThreadSource } from './threadSource.ts'
import type { UnreadSeekResult } from './unread-action-port.ts'

export type MarkedUnreadSource = Pick<ThreadSource, 'list' | 'unreadActions' | 'reload'>

/** The Inbox reads the actor-scoped canonical collection, including closed threads. */
export const LiveMarkedUnreadThreads = ({ onOpenThread }: { readonly onOpenThread: (id: string) => void }): React.ReactNode => {
  const source = useDataSource().threads
  return source === undefined ? null : <LiveMarkedUnreadSource source={source} onOpenThread={onOpenThread} />
}

const LiveMarkedUnreadSource = ({ source, onOpenThread }: { readonly source: ThreadSource; readonly onOpenThread: (id: string) => void }) => {
  const access = useAtomValue(source.access)
  return access.upgradeRequired
    ? <SystemNotice kind="empty" scope="inline" title={upgradeThreadEndpoint} />
    : <MarkedUnreadThreads source={source} onOpenThread={onOpenThread} />
}

export const MarkedUnreadThreads = ({ source, onOpenThread }: {
  readonly source: MarkedUnreadSource
  readonly onOpenThread: (id: string) => void
}): React.ReactNode => {
  const readAtom = React.useMemo(() => Atom.make(get => readMarkedUnreadPages(cursor => get(source.list({
    marked_unread: true,
    ...(cursor === undefined ? {} : { cursor }),
  })))), [source])
  const read = useAtomValue(readAtom)
  const open = async (thread: ThreadSummary): Promise<UnreadSeekResult> => {
    const reminder = Option.getOrUndefined(thread.unread_reminder)
    if (reminder === undefined || source.unreadActions === undefined) {
      return { _tag: 'Unavailable', detail: 'Reload this list before opening the unread reminder.' }
    }
    const result = await source.unreadActions.seek({ threadId: thread.id, anchor_message_id: reminder.anchor_message_id })
    if (result._tag === 'Found') onOpenThread(thread.id)
    return result
  }
  return <MarkedUnreadThreadsView read={read} onOpenThread={open} onRetry={source.reload} />
}

/** Public presentation surface: stories use the same rows, notices and opening state as the live Inbox. */
export const MarkedUnreadThreadsView = ({ read, onOpenThread, onRetry }: {
  readonly read: MarkedUnreadRead
  readonly onOpenThread: (thread: ThreadSummary) => Promise<UnreadSeekResult>
  readonly onRetry?: (() => void) | undefined
}): React.ReactNode => {
  const heading = React.useId()
  return <section aria-labelledby={heading} data-marked-unread-threads="" {...stylex.props(styles.section)}>
    <h3 id={heading} {...stylex.props(styles.heading)}>Marked unread</h3>
    {read._tag === 'Loading' ? <SystemNotice kind="loading" scope="inline" title="Loading marked-unread threads" />
      : read._tag === 'Error' ? <div data-marked-unread-error={read.error._tag}><SystemNotice kind="error" scope="inline" title={markedUnreadErrorMessage(read.error)} onRetry={onRetry} /></div>
      : <>
        {read.stale ? <SystemNotice kind="stale" scope="inline" title="Updates paused" onRetry={onRetry} /> : null}
        {read.items.length === 0 ? <SystemNotice kind="empty" scope="inline" title="No marked-unread threads" />
          : <div {...stylex.props(styles.list)}>{read.items.map(thread => <MarkedUnreadRow key={thread.id} thread={thread} onOpenThread={onOpenThread} />)}</div>}
      </>}
  </section>
}

type Opening = { readonly _tag: 'Idle' } | { readonly _tag: 'Seeking' } | Exclude<UnreadSeekResult, { readonly _tag: 'Found' }>

const MarkedUnreadRow = ({ thread, onOpenThread }: {
  readonly thread: ThreadSummary
  readonly onOpenThread: (thread: ThreadSummary) => Promise<UnreadSeekResult>
}): React.ReactNode => {
  const [opening, setOpening] = React.useState<Opening>({ _tag: 'Idle' })
  const open = async () => {
    if (opening._tag === 'Seeking') return
    setOpening({ _tag: 'Seeking' })
    try {
      const result = await onOpenThread(thread)
      setOpening(result._tag === 'Found' ? { _tag: 'Idle' } : result)
    } catch {
      setOpening({ _tag: 'Unavailable', detail: 'Fractal could not load this reminder’s message. Try again once it reconnects.' })
    }
  }
  return <div data-marked-unread-thread={thread.id} aria-busy={opening._tag === 'Seeking'}>
    <ThreadPanel thread={thread} {...(thread.message_count === undefined ? {} : { messageCount: thread.message_count })} unread={thread.unread === true}
      inList onActivate={() => { void open() }} age={<span data-private-unread-reminder="" title="Private reminder">
        <Icon name="bell" size={12} /><VisuallyHidden elementType="span">Private reminder</VisuallyHidden>
      </span>}>
      {null}
    </ThreadPanel>
    {opening._tag === 'Seeking' ? <SystemNotice kind="loading" scope="inline" title="Loading reminder message" />
      : opening._tag === 'Pending' || opening._tag === 'Unavailable' ? <div role="status"><SystemNotice kind="empty" scope="inline" title={opening.detail} /></div> : null}
  </div>
}

const styles = stylex.create({
  section: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0, paddingInline: s.row, paddingBlock: s.sm, fontSize: t.metaSize },
  heading: { margin: 0, color: c.fgMuted, fontSize: t.metaSize, fontWeight: 600 },
  list: { display: 'flex', flexDirection: 'column', minWidth: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm },
})
