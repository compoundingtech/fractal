import { ClientError, type FractalClient, type ThreadListQuery, type ThreadReplyParameters, type ThreadStartParameters } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import type { ThreadSummary, ThreadSummaryPage, ThreadTimelineEvent, ThreadTimelinePage } from '@fractal/client/schema/read'
import type { FollowEvent, ThreadWindow } from '@fractal/sdk/read'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { hasConversationCapability } from '../data/conversationPort.ts'
import { unavailable, waiting, type Feed } from '../data/source.ts'

export type ThreadObservation = { readonly summary: ThreadSummary; readonly snapshotId: string }

export type ThreadDraft = {
  readonly content: string
  readonly quote?: { readonly id: string; readonly sender: string; readonly excerpt: string }
}

export type ThreadSendResult =
  | { readonly _tag: 'Sent'; readonly threadId: string }
  | { readonly _tag: 'Refused'; readonly detail: string }

type ThreadAccess = { readonly read: boolean; readonly actions: ReadonlySet<string>; readonly actor?: string }

export interface ThreadSource {
  readonly access: Atom.Atom<ThreadAccess>
  readonly titles: Atom.Atom<ReadonlyMap<string, string>>
  readonly list: (query: ThreadListQuery) => Atom.Atom<Feed<ThreadSummaryPage>>
  readonly detail: (threadId: string) => Atom.Atom<Feed<ThreadObservation>>
  readonly timeline: (threadId: string, cursor?: string) => Atom.Atom<Feed<ThreadTimelinePage>>
  readonly draft: (destination: string) => Atom.Writable<ThreadDraft>
  readonly start: (draftKey: string, parameters: Omit<ThreadStartParameters, 'target_id' | 'signature'>) => Promise<ThreadSendResult>
  readonly reply: (parameters: Omit<ThreadReplyParameters, 'signature'>) => Promise<ThreadSendResult>
  readonly rename: (thread: ThreadObservation, title: string, titleRevision: string) => Promise<string | undefined>
  readonly changeState: (thread: ThreadObservation, state: ThreadSummary['state']) => Promise<string | undefined>
}

type Watch<Row> = (receive: (event: FollowEvent<ThreadWindow<Row>>) => void) => () => void
interface ThreadWatches {
  readonly summaries: (query: ThreadListQuery) => Watch<ThreadSummary> | undefined
  readonly timeline: (id: string) => Watch<ThreadTimelineEvent>
}

const noAccess: ThreadAccess = { read: false, actions: new Set() }
const refused = (error: unknown): Feed<never> => unavailable({
  reason: error instanceof ClientError && (error.status === 401 || error.status === 403) ? 'ungranted' : 'failed',
  detail: 'Fractal could not load this thread.',
})

