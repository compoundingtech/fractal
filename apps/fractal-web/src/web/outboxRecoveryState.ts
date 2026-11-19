import type { OutboxCopyOutcome, OutboxRecoveryCopy, OutboxRecoveryItem, OutboxRecoveryPort } from '../data/source.ts'

/** Row operations; at most one runs per attempt id. */
export type RecoveryOperation = 'discard' | 'retry' | 'send' | 'copy'

/** The outbox list as last read: storage that cannot be read is a notice, never an empty or damaged list. */
export type RecoveryList =
  | { readonly _tag: 'Checking' }
  | { readonly _tag: 'Unavailable' }
  | { readonly _tag: 'Listed'; readonly items: readonly OutboxRecoveryItem[] }

/**
 * Why a row's last operation did not happen. `stale`: the attempt's revision moved past the one the person acted on
 * (another tab sent, retried or discarded it), so nothing was done and the row now shows its current state.
 */
export interface RecoveryFailure {
  readonly operation: RecoveryOperation
  readonly cause: 'error' | 'stale'
}

export interface RecoveryView {
  readonly list: RecoveryList
  /** Attempt id → its running operation. */
  readonly busy: ReadonlyMap<string, RecoveryOperation>
  /** Attempt id → why its last operation did not happen; cleared when the id leaves the list or another operation starts. */
  readonly failed: ReadonlyMap<string, RecoveryFailure>
}

/**
 * One command's answer. `Stale` means the persisted attempt is no longer at the observed revision: the store refused
 * locally or the outbox refused atomically, nothing changed, and the list was read again. `Skipped`: the attempt is not
 * listed, is busy, or its listed state does not allow the command, so the port was not called.
 */
export type RecoveryOutcome<A = void> =
  | { readonly _tag: 'Done'; readonly value: A }
  | { readonly _tag: 'Stale' }
  | { readonly _tag: 'Failed' }
  | { readonly _tag: 'Skipped' }

/** A copy's send answer; `Stale` means the original moved past the revision the copy was opened from and nothing was saved. */
export type RecoveryCopyOutcome = OutboxCopyOutcome | { readonly _tag: 'Stale' }

/**
 * The outbox's typed refusal of a command whose observed revision no longer matches the persisted attempt. Identified
 * structurally by its tag, so the page does not load the outbox SDK to recognize it.
 */
export const isOutboxConflict = (cause: unknown): cause is { readonly _tag: 'OutboxConflict' } =>
  typeof cause === 'object' && cause !== null && '_tag' in cause && cause._tag === 'OutboxConflict'

/**
 * Every command names the attempt and the revision the person saw (`undefined` only for a damaged entry with no
 * readable revision). The store never substitutes a later revision: a command against a row the list has since
 * re-read at another revision is refused as `Stale` without calling the port, and the port refuses atomically otherwise.
 */
export interface RecoveryStore {
  readonly subscribe: (listener: () => void) => () => void
  readonly snapshot: () => RecoveryView
  /** A mounted surface: the first holder lists and follows the port's change wakeups; the last releases them. */
  readonly acquire: () => () => void
  /** Never retried: a `Stale` answer needs the person to confirm again against the attempt's current state. */
  readonly discard: (id: string, expectedRevision: number | undefined) => Promise<RecoveryOutcome>
  /** Not-applied attempts only: the port starts a fresh attempt. */
  readonly retry: (id: string, expectedRevision: number | undefined) => Promise<RecoveryOutcome>
  /** Pending attempts only: the port revalidates recipient and access before it permits the send. */
  readonly send: (id: string, expectedRevision: number | undefined) => Promise<RecoveryOutcome>
  /** The original's text and retained images for a separate new attempt; the port pins `expectedRevision` to its copy key. */
  readonly copy: (id: string, expectedRevision: number | undefined) => Promise<RecoveryOutcome<OutboxRecoveryCopy>>
  /**
   * Sends the composer's copy under its one `copyKey`, so pressing Send again can never start a second attempt.
   * `Refused` means nothing was saved and the composer may try again. `Stale` means the original changed since the copy
   * opened and nothing was saved. `Recorded` means a new attempt holds the key, whatever its delivery outcome, and the
   * list now owns it. Any other thrown error counts as `Recorded`, since the copy may have been saved.
   */
  readonly sendCopy: (id: string, copyKey: string, text: string) => Promise<RecoveryCopyOutcome>
}

/** The actions a row may offer: what the outbox granted for its kind, and only those this port implements. */
export const offeredActions = (item: OutboxRecoveryItem, port: OutboxRecoveryPort): readonly RecoveryOperation[] => {
  const byKind: Readonly<Record<OutboxRecoveryItem['kind'], readonly RecoveryOperation[]>> = {
    // An unconfirmed original may have arrived: it is never retried or re-sent, only copied into a new attempt.
    unknown: ['copy', 'discard'],
    'not-applied': ['retry', 'discard'],
    pending: ['send', 'discard'],
    corrupt: ['discard'],
    'identity-mismatch': ['discard'],
  }
  return byKind[item.kind].filter(action => item.actions.includes(action)
    && port[action] !== undefined
    && (action !== 'copy' || port.sendCopy !== undefined))
}

