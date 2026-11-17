import { actionReceiptCheck, appliedAnswer, sendBoundAction, resendBoundAction, type BoundAction, type ActionReceiptExpectation, ClientError, type FractalClient, type ThreadListQuery, type ThreadReplyParameters, type ThreadStartParameters } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import type { RequestHistoryPage, ThreadStartContext, ThreadSummary, ThreadSummaryPage, ThreadTimelineEvent, ThreadTimelinePage } from '@fractal/client/schema/read'
import type { ErrorEnvelope } from '@fractal/client/schema'
import type { FollowEvent, ThreadWindow } from '@fractal/sdk/read'
import { Effect, Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { hasConversationCapability } from '../data/conversationPort.ts'
import { clientReadAuthorization, credentialExpiredMessage, unavailable, waiting, type Feed } from '../data/source.ts'
import { gatewayUnreadActions, type UnreadActionPort } from './unread-action-port.ts'
import { readRequestHistoryPages, RequestHistoryPagingError } from './requestHistoryPages.ts'

export type ThreadObservation = { readonly summary: ThreadSummary; readonly snapshotId: string }

export const upgradeThreadEndpoint = 'Upgrade the connected Fractal to use threads. Your browser pairing is kept.'
const threadProtocolVersion = 3

export type ThreadDraft = {
  readonly content: string
  readonly quote?: { readonly id: string; readonly sender: string; readonly excerpt: string }
}

/**
 * A thread action that did not go through. `detail` is the sentence the reader sees first; `explanation` is
 * Fractal's own refusal, shown on its own line under it and never on its own.
 */
export type ThreadRefusal = { readonly detail: string; readonly explanation?: string }

export type ThreadSendResult =
  | { readonly _tag: 'Sent'; readonly threadId: string }
  | ({ readonly _tag: 'Refused' } & ThreadRefusal)

type ThreadActionKind = 'thread.start' | 'thread.reply' | 'thread.rename' | 'thread.close' | 'thread.reopen' | 'thread.read' | 'thread.mark-unread' | 'thread.clear-unread'
/** The sentence that names a refused action, so Fractal's reason is never shown without what it refused. */
const refusedAction: Readonly<Record<ThreadActionKind, string>> = {
  'thread.start': "Couldn't start this thread.",
  'thread.reply': "Couldn't send your reply.",
  'thread.rename': "Couldn't rename this thread.",
  'thread.close': "Couldn't close this thread.",
  'thread.reopen': "Couldn't reopen this thread.",
  'thread.read': "Couldn't mark these replies read.",
  'thread.mark-unread': "Couldn't mark this thread unread.",
  'thread.clear-unread': "Couldn't clear this unread reminder.",
}

/** A send that never reached Fractal: nothing changed, and the advice holds only until it reconnects. */
export const unreachableSend = 'Fractal could not be reached, so nothing changed. Try again once it reconnects.'

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

type ThreadAccess = {
  readonly read: boolean
  readonly inboxOrigins?: boolean
  readonly upgradeRequired?: boolean
  readonly actions: ReadonlySet<string>
  readonly actor?: string
  /** Fractal has not answered which thread operations this connection holds, because the path to it is lost: `read: false` is a default, not a refusal. */
  readonly unknown?: true
  /** The capabilities read completed and failed: a refused credential, or a broken answer (an HTTP error or one that does not decode). */
  readonly failure?: Feed<never>
}

export interface ThreadSource {
  readonly access: Atom.Atom<ThreadAccess>
  readonly titles: Atom.Atom<ReadonlyMap<string, string>>
  readonly list: (query: ThreadListQuery) => Atom.Atom<Feed<ThreadSummaryPage>>
  readonly detail: (threadId: string) => Atom.Atom<Feed<ThreadObservation>>
  readonly timeline: (threadId: string, cursor?: string) => Atom.Atom<Feed<ThreadTimelinePage>>
  /** Authorized original request episodes, read-only and separate from the conversation timeline. */
  readonly requestHistory?: (threadId: string) => Atom.Atom<Feed<RequestHistoryPage>>
  readonly reloadRequestHistory?: (threadId: string) => void
  /** Exact canonical first-send recipient; missing support never grants a guessed recipient. */
  readonly startContext?: (sourceId: string, episode: string) => Atom.Atom<Feed<ThreadStartContext>>
  readonly reloadStartContext?: (sourceId: string, episode: string) => void
  readonly draft: (destination: string) => Atom.Writable<ThreadDraft>
  readonly start: (draftKey: string, parameters: Omit<ThreadStartParameters, 'target_id' | 'signature'>) => Promise<ThreadSendResult>
  readonly pendingStarts?: (sourceId?: string) => Atom.Atom<readonly RetainedThreadStart[]>
  readonly retryStart?: (draftKey: string) => Promise<ThreadSendResult>
  readonly reply: (parameters: Omit<ThreadReplyParameters, 'signature'>) => Promise<ThreadSendResult>
  readonly rename: (thread: ThreadObservation, title: string, titleRevision: string) => Promise<ThreadRefusal | undefined>
  readonly changeState: (thread: ThreadObservation, state: ThreadSummary['state']) => Promise<ThreadRefusal | undefined>
  readonly pendingChange?: (threadId: string) => Atom.Atom<string | undefined>
  readonly retryChange?: (threadId: string) => Promise<ThreadRefusal | undefined>
  readonly markRead?: (thread: ThreadObservation, through: string, seen: readonly string[]) => Promise<ThreadRefusal | undefined>
  /** Explicit reminder operations are separate from natural thread.read receipts. */
  readonly unreadActions?: UnreadActionPort
  /** Read every mounted thread projection again, including one refused before. */
  readonly reload?: () => void
}

type Watch<Row> = (receive: (event: FollowEvent<ThreadWindow<Row>>) => void) => () => void
export interface ThreadWatches {
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
const unknownAccess: ThreadAccess = { ...noAccess, unknown: true }
// A revoked or expired browser credential needs pairing again; a missing grant does not.
const refused = (error: unknown): Feed<never> => {
  const reason = clientReadAuthorization(error) ?? 'failed'
  return unavailable({
    reason,
    detail: reason === 'unauthorized' ? credentialExpiredMessage
      : reason === 'ungranted' ? 'This connection has no access to threads.' : 'Fractal could not load threads.',
    ...(reason === 'failed' && error instanceof ClientError ? { code: error.response.code } : {}),
  })
}
// A connection that still reads threads can lose one of them: its 403 names that thread and keeps Fractal's reason.
const refusedThread = (error: unknown): Feed<never> => {
  const failure = refused(error)
  if (failure._tag !== 'Unavailable' || failure.reason !== 'ungranted' || !(error instanceof ClientError) || error.status !== 403) return failure
  const explanation = error.response.message.trim()
  return unavailable({ reason: 'ungranted', detail: 'You no longer have access to this thread.', ...(explanation === '' ? {} : { explanation }) })
}
// Only a lost path to Fractal (the network, or a gateway that cannot reach the daemon) leaves a read unanswered.
const lostPath = (error: unknown): boolean =>
  error instanceof TypeError || (error instanceof ClientError && (error.status === 502 || error.status === 503 || error.status === 504))
const logReadFailure = (message: string, error: unknown) => Effect.runSync(Effect.logWarning(message, {
  error: error instanceof ClientError ? `${error.status} ${error.response.code}` : error instanceof Error ? `${error.name}: ${error.message}` : String(error),
}))
const sameAccess = (a: ThreadAccess, b: ThreadAccess) => a.failure === undefined && b.failure === undefined
  && a.read === b.read && a.inboxOrigins === b.inboxOrigins && a.upgradeRequired === b.upgradeRequired
  && a.unknown === b.unknown && a.actor === b.actor && a.actions.size === b.actions.size && [...b.actions].every(action => a.actions.has(action))

/** `transportLive`, when given, re-reads access each time the gateway connection is live again, as grants do. */
export function gatewayThreads(client: FractalClient, registry: AtomRegistry.AtomRegistry, watches?: ThreadWatches, transportLive?: Atom.Atom<boolean>): ThreadSource {
  let observedAccess: ThreadAccess | undefined
  const access = Atom.make<ThreadAccess>(get => {
    // Retry (`reload`) reads access again too, so a failed capabilities read can recover.
    get(revision)
    // While disconnected the last answer stands; a read that failed then is retried once the connection is live.
    if (transportLive !== undefined && !get(transportLive) && observedAccess !== undefined) return observedAccess
    const abort = new AbortController()
    get.addFinalizer(() => abort.abort())
    void client.capabilities({ signal: abort.signal }).then(({ value }) => {
      const caps = Native.decodeUnknownSync(Native.Capabilities)(value)
      const upgradeRequired = ([['threads-v1', threadProtocolVersion], ['thread-summary-feed', 1]] as const).some(([id, version]) => !caps.capabilities.some(capability => capability.id === id && capability.version >= version))
      const inboxOrigins = caps.capabilities.some(capability => capability.id === 'threads-v1' && capability.version >= threadProtocolVersion && capability.state === 'granted')
      const read = inboxOrigins && hasConversationCapability(caps, 'read.projections') && hasConversationCapability(caps, 'thread-summary-feed')
      if (!abort.signal.aborted) {
        if (!read && registry.get(titles).size > 0) registry.set(titles, new Map())
        const actions = new Set(read ? caps.capabilities.filter(capability =>
          capability.state === 'granted' && capability.id.startsWith('thread.')
          && (canSendUnsignedThread(caps) || (capability.id !== 'thread.start' && capability.id !== 'thread.reply'))
        ).map(capability => capability.id) : [])
        const answered = { read, inboxOrigins, upgradeRequired, actions, actor: caps.session_actor }
        // An unchanged answer keeps its identity, so reading access again does not read every thread projection twice.
        if (observedAccess === undefined || !sameAccess(observedAccess, answered)) observedAccess = answered
        get.setSelf(observedAccess)
      }
    }).catch(error => {
      if (abort.signal.aborted) return
      if (clientReadAuthorization(error) !== undefined) {
        get.setSelf(observedAccess = { ...noAccess, failure: refused(error) })
        return
      }
      // Unanswered over a lost path: unknown until the connection is live again, which reads access anew.
      // A lost path while the connection reports Live would never be retried that way, so it is a failure.
      if (lostPath(error) && !(transportLive !== undefined && registry.get(transportLive))) {
        get.setSelf(observedAccess ?? unknownAccess)
        return
      }
      // A completed but broken read (an HTTP error, an answer that does not decode) is shown with Retry and recorded.
      logReadFailure('Thread access read failed', error)
      get.setSelf({ ...noAccess, failure: refused(error) })
    })
    return observedAccess ?? unknownAccess
  })
  const titles = Atom.keepAlive(Atom.make<ReadonlyMap<string, string>>(new Map()))
  const rememberTitles = (items: readonly ThreadSummary[]) => {
    const previous = registry.get(titles)
    const changed = items.filter(item => previous.get(item.id) !== item.title)
    if (changed.length > 0) registry.set(titles, new Map([...previous, ...changed.map(item => [item.id, item.title] as const)]))
  }
  // Patch existing mounted pages, including continuation pages without their own feed.
  // The pages remain the summary cache; this set only owns their live subscriptions.
  const summaryListeners = new Set<(id: string, summary: ThreadSummary | undefined) => void>()
  const observeSummary = (summary: ThreadSummary) => {
    rememberTitles([summary])
    for (const receive of summaryListeners) receive(summary.id, summary)
  }
  const forgetSummary = (id: string) => {
    const previous = registry.get(titles)
    if (previous.has(id)) {
      const next = new Map(previous)
      next.delete(id)
      registry.set(titles, next)
    }
    for (const receive of summaryListeners) receive(id, undefined)
  }
  const revision = Atom.make(0)
  function reader<T>(
    read: (signal: AbortSignal) => Promise<T>,
    settings?: {
      readonly follow?: (publish: (value: T) => void, reload: () => void, stale: () => void) => () => void
      readonly onObserved?: (value: T) => void
      readonly onFailure?: (error: unknown) => void
      /** How a failed read reads; one thread's own reads name that thread when it is refused. */
      readonly refused?: (error: unknown) => Feed<never>
      /** History reauthorizes every open/refresh; discard its previous cut before requesting a new one. */
      readonly resetOnReload?: boolean
      readonly updates?: (update: (change: (value: T) => T) => void) => () => void
    },
  ): Atom.Atom<Feed<T>> {
    let previous: Feed<T> = waiting
    return Atom.make(get => {
      const demand = get(access)
      get(revision)
      if (settings?.resetOnReload) previous = waiting
      if (!demand.read) return previous = waiting
      // A new read of a refused projection is loading again, not still refused.
      if (previous._tag === 'Unavailable') previous = waiting
      let active = true
      let abort: AbortController | undefined
      const publish = (value: T, freshness: 'live' | 'stale' = 'live') => {
        if (!active) return
        settings?.onObserved?.(value)
        get.setSelf(previous = { _tag: 'Observed', value, freshness })
      }
      const stopUpdates = settings?.updates?.(change => {
        if (!active || previous._tag !== 'Observed') return
        const value = change(previous.value)
        if (value === previous.value) return
        const freshness = previous.freshness
        // A page request started before this exact observation cannot restore its old row.
        abort?.abort()
        publish(value, freshness)
      })
      const reload = () => {
        if (!active) return
        abort?.abort()
        const current = new AbortController()
        abort = current
        void read(current.signal).then(value => {
          if (!current.signal.aborted) publish(value)
        }, error => {
          if (!current.signal.aborted && active) {
            // A revoked read cannot be revived by a later frame from this demand.
            if (error instanceof ClientError && (error.status === 401 || error.status === 403)) active = false
            settings?.onFailure?.(error)
            const failure = (settings?.refused ?? refused)(error)
            // Only a lost path to Fractal keeps what was observed, marked paused. A decode, validation or other
            // HTTP failure is shown and recorded instead.
            const lost = failure._tag === 'Unavailable' && failure.reason === 'failed' && lostPath(error)
            if (failure._tag === 'Unavailable' && failure.reason === 'failed' && !lost) logReadFailure('Thread read failed', error)
            get.setSelf(previous = previous._tag === 'Observed' && lost ? { ...previous, freshness: 'stale' } : failure)
          }
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
        stopUpdates?.()
      })
      return previous
    })
  }
  function followPage<Row>(
    watch: Watch<Row> | undefined,
  ): ((publish: (value: { readonly items: readonly Row[]; readonly next_cursor: Option.Option<string> }) => void, reload: () => void, stale: () => void) => () => void) | undefined {
    return watch === undefined ? undefined : (publish, reload, stale) => watch(event => {
      if (event._tag !== 'Observed') {
        if (event._tag === 'Failed' && event.error._tag === 'Rejected') reload()
        else stale()
        return
      }
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
        publish({ summary, snapshotId: event.value.snapshotId })
      })
    }
    return reader(async signal => {
      const { value, snapshot } = await client.threadGet(threadId, { signal })
      const summary = Native.decodeUnknownSync(Native.ThreadSummary)(value)
      // Not a TypeError: that class means the network failed, and this observation is wrong, not paused.
      if (summary.id !== threadId) throw new Error('Mismatched thread observation')
      signal.throwIfAborted()
      return { summary, snapshotId: snapshot.id }
    }, {
      follow,
      refused: refusedThread,
      onObserved: ({ summary }) => observeSummary(summary),
      onFailure: error => {
        if (error instanceof ClientError && (error.status === 401 || error.status === 403)) forgetSummary(threadId)
      },
    })
  })
  const readList = Atom.family((key: string) => {
    const query = JSON.parse(key) as ThreadListQuery
    const follow = followPage(query.cursor === undefined ? watches?.summaries(query) : undefined)
    return reader(async signal => {
      const { value } = await client.threadList({ limit: 40, ...query }, { signal })
      const page = Native.decodeUnknownSync(Native.ThreadSummaryPage)(value)
      if ('agent_id' in query && page.items.some(item => item.agent_id !== query.agent_id)) throw new Error('Mismatched thread list')
      signal.throwIfAborted()
      return page
    }, {
      ...(follow === undefined ? {} : { follow }),
      onObserved: page => rememberTitles(page.items),
      updates: update => {
        const receive = (id: string, summary: ThreadSummary | undefined) => update(page => {
          if (!page.items.some(item => item.id === id)) return page
          return {
            ...page,
            items: summary === undefined ? page.items.filter(item => item.id !== id)
              : page.items.map(item => item.id === id ? summary : item),
          }
        })
        summaryListeners.add(receive)
        return () => { summaryListeners.delete(receive) }
      },
    })
  })
  const readTimeline = Atom.family((key: string) => {
    const [threadId, cursor] = key.split('\n') as [string, string | undefined]
    const follow = followPage(cursor === undefined ? watches?.timeline(threadId) : undefined)
    return reader(async signal => {
      const { value } = await client.threadTimeline(threadId, { limit: 40, ...(cursor === undefined ? {} : { cursor }) }, { signal })
      return Native.decodeUnknownSync(Native.ThreadTimelinePage)(value)
    }, { refused: refusedThread, ...(follow === undefined ? {} : { follow }) })
  })
  const readHistory = Atom.family((threadId: string) => reader(
    signal => readRequestHistoryPages(async cursor => {
      const { value } = await client.threadRequestHistory(threadId, { limit: 40, ...(cursor === undefined ? {} : { cursor }) }, { signal })
      signal.throwIfAborted()
      return Native.decodeUnknownSync(Native.RequestHistoryPage)(value)
    }),
    {
      resetOnReload: true,
      refused: error => error instanceof RequestHistoryPagingError
        ? unavailable({ reason: 'failed', detail: error.message, retryable: true })
        : refusedThread(error),
    },
  ))
  const readStartContext = Atom.family((sourceId: string) => Atom.family((episode: string) => reader(async signal => {
    const { value } = await client.threadStartContext(sourceId, episode, { signal })
    signal.throwIfAborted()
    const context = Native.decodeUnknownSync(Native.ThreadStartContext)(value)
    if (context.source_id !== sourceId || context.episode !== episode) throw new Error('Mismatched thread start context')
    return context
  }, {
    resetOnReload: true,
    refused: error => {
      const failure = refused(error)
      if (failure._tag !== 'Unavailable' || failure.reason !== 'failed') return failure
      const explanation = error instanceof ClientError ? error.response.message.trim() : ''
      return { ...failure, detail: 'Fractal could not load the first-send recipient.',
        ...(explanation === '' ? {} : { explanation }) }
    },
  })))
  const reloadStartContext = (sourceId: string, episode: string): Promise<void> => {
    const atom = readStartContext(sourceId)(episode)
    registry.refresh(atom)
    // Finish the canonical reread even if the rejected composer is unmounted.
    let resolve!: () => void
    const promise = new Promise<void>(complete => { resolve = complete })
    const stop = registry.subscribe(atom, feed => {
      if (feed._tag === 'Waiting' && registry.get(access).read) return
      queueMicrotask(() => { stop(); resolve() })
    }, { immediate: true })
    return promise
  }
  const soughtPages = Atom.family((_threadId: string) => Atom.make<{
    readonly page: ThreadTimelinePage
    readonly actor: string | undefined
  } | undefined>(undefined))
  const firstTimeline = Atom.family((threadId: string) => Atom.make<Feed<ThreadTimelinePage>>(get => {
    const grant = get(access)
    const page = get(soughtPages(threadId))
    const latest = get(readTimeline(threadId))
    if (!grant.read) return waiting
    if (page === undefined || page.actor !== grant.actor || latest._tag === 'Unavailable') return latest
    const stale = (transportLive !== undefined && !get(transportLive))
      || (latest._tag === 'Observed' && latest.freshness === 'stale')
    return { _tag: 'Observed', value: page.page, freshness: stale ? 'stale' : 'live' }
  }))
  const deliveryRevision = Atom.make(0)
  const deliveryChanged = () => registry.set(deliveryRevision, registry.get(deliveryRevision) + 1)
  const invalidate = () => {
    registry.set(revision, registry.get(revision) + 1)
    deliveryChanged()
  }
  const drafts = Atom.family((_destination: string) => Atom.keepAlive(Atom.make<ThreadDraft>({ content: '' })))
  type Request = { id: string; idempotency_key: string; fence: { snapshot_id: string; subject_revisions: Record<string, string> } }
  const recipientUnassigned: Extract<ErrorEnvelope, { readonly code: 'recipient-unassigned' }>['code'] = 'recipient-unassigned'
  type Attempt = {
    readonly fingerprint: string
    readonly key: string
    readonly target: string
    readonly actor: string | undefined
    readonly kind: ThreadActionKind
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
    kind: ThreadActionKind,
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
            || ((kind === 'thread.mark-unread' || kind === 'thread.clear-unread')
              && !['thread.mark-unread', 'thread.clear-unread'].every(id => value.capabilities.some(capability => capability.id === id && capability.state === 'granted')))
            || ((kind === 'thread.start' || kind === 'thread.reply') && !canSendUnsignedThread(value))) {
            throw new Error('This connection cannot perform that thread action.')
          }
          return { snapshot_id: snapshot.id, subject_revisions: prepared.revisions ?? {} }
        }
        const confirm = actionReceiptCheck(prepared.expected)
        let signedRequest: Request | undefined
        const ports = {
          build: async (): Promise<Request> => {
            signedRequest = { id: `action/${attempt.key}`, idempotency_key: attempt.key, fence: await fence() }
            return signedRequest
          },
          fence,
          send: (request: Request) => confirm(() => prepared.send(request)),
          retry: false as const,
          wait: (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms)),
        }
        const outcome = attempt.retry === undefined
          ? await sendBoundAction(attempt.actor ?? 'local', ports)
          : await resendBoundAction(attempt.retry)
        // A signed Start needs positive nonapplication evidence. A refusal code (even stale-fence)
        // or retryable:false cannot authorize discarding its packet and choosing a new recipient.
        if (kind === 'thread.start' && signedRequest !== undefined && outcome.how === 'failed'
          && outcome.error instanceof ClientError && appliedAnswer(outcome.error) !== 'none') {
          attempt.retry = { member: attempt.actor ?? 'local', request: signedRequest, ports }
          return reject('Fractal has not confirmed this action. Retry to check the same action.')
        }
        if (outcome.how === 'unconfirmed') {
          attempt.retry = outcome.request
          return reject('Fractal has not confirmed this action. Retry to check the same action.')
        }
        attempts.delete(destination)
        if (outcome.how === 'sent') return { _tag: 'Sent', threadId: attempt.target }
        // A failed outcome applied nothing. A gateway or network failure says so in words; a daemon refusal names the
        // refused action, with Fractal's own reason under it.
        const error = outcome.error
        if (error instanceof ClientError && error.status < 500) {
          if (kind === 'thread.start' && start !== undefined && appliedAnswer(error) === 'none' && (
            (error.status === 404 && error.response.code === 'not-found')
            || (error.status === 409 && (error.response.code === 'stale-fence' || error.response.code === 'recipient-retired' || error.response.code === recipientUnassigned))
          )) {
            // The definitive refusal applied nothing. Re-read visibly; only a new explicit Send may use the new preview.
            await reloadStartContext(start.origin.source_id, start.origin.episode)
          }
          const explanation = error.response.message.trim()
          return { _tag: 'Refused', detail: refusedAction[kind], ...(explanation === '' ? {} : { explanation }) }
        }
        return reject(error instanceof ClientError || error instanceof TypeError ? unreachableSend
          : error instanceof Error ? error.message : 'Fractal could not perform this action.')
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
  const refusal = (result: ThreadSendResult): ThreadRefusal | undefined => result._tag === 'Sent' ? undefined
    : { detail: result.detail, ...(result.explanation === undefined ? {} : { explanation: result.explanation }) }
  const lifecycle = async (thread: ThreadObservation, action: 'thread.rename' | 'thread.close' | 'thread.reopen', parameters: object, edit: (
    request: Request & { parameters: { target_id: string } },
  ) => ReturnType<FractalClient['threadClose']>): Promise<ThreadRefusal | undefined> => {
    const result = await deliver(`edit/${thread.summary.id}`, JSON.stringify({ action, parameters }), action,
      () => thread.summary.id,
      attempt => ({
        expected: { kind: 'metadata', actionId: `action/${attempt.key}`, targetId: thread.summary.id },
        revisions: { [thread.summary.id]: thread.summary.revision },
        send: request => edit({ ...request, parameters: { target_id: thread.summary.id } }),
      }),
    )
    return refusal(result)
  }
  return {
    access,
    titles,
    unreadActions: gatewayUnreadActions({
      client, registry, access, detail: readDetail,
      onFound: (threadId, page) => registry.set(soughtPages(threadId), { page, actor: registry.get(access).actor }),
      send: async (thread, edit) => {
        const parameters = {
          target_id: thread.summary.id,
          observed_marks: [...edit.observed_marks],
          ...(edit.kind === 'thread.mark-unread' ? { anchor_message_id: edit.anchor_message_id } : {}),
        }
        const result = await deliver(`unread/${thread.summary.id}`, JSON.stringify({ kind: edit.kind, parameters }), edit.kind,
          () => thread.summary.id,
          attempt => ({
            expected: { kind: 'unread', actionId: `action/${attempt.key}`, targetId: thread.summary.id },
            revisions: { [thread.summary.id]: thread.summary.revision },
            send: request => edit.kind === 'thread.mark-unread'
              ? client.threadMarkUnread({ ...request, parameters: { ...parameters, anchor_message_id: edit.anchor_message_id } })
              : client.threadClearUnread({ ...request, parameters }),
          }),
        )
        return result._tag === 'Sent' ? { _tag: 'Sent' } : { _tag: 'Refused', detail: result.detail }
      },
    }),
    reload: () => registry.set(revision, registry.get(revision) + 1),
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
      if (attempt?.retry === undefined || attempt.run === undefined) return { detail: 'There is no unconfirmed thread change.' }
      return refusal(await attempt.run())
    },
    list: query => readList(JSON.stringify(query)),
    detail: readDetail,
    requestHistory: readHistory,
    reloadRequestHistory: threadId => registry.refresh(readHistory(threadId)),
    startContext: (sourceId, episode) => readStartContext(sourceId)(episode),
    reloadStartContext: (sourceId, episode) => { void reloadStartContext(sourceId, episode) },
    timeline: (threadId, cursor) => cursor === undefined ? firstTimeline(threadId) : readTimeline(pageKey(threadId, cursor)),
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
        return { detail: 'This connection cannot mark these replies read.' }
      }
      const parameters = { target_id: thread.summary.id, through_message_id: through, seen_message_ids: [...seen] }
      const result = await deliver(`read/${thread.summary.id}/${through}`, JSON.stringify(parameters), 'thread.read',
        () => thread.summary.id,
        attempt => ({
          expected: { kind: 'read', actionId: `action/${attempt.key}`, readMessageIds: seen },
          send: request => client.threadRead({ ...request, parameters }),
        }),
      )
      return refusal(result)
    },
  }
}
