import type { OutboxCopyOutcome, OutboxRecoveryCopy, OutboxRecoveryItem, OutboxRecoveryPort } from '../data/source.ts'

/** Row operations; at most one runs per attempt id. */
export type RecoveryOperation = 'discard' | 'retry' | 'send' | 'copy'

/** The outbox list as last read: storage that cannot be read is a notice, never an empty or damaged list. */
export type RecoveryList =
  | { readonly _tag: 'Checking' }
  | { readonly _tag: 'Unavailable' }
  | { readonly _tag: 'Listed'; readonly items: readonly OutboxRecoveryItem[] }

export interface RecoveryView {
  readonly list: RecoveryList
  /** Attempt id → its running operation. */
  readonly busy: ReadonlyMap<string, RecoveryOperation>
  /** Attempt id → the operation that last failed for it; cleared when the id leaves the list or another operation starts. */
  readonly failed: ReadonlyMap<string, RecoveryOperation>
}

export interface RecoveryStore {
  readonly subscribe: (listener: () => void) => () => void
  readonly snapshot: () => RecoveryView
  /** A mounted surface: the first holder lists and follows the port's change wakeups; the last releases them. */
  readonly acquire: () => () => void
  readonly discard: (id: string) => Promise<void>
  /** Not-applied attempts only: the port starts a fresh attempt. */
  readonly retry: (id: string) => Promise<void>
  /** Pending attempts only: the port revalidates recipient and access before it permits the send. */
  readonly send: (id: string) => Promise<void>
  /** The original's text and retained images for a separate new attempt; `undefined` when unreadable. */
  readonly copy: (id: string) => Promise<OutboxRecoveryCopy | undefined>
  /**
   * Sends the composer's copy under its one `copyKey`, so pressing Send again can never start a second attempt.
   * `Refused` means nothing was saved and the composer may try again. `Recorded` means a new attempt holds the key,
   * whatever its delivery outcome, and the list now owns it. A thrown error counts as `Recorded`, since the copy may
   * have been saved.
   */
  readonly sendCopy: (id: string, copyKey: string, text: string) => Promise<OutboxCopyOutcome>
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
  /** Runs one operation for an attempt the current list still shows with an eligible kind, then re-reads the list. */
  const run = async <A>(id: string, operation: RecoveryOperation, kinds: readonly OutboxRecoveryItem['kind'][], call: () => Promise<A>): Promise<{ readonly ok: true; readonly value: A } | { readonly ok: false }> => {
    const item = listed(id)
    if (item === undefined || !kinds.includes(item.kind) || view.busy.has(id)) return { ok: false }
    set({ busy: new Map([...view.busy, [id, operation]]), failed: new Map([...view.failed].filter(([key]) => key !== id)) })
    let result: { readonly ok: true; readonly value: A } | { readonly ok: false }
    try { result = { ok: true, value: await call() } } catch { result = { ok: false } }
    set({
      busy: new Map([...view.busy].filter(([key]) => key !== id)),
      ...(result.ok ? {} : { failed: new Map([...view.failed, [id, operation]]) }),
    })
    if (operation !== 'copy') await list()
    return result
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
    discard: async id => { await run(id, 'discard', ['unknown', 'not-applied', 'pending', 'corrupt', 'identity-mismatch'], () => port.discard(id)) },
    retry: async id => { const retry = port.retry; if (retry !== undefined) await run(id, 'retry', ['not-applied'], () => retry(id)) },
    send: async id => { const send = port.send; if (send !== undefined) await run(id, 'send', ['pending'], () => send(id)) },
    copy: async id => {
      const copy = port.copy
      if (copy === undefined) return undefined
      const result = await run(id, 'copy', ['unknown'], () => copy(id))
      return result.ok ? result.value : undefined
    },
    sendCopy: async (id, copyKey, text) => {
      const sendCopy = port.sendCopy
      if (sendCopy === undefined) return { _tag: 'Refused', detail: 'This outbox cannot send copies.' }
      let outcome: OutboxCopyOutcome
      try { outcome = await sendCopy(id, copyKey, text) } catch { outcome = { _tag: 'Recorded' } }
      if (outcome._tag === 'Recorded') await list()
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