export function gatewayThreads(client: FractalClient, registry: AtomRegistry.AtomRegistry, watches?: ThreadWatches): ThreadSource {
  const access = Atom.make<ThreadAccess>(get => {
    const abort = new AbortController()
    get.addFinalizer(() => abort.abort())
    void client.capabilities({ signal: abort.signal }).then(({ value }) => {
      const caps = Native.decodeUnknownSync(Native.Capabilities)(value)
      const read = hasConversationCapability(caps, 'threads-v1') && hasConversationCapability(caps, 'read.projections')
      if (!abort.signal.aborted) {
        if (!read && registry.get(titles).size > 0) registry.set(titles, new Map())
        const actions = new Set(read ? caps.capabilities.filter(capability =>
          capability.state === 'granted' && capability.id.startsWith('thread.')).map(capability => capability.id) : [])
        get.setSelf({ read, actions, actor: caps.session_actor })
      }
    }).catch(() => { if (!abort.signal.aborted) get.setSelf(noAccess) })
    return noAccess
  })
  const titles = Atom.keepAlive(Atom.make<ReadonlyMap<string, string>>(new Map()))
  const rememberTitles = (items: readonly ThreadSummary[]) => {
    const previous = registry.get(titles)
    const changed = items.filter(item => previous.get(item.id) !== item.title)
    if (changed.length > 0) registry.set(titles, new Map([...previous, ...changed.map(item => [item.id, item.title] as const)]))
  }
  const revision = Atom.make(0)
  const metadata = Atom.family((_threadId: string) => Atom.make<string | undefined>(undefined))
  function reader<T>(
    read: (signal: AbortSignal) => Promise<T>,
    settings?: {
      readonly dependsOn?: Atom.Atom<unknown>
      readonly follow?: (publish: (value: T) => void, reload: () => void, stale: () => void) => () => void
    },
  ): Atom.Atom<Feed<T>> {
    let previous: Feed<T> = waiting
    return Atom.make(get => {
      const demand = get(access)
      get(revision)
      if (settings?.dependsOn !== undefined) get(settings.dependsOn)
      if (!demand.read) return previous = waiting
      let active = true
      let abort: AbortController | undefined
      const publish = (value: T) => {
        if (active) get.setSelf(previous = { _tag: 'Observed', value, freshness: 'live' })
      }
      const reload = () => {
        if (!active) return
        abort?.abort()
        const current = new AbortController()
        abort = current
        void read(current.signal).then(value => {
          if (!current.signal.aborted) publish(value)
        }, error => {
          if (!current.signal.aborted && active) get.setSelf(previous = refused(error))
        })
      }
      const stop = settings?.follow?.(value => { abort?.abort(); publish(value) }, reload, () => {
        if (active && previous._tag === 'Observed') get.setSelf(previous = { ...previous, freshness: 'stale' })
      })
      reload()
      get.addFinalizer(() => {
        active = false
        abort?.abort()
        stop?.()
      })
      return previous
    })
  }
  function followPage<Row>(
    watch: Watch<Row> | undefined,
    onWindow?: (window: ThreadWindow<Row>) => void,
  ): ((publish: (value: { readonly items: readonly Row[]; readonly next_cursor: Option.Option<string> }) => void, reload: () => void, stale: () => void) => () => void) | undefined {
    return watch === undefined ? undefined : (publish, reload, stale) => watch(event => {
      if (event._tag !== 'Observed') {
        if (event._tag === 'Failed' && event.error._tag === 'Rejected') reload()
        else stale()
        return
      }
      onWindow?.(event.value)
      // The stream has no cursor. Read the first HTTP page only when it must
      // supply one, so older pages keep the daemon's exact seek token.
      if (event.value.hasMore) reload()
      else publish({ items: event.value.items, next_cursor: Option.none() })
    })
  }
  const readDetail = Atom.family((threadId: string) => reader(async signal => {
    const { value, snapshot } = await client.threadGet(threadId, { signal })
    const summary = Native.decodeUnknownSync(Native.ThreadSummary)(value)
    if (summary.id !== threadId) throw new TypeError('Mismatched thread observation')
    signal.throwIfAborted()
    rememberTitles([summary])
    return { summary, snapshotId: snapshot.id }
  }, { dependsOn: metadata(threadId) }))
  const readList = Atom.family((key: string) => {
    const query = JSON.parse(key) as ThreadListQuery
    const follow = followPage(query.cursor === undefined ? watches?.summaries(query) : undefined, window => rememberTitles(window.items))
    return reader(async signal => {
      const { value } = await client.threadList({ limit: 40, ...query }, { signal })
      const page = Native.decodeUnknownSync(Native.ThreadSummaryPage)(value)
      if ('agent_id' in query && page.items.some(item => item.agent_id !== query.agent_id)) throw new TypeError('Mismatched thread list')
      signal.throwIfAborted()
      rememberTitles(page.items)
      return page
    }, follow === undefined ? undefined : { follow })
  })
  const readTimeline = Atom.family((key: string) => {
    const [threadId, cursor] = key.split('\n') as [string, string | undefined]
    const follow = followPage(cursor === undefined ? watches?.timeline(threadId) : undefined, window => {
      const latestMetadata = window.items.find(event => event.kind !== 'message')?.id
      const atom = metadata(threadId)
      if (registry.get(atom) !== latestMetadata) registry.set(atom, latestMetadata)
    })
    return reader(async signal => {
      const { value } = await client.threadTimeline(threadId, { limit: 40, ...(cursor === undefined ? {} : { cursor }) }, { signal })
      return Native.decodeUnknownSync(Native.ThreadTimelinePage)(value)
    }, follow === undefined ? undefined : { follow })
  })
  const invalidate = () => registry.set(revision, registry.get(revision) + 1)
  const drafts = Atom.family((_destination: string) => Atom.keepAlive(Atom.make<ThreadDraft>({ content: '' })))
  type Attempt = { readonly fingerprint: string; readonly key: string; readonly target: string; pending?: Promise<ThreadSendResult> }
  const attempts = new Map<string, Attempt>()
  const send = (destination: string, parameters: Omit<ThreadStartParameters, 'target_id' | 'signature'> | Omit<ThreadReplyParameters, 'signature'>): Promise<ThreadSendResult> => {
    const reject = (detail: string): ThreadSendResult => ({ _tag: 'Refused', detail })
    const grant = registry.get(access)
    const kind = 'target_id' in parameters ? 'thread.reply' : 'thread.start'
    if (!grant.actions.has(kind)) return Promise.resolve(reject('This connection cannot send to threads.'))
    const previous = attempts.get(destination)
    const fingerprint = JSON.stringify(parameters)
    if (previous !== undefined && previous.fingerprint !== fingerprint) {
      return Promise.resolve(reject('Fractal has not confirmed the previous send. Retry that message first.'))
    }
    if (previous?.pending !== undefined) return previous.pending
    const attempt: Attempt = previous ?? {
      fingerprint,
      key: crypto.randomUUID(),
      target: 'target_id' in parameters ? parameters.target_id : `thread/${crypto.randomUUID()}`,
    }
    attempts.set(destination, attempt)
    attempt.pending = (async () => {
      try {
        const { snapshot, value } = await client.capabilities()
        if (value.session_actor !== grant.actor) {
          attempts.delete(destination)
          return reject('The connected account changed. Open the thread again.')
        }
        const request = { id: `action/${attempt.key}`, idempotency_key: attempt.key, fence: { snapshot_id: snapshot.id, subject_revisions: {} } }
        const result = 'target_id' in parameters
          ? await client.threadReply({ ...request, parameters })
          : await client.threadStart({ ...request, parameters: { ...parameters, target_id: attempt.target } })
        if (!accepted(result, request.id, attempt.target) || result.value.affected_ids.length !== 2
          || !result.value.affected_ids.some(id => id.startsWith('message/'))) {
          return reject('Fractal has not confirmed this send. Retry to check the same message.')
        }
        attempts.delete(destination)
        return { _tag: 'Sent', threadId: attempt.target }
      } catch (cause) {
        if (previous === undefined && cause instanceof ClientError && cause.status < 500
          && cause.response.details?.applied !== 'unknown') {
          attempts.delete(destination)
          return reject(cause.response.message)
        }
        return reject('Fractal has not confirmed this send. Retry to check the same message.')
      } finally {
        delete attempt.pending
        invalidate()
      }
    })()
    return attempt.pending
  }
  const pageKey = (subject: string, cursor?: string) => cursor === undefined ? subject : `${subject}\n${cursor}`
  const lifecycle = async (thread: ThreadObservation, action: string, edit: (
    request: {
      id: string
      idempotency_key: string
      fence: { snapshot_id: string; subject_revisions: Record<string, string> }
      parameters: { target_id: string }
    },
  ) => ReturnType<FractalClient['threadClose']>): Promise<string | undefined> => {
    if (!registry.get(access).actions.has(action)) return 'This connection cannot change threads.'
    try {
      const key = crypto.randomUUID()
      const id = `action/${key}`
      const response = await edit({
        id,
        idempotency_key: key,
        fence: { snapshot_id: thread.snapshotId, subject_revisions: { [thread.summary.id]: thread.summary.revision } },
        parameters: { target_id: thread.summary.id },
      })
      if (!accepted(response, id, thread.summary.id) || typeof response.value.metadata_revision !== 'string') {
        return 'Fractal could not confirm this thread change.'
      }
      return undefined
    } catch (cause) {
      return cause instanceof ClientError
        ? cause.response.message
        : 'Fractal could not confirm this thread change.'
    } finally {
      invalidate()
    }
  }
  return {
    access,
    titles,
    list: query => readList(JSON.stringify(query)),
    detail: readDetail,
    timeline: (threadId, cursor) => readTimeline(pageKey(threadId, cursor)),
    draft: drafts,
    start: (draftKey, parameters) => send(draftKey, parameters),
    reply: parameters => send(parameters.target_id, parameters),
    rename: (thread, title, title_revision) => lifecycle(thread, 'thread.rename', request => client.threadRename({
      ...request, parameters: { ...request.parameters, title, title_revision },
    })),
    changeState: (thread, state) => lifecycle(thread, state === 'closed' ? 'thread.close' : 'thread.reopen', request => state === 'closed'
      ? client.threadClose(request)
      : client.threadReopen(request)),
  }
}

function accepted(response: Awaited<ReturnType<FractalClient['threadClose']>>, actionId: string, threadId: string): boolean {
  return response.value.kind === 'action-result' && response.value.status === 'completed'
    && response.value.action_id === actionId && response.value.affected_ids.includes(threadId)
}
