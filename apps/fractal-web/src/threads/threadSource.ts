import { actionReceiptCheck, sendBoundAction, resendBoundAction, type BoundAction, type ActionReceiptExpectation, ClientError, type FractalClient, type ThreadListQuery, type ThreadReplyParameters, type ThreadStartParameters } from '@fractal/client'
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

type ThreadStartIntent = Readonly<Omit<ThreadStartParameters, 'target_id' | 'signature' | 'origin'>> & {
  readonly origin: Readonly<ThreadStartParameters['origin']>
}

/** Discover the original first send even when the current source has moved to another episode. */
export interface RetainedThreadStart {
  readonly draftKey: string
  readonly targetId: string
  readonly state: 'sending' | 'unconfirmed'
  readonly parameters: ThreadStartIntent
}

type ThreadAccess = { readonly read: boolean; readonly inboxOrigins?: boolean; readonly upgradeRequired?: boolean; readonly actions: ReadonlySet<string>; readonly actor?: string }

export interface ThreadSource {
  readonly access: Atom.Atom<ThreadAccess>
  readonly titles: Atom.Atom<ReadonlyMap<string, string>>
  readonly list: (query: ThreadListQuery) => Atom.Atom<Feed<ThreadSummaryPage>>
  readonly detail: (threadId: string) => Atom.Atom<Feed<ThreadObservation>>
  readonly timeline: (threadId: string, cursor?: string) => Atom.Atom<Feed<ThreadTimelinePage>>
  readonly draft: (destination: string) => Atom.Writable<ThreadDraft>
  readonly start: (draftKey: string, parameters: Omit<ThreadStartParameters, 'target_id' | 'signature'>) => Promise<ThreadSendResult>
  readonly pendingStarts?: (sourceId?: string) => Atom.Atom<readonly RetainedThreadStart[]>
  readonly retryStart?: (draftKey: string) => Promise<ThreadSendResult>
  readonly reply: (parameters: Omit<ThreadReplyParameters, 'signature'>) => Promise<ThreadSendResult>
  readonly rename: (thread: ThreadObservation, title: string, titleRevision: string) => Promise<string | undefined>
  readonly changeState: (thread: ThreadObservation, state: ThreadSummary['state']) => Promise<string | undefined>
  readonly pendingChange?: (threadId: string) => Atom.Atom<string | undefined>
  readonly retryChange?: (threadId: string) => Promise<string | undefined>
  readonly markRead?: (thread: ThreadObservation, through: string, seen: readonly string[]) => Promise<string | undefined>
}

type Watch<Row> = (receive: (event: FollowEvent<ThreadWindow<Row>>) => void) => () => void
interface ThreadWatches {
  readonly summaries: (query: ThreadListQuery) => Watch<ThreadSummary> | undefined
  readonly detail: (id: string) => Watch<ThreadSummary>
  readonly timeline: (id: string) => Watch<ThreadTimelineEvent>
}