export const createRecoveryStore = (port: OutboxRecoveryPort): RecoveryStore => {
  let view: RecoveryView = { list: { _tag: 'Checking' }, busy: new Map(), failed: new Map() }
  let holders = 0
  let unfollow: (() => void) | undefined
  // Only the newest list answer is applied; an older one that resolves later is dropped.
  let generation = 0
  const listeners = new Set<() => void>()
  const set = (next: Partial<RecoveryView>) => {
    view = { ...view, ...next }
    for (const listener of listeners) listener()
  }
  const list = async () => {
    const at = ++generation
    let items: readonly OutboxRecoveryItem[]
    try { items = await port.list() } catch {
      if (at === generation) set({ list: { _tag: 'Unavailable' } })
      return
    }
    if (at !== generation) return
    const ids = new Set(items.map(item => item.id))
    set({ list: { _tag: 'Listed', items }, failed: new Map([...view.failed].filter(([id]) => ids.has(id))) })
  }
  const listed = (id: string) => view.list._tag === 'Listed' ? view.list.items.find(item => item.id === id) : undefined
  /**
   * Runs one operation for the observed revision of an attempt the current list still shows with an eligible kind,
   * then re-reads the list. A row already re-read at another revision is stale before any port call.
   */
  const run = async <A>(id: string, expectedRevision: number | undefined, operation: RecoveryOperation, kinds: readonly OutboxRecoveryItem['kind'][], call: () => Promise<A>): Promise<RecoveryOutcome<A>> => {
    const item = listed(id)
    if (item === undefined || view.busy.has(id)) return { _tag: 'Skipped' }
    if (item.revision !== expectedRevision) {
      set({ failed: new Map<string, RecoveryFailure>([...view.failed, [id, { operation, cause: 'stale' }]]) })
      return { _tag: 'Stale' }
    }
    if (!kinds.includes(item.kind)) return { _tag: 'Skipped' }
    set({ busy: new Map([...view.busy, [id, operation]]), failed: new Map([...view.failed].filter(([key]) => key !== id)) })
    let outcome: RecoveryOutcome<A>
    try { outcome = { _tag: 'Done', value: await call() } } catch (cause) { outcome = isOutboxConflict(cause) ? { _tag: 'Stale' } : { _tag: 'Failed' } }
    set({
      busy: new Map([...view.busy].filter(([key]) => key !== id)),
      ...(outcome._tag === 'Done' ? {} : { failed: new Map<string, RecoveryFailure>([...view.failed, [id, { operation, cause: outcome._tag === 'Stale' ? 'stale' : 'error' }]]) }),
    })
    // Every answer but an opened copy re-reads, so a stale or failed row shows its current state.
    if (operation !== 'copy' || outcome._tag !== 'Done') await list()
    return outcome
  }
  return {
    subscribe: listener => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    snapshot: () => view,
    acquire: () => {
      holders += 1
      if (holders === 1) {
        // A wakeup carries no authority: it only asks for a fresh read of the persisted outbox.
        unfollow = port.subscribe?.(() => { void list() })
        void list()
      }
      let released = false
      return () => {
        if (released) return
        released = true
        holders -= 1
        if (holders === 0) { unfollow?.(); unfollow = undefined }
      }
    },
    discard: (id, expectedRevision) => run(id, expectedRevision, 'discard', ['unknown', 'not-applied', 'pending', 'corrupt', 'identity-mismatch'], () => port.discard(id, expectedRevision)),
    retry: async (id, expectedRevision) => {
      const retry = port.retry
      return retry === undefined ? { _tag: 'Skipped' } : run(id, expectedRevision, 'retry', ['not-applied'], () => retry(id, expectedRevision))
    },
    send: async (id, expectedRevision) => {
      const send = port.send
      return send === undefined ? { _tag: 'Skipped' } : run(id, expectedRevision, 'send', ['pending'], () => send(id, expectedRevision))
    },
    copy: async (id, expectedRevision) => {
      const copy = port.copy
      return copy === undefined ? { _tag: 'Skipped' } : run(id, expectedRevision, 'copy', ['unknown'], () => copy(id, expectedRevision))
    },
    sendCopy: async (id, copyKey, text) => {
      const sendCopy = port.sendCopy
      if (sendCopy === undefined) return { _tag: 'Refused', detail: 'This outbox cannot send copies.' }
      let outcome: RecoveryCopyOutcome
      try { outcome = await sendCopy(id, copyKey, text) } catch (cause) { outcome = isOutboxConflict(cause) ? { _tag: 'Stale' } : { _tag: 'Recorded' } }
      if (outcome._tag !== 'Refused') await list()
      return outcome
    },
  }
}

const stores = new WeakMap<OutboxRecoveryPort, RecoveryStore>()
/** The page's store for its source's port; one per port, however many surfaces mount. */
export const recoveryStoreFor = (port: OutboxRecoveryPort): RecoveryStore => {
  let store = stores.get(port)
  if (store === undefined) {
    store = createRecoveryStore(port)
    stores.set(port, store)
  }
  return store
}