// Paired native actions require a device signature; browser sends need their own grant.
const canSendUnsignedThread = (caps: Pick<Native.Capabilities, 'transport' | 'capabilities'>): boolean =>
  caps.transport === 'local-web' || caps.capabilities.some(capability =>
    capability.id === 'thread-browser-send' && capability.version >= 1 && capability.state === 'granted',
  )

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
      const upgradeRequired = ([['threads-v1', 2], ['thread-summary-feed', 1]] as const).some(([id, version]) => !caps.capabilities.some(capability => capability.id === id && capability.version >= version))
      const inboxOrigins = caps.capabilities.some(capability => capability.id === 'threads-v1' && capability.version >= 2 && capability.state === 'granted')
      const read = inboxOrigins && hasConversationCapability(caps, 'read.projections') && hasConversationCapability(caps, 'thread-summary-feed')
      if (!abort.signal.aborted) {
        if (!read && registry.get(titles).size > 0) registry.set(titles, new Map())
        const actions = new Set(read ? caps.capabilities.filter(capability =>
          capability.state === 'granted' && capability.id.startsWith('thread.')
          && (canSendUnsignedThread(caps) || (capability.id !== 'thread.start' && capability.id !== 'thread.reply'))
        ).map(capability => capability.id) : [])
        get.setSelf({ read, inboxOrigins, upgradeRequired, actions, actor: caps.session_actor })
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
  function reader<T>(
    read: (signal: AbortSignal) => Promise<T>,
    settings?: {
      readonly follow?: (publish: (value: T) => void, reload: () => void, stale: () => void) => () => void
    },
  ): Atom.Atom<Feed<T>> {
    let previous: Feed<T> = waiting
    return Atom.make(get => {
      const demand = get(access)
      get(revision)
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
  const readDetail = Atom.family((threadId: string) => {
    const follow = (publish: (value: ThreadObservation) => void, reload: () => void, stale: () => void) => {
      if (watches === undefined) return () => {}
      return watches.detail(threadId)(event => {
        if (event._tag !== 'Observed') {
          if (event._tag === 'Failed' && event.error._tag === 'Rejected') reload()
          else stale()
          return
        }
        const [summary] = event.value.items
        if (summary?.id !== threadId || event.value.items.length !== 1 || event.value.hasMore) {
          reload()
          return
        }
        rememberTitles([summary])
        publish({ summary, snapshotId: event.value.snapshotId })
      })
    }
    return reader(async signal => {
      const { value, snapshot } = await client.threadGet(threadId, { signal })
      const summary = Native.decodeUnknownSync(Native.ThreadSummary)(value)
      if (summary.id !== threadId) throw new TypeError('Mismatched thread observation')
      signal.throwIfAborted()
      rememberTitles([summary])
      return { summary, snapshotId: snapshot.id }
    }, { follow })
  })
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
    const follow = followPage(cursor === undefined ? watches?.timeline(threadId) : undefined)
    return reader(async signal => {
      const { value } = await client.threadTimeline(threadId, { limit: 40, ...(cursor === undefined ? {} : { cursor }) }, { signal })
      return Native.decodeUnknownSync(Native.ThreadTimelinePage)(value)
    }, follow === undefined ? undefined : { follow })
  })
  const deliveryRevision = Atom.make(0)
  const deliveryChanged = () => registry.set(deliveryRevision, registry.get(deliveryRevision) + 1)
  const invalidate = () => {
    registry.set(revision, registry.get(revision) + 1)
    deliveryChanged()
  }
  const drafts = Atom.family((_destination: string) => Atom.keepAlive(Atom.make<ThreadDraft>({ content: '' })))
  type Request = { id: string; idempotency_key: string; fence: { snapshot_id: string; subject_revisions: Record<string, string> } }
  type Attempt = {
    readonly fingerprint: string
    readonly key: string
    readonly target: string
    readonly actor: string | undefined
    readonly kind: string
    readonly start?: ThreadStartIntent
    readonly startDraft?: ThreadDraft
    run?: () => Promise<ThreadSendResult>
    retry?: BoundAction<Request>
    pending?: Promise<ThreadSendResult>
  }
  const attempts = new Map<string, Attempt>()
  // Outbox identity is independent of presentation: all thread actions retain their original
  // intent and transport after a lost receipt, just like ordinary conversation sends.
  const deliver = (
    destination: string,
    fingerprint: string,
    kind: string,
    target: () => string,
    prepare: (attempt: Attempt) => {
      readonly expected: ActionReceiptExpectation
      readonly send: (request: Request) => ReturnType<FractalClient['threadReply']>
      readonly revisions?: Record<string, string>
    },
    start?: ThreadStartIntent,
  ): Promise<ThreadSendResult> => {
    const reject = (detail: string): ThreadSendResult => ({ _tag: 'Refused', detail })
    const grant = registry.get(access)
    const previous = attempts.get(destination)
    if (previous !== undefined && previous.fingerprint !== fingerprint) {
      return Promise.resolve(reject('Fractal has not confirmed the previous action. Retry that action first.'))
    }
    if (previous?.pending !== undefined) return previous.pending
    if (!grant.actions.has(kind)) return Promise.resolve(reject('This connection cannot perform that thread action.'))
    const draft = start === undefined ? undefined : registry.get(drafts(destination))
    const attempt: Attempt = previous ?? { fingerprint, key: crypto.randomUUID(), target: target(), actor: grant.actor, kind,
      ...(start === undefined ? {} : { start }),
      ...(draft !== undefined && draft.content.trim() === start?.content && draft.quote === undefined ? { startDraft: draft } : {}),
    }
    attempts.set(destination, attempt)
    attempt.run = () => deliver(destination, fingerprint, kind, target, prepare, start)
    attempt.pending = (async () => {
      try {
        const prepared = prepare(attempt)
        const fence = async (): Promise<Request['fence']> => {
          const { snapshot, value } = await client.capabilities()
          if (value.session_actor !== attempt.actor) throw new Error('The connected account changed. Return to the original account to retry this action.')
          if (!value.capabilities.some(capability => capability.id === kind && capability.state === 'granted')
            || ((kind === 'thread.start' || kind === 'thread.reply') && !canSendUnsignedThread(value))) {
            throw new Error('This connection cannot perform that thread action.')
          }
          return { snapshot_id: snapshot.id, subject_revisions: prepared.revisions ?? {} }
        }
        const confirm = actionReceiptCheck(prepared.expected)
        const outcome = attempt.retry === undefined
          ? await sendBoundAction(attempt.actor ?? 'local', {
            build: async () => ({ id: `action/${attempt.key}`, idempotency_key: attempt.key, fence: await fence() }),
            fence,
            send: request => confirm(() => prepared.send(request)),
            retry: false,
            wait: ms => new Promise(resolve => setTimeout(resolve, ms)),
          })
          : await resendBoundAction(attempt.retry)
        if (outcome.how === 'unconfirmed') {
          attempt.retry = outcome.request
          return reject('Fractal has not confirmed this action. Retry to check the same action.')
        }
        attempts.delete(destination)
        return outcome.how === 'sent'
          ? { _tag: 'Sent', threadId: attempt.target }
          : reject(outcome.error instanceof ClientError ? outcome.error.response.message : outcome.error instanceof Error ? outcome.error.message : 'Fractal could not perform this action.')
      } finally {
        delete attempt.pending
        invalidate()
      }
    })()
    deliveryChanged()
    return attempt.pending
  }
  const send = (destination: string, parameters: Omit<ThreadStartParameters, 'target_id' | 'signature'> | Omit<ThreadReplyParameters, 'signature'>): Promise<ThreadSendResult> => {
    const intent = structuredClone(parameters)
    const kind = 'target_id' in intent ? 'thread.reply' : 'thread.start'
    return deliver(destination, JSON.stringify(intent), kind,
      () => 'target_id' in intent ? intent.target_id : `thread/${crypto.randomUUID()}`,
      attempt => ({
        expected: { kind: 'message', actionId: `action/${attempt.key}`, targetId: attempt.target },
        send: request => 'target_id' in intent
          ? client.threadReply({ ...request, parameters: intent })
          : client.threadStart({ ...request, parameters: { ...intent, target_id: attempt.target } }),
      }),
      'target_id' in intent ? undefined : intent,
    )
  }
  const pageKey = (subject: string, cursor?: string) => cursor === undefined ? subject : `${subject}\n${cursor}`
  const lifecycle = async (thread: ThreadObservation, action: string, parameters: object, edit: (
    request: Request & { parameters: { target_id: string } },
  ) => ReturnType<FractalClient['threadClose']>): Promise<string | undefined> => {
    const result = await deliver(`edit/${thread.summary.id}`, JSON.stringify({ action, parameters }), action,
      () => thread.summary.id,
      attempt => ({
        expected: { kind: 'metadata', actionId: `action/${attempt.key}`, targetId: thread.summary.id },
        revisions: { [thread.summary.id]: thread.summary.revision },
        send: request => edit({ ...request, parameters: { target_id: thread.summary.id } }),
      }),
    )
    return result._tag === 'Sent' ? undefined : result.detail
  }
  return {
    access,
    titles,
    pendingStarts: Atom.family((sourceId?: string) => Atom.make(get => {
      get(deliveryRevision)
      const grant = get(access)
      if (!grant.read) return []
      return [...attempts].flatMap(([draftKey, attempt]): RetainedThreadStart[] => {
        if (attempt.start === undefined || (sourceId !== undefined && attempt.start.origin.source_id !== sourceId) || attempt.actor !== grant.actor) return []
        return [{ draftKey, targetId: attempt.target, state: attempt.pending === undefined ? 'unconfirmed' : 'sending',
          parameters: structuredClone(attempt.start) }]
      })
    })),
    retryStart: async draftKey => {
      const attempt = attempts.get(draftKey)
      if (attempt?.start === undefined || attempt.run === undefined) {
        return { _tag: 'Refused', detail: 'There is no retained first send for this draft.' }
      }
      const result = await attempt.run()
      // A remounted recovery control cannot acknowledge a newer draft, even one with identical text.
      if (result._tag === 'Sent' && attempt.startDraft !== undefined && registry.get(drafts(draftKey)) === attempt.startDraft) {
        registry.set(drafts(draftKey), { content: '' })
      }
      return result
    },
    pendingChange: Atom.family((threadId: string) => Atom.make(get => {
      get(deliveryRevision)
      const attempt = attempts.get(`edit/${threadId}`)
      return attempt?.retry === undefined ? undefined : attempt.kind === 'thread.rename' ? 'rename' : attempt.kind === 'thread.close' ? 'close' : 'reopen'
    })),
    retryChange: async threadId => {
      const attempt = attempts.get(`edit/${threadId}`)
      if (attempt?.retry === undefined || attempt.run === undefined) return 'There is no unconfirmed thread change.'
      const result = await attempt.run()
      return result._tag === 'Sent' ? undefined : result.detail
    },
    list: query => readList(JSON.stringify(query)),
    detail: readDetail,
    timeline: (threadId, cursor) => readTimeline(pageKey(threadId, cursor)),
    draft: drafts,
    start: (draftKey, parameters) => send(draftKey, parameters),
    reply: parameters => send(parameters.target_id, parameters),
    rename: (thread, title, title_revision) => lifecycle(thread, 'thread.rename', { title, title_revision }, request => client.threadRename({
      ...request, parameters: { ...request.parameters, title, title_revision },
    })),
    changeState: (thread, state) => lifecycle(thread, state === 'closed' ? 'thread.close' : 'thread.reopen', { state }, request => state === 'closed'
      ? client.threadClose(request)
      : client.threadReopen(request)),
    markRead: async (thread, through, seen) => {
      const grant = registry.get(access)
      if (!grant.actions.has('thread.read') || seen.length === 0 || !seen.includes(through)) {
        return 'This connection cannot mark these replies read.'
      }
      const parameters = { target_id: thread.summary.id, through_message_id: through, seen_message_ids: [...seen] }
      const result = await deliver(`read/${thread.summary.id}/${through}`, JSON.stringify(parameters), 'thread.read',
        () => thread.summary.id,
        attempt => ({
          expected: { kind: 'read', actionId: `action/${attempt.key}`, readMessageIds: seen },
          send: request => client.threadRead({ ...request, parameters }),
        }),
      )
      return result._tag === 'Sent' ? undefined : result.detail
    },
  }
}
