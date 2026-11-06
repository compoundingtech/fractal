import type { ActionOf, Arrangement, ArrangementOperation, ErrorEnvelope } from '@fractal/client'
import { ErrorCode, decodeUnknownSync, type ActionResult } from '@fractal/client/schema'
import type { ActionFailure } from '@fractal/sdk/mutations'
import type { SdkChunkFailure } from '../data/lazySdk.ts'
import { Deferred, Effect, Fiber, Semaphore, SubscriptionRef } from 'effect'
import { reservedSidebarSubject, sidebarCandidates, sidebarWinner, type ArrangementInventory } from '../data/arrangements.ts'
import { prepareArrangementRetry, stageArrangementEdit, type EditJob } from './editRequest.ts'
import { makeOptimisticMutations, MutationRefused } from '../data/optimisticMutation.ts'

/** Loading errors are refused before submission, and the existing edit retry reacquires the chunk. */
export interface ArrangementActionPort {
  readonly snapshot: Effect.Effect<string, ActionFailure | SdkChunkFailure>
  readonly submitAction: (request: ActionOf<'arrangement.edit'>) =>
    Effect.Effect<typeof ActionResult.Type, ActionFailure | SdkChunkFailure>
}

/** Only folder/placement operations are user-editable: Sidebar name and retirement are not. */
export type SidebarOperation = Extract<ArrangementOperation, { op: 'folder.create' | 'folder.rename' | 'folder.move' | 'folder.delete' | 'subject.place' }>
type Fields<TOp extends SidebarOperation['op']> = Omit<Extract<SidebarOperation, { op: TOp }>, 'op'>
export const sidebarOperations = {
  createFolder: (fields: Fields<'folder.create'>): SidebarOperation => ({ ...fields, op: 'folder.create' }),
  renameFolder: (fields: Fields<'folder.rename'>): SidebarOperation => ({ ...fields, op: 'folder.rename' }),
  moveFolder: (fields: Fields<'folder.move'>): SidebarOperation => ({ ...fields, op: 'folder.move' }),
  deleteFolder: (fields: Fields<'folder.delete'>): SidebarOperation => ({ ...fields, op: 'folder.delete' }),
  placeSubject: (fields: Fields<'subject.place'>): SidebarOperation => ({ ...fields, op: 'subject.place' }),
}

/** Lowercase UUIDv7 for new folders, generated only in memory. */
export const arrangementUuid = (): string => {
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  let time = BigInt(Date.now())
  for (let index = 5; index >= 0; index--) { bytes[index] = Number(time & 255n); time >>= 8n }
  bytes[6] = (bytes[6]! & 15) | 112
  bytes[8] = (bytes[8]! & 63) | 128
  const hex = [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

export interface ArrangementAttempt {
  readonly id: string
  readonly label: string
  readonly kind: 'folder' | 'agent'
}
export type ArrangementRefusal = {
  readonly retryableLoad?: boolean
  readonly reason: { readonly _tag: 'Known'; readonly code: Extract<typeof ErrorCode.Type, string> } | { readonly _tag: 'Unknown'; readonly code?: string }
  readonly detail: string
  readonly targets: readonly string[]
  readonly error?: ErrorEnvelope
  /** Admission found the subject, but its live inventory row is not visible yet. */
  readonly awaitingVisibility?: boolean
  /** Ephemeral request labels survive rollback, deletion and view filtering. */
  readonly attemptedTargets?: readonly ArrangementAttempt[]
  /** Fixed client copy: restaging this intent against fresh state is invalid, so retrying cannot help. */
  readonly restageRefusal?: string
}
export const arrangementRefusal = (failure: ActionFailure | SdkChunkFailure, targets: readonly string[]): ArrangementRefusal => {
  if (failure._tag === 'SdkChunkFailure') return {
    reason: { _tag: 'Unknown' }, retryableLoad: true, detail: failure.message,
    targets,
  }
  if (failure._tag !== 'ActionRefused') return { reason: { _tag: 'Unknown' }, detail: failure.message, targets }
  const code = decodeUnknownSync(ErrorCode)(failure.response.code)
  return {
    reason: typeof code === 'string' ? { _tag: 'Known', code } : { _tag: 'Unknown', code: code.raw },
    detail: failure.response.message, targets, error: failure.response,
  }
}
/** Diagnostic envelopes stay structured; the visible sentence never quotes server detail. */
const refusalSentences: Readonly<Partial<Record<Extract<typeof ErrorCode.Type, string>, string>>> = {
  'stale-fence': 'The folder layout changed elsewhere; refresh and try again.',
  'forbidden': 'This connection cannot edit the folder layout; request editing access.',
  'arrangement-owner-forbidden': 'This connection cannot edit the folder layout; request editing access.',
  'unsupported-capability': 'Folder editing is unavailable for this connection; reconnect with editing access.',
  'arrangement-exists': 'The folder layout was created elsewhere; refresh and try again.',
  'arrangement-folder-exists': 'This folder already exists; refresh before creating another folder.',
  'arrangement-retired': 'This folder layout was removed; refresh to use the current layout.',
  'arrangement-folder-deleted': 'This folder was deleted; refresh and choose another folder.',
  'arrangement-limit': 'The folder layout has reached its limit; reduce the layout before trying again.',
  'arrangement-body-too-large': 'The folder layout is too large; reduce the layout before trying again.',
  'arrangement-cycle': 'A folder cannot contain itself; choose a different parent folder.',
  'invalid-arrangement-subject': 'The folder layout could not be identified; refresh and try again.',
  'invalid-arrangement-action': 'This change could not be accepted; refresh and try again.',
  'invalid-arrangement-operations': 'This change could not be accepted; refresh and try again.',
  'invalid-arrangement-folder': 'The folder could not be identified; refresh and try again.',
  'invalid-arrangement-name': 'The folder name is not valid; use a shorter, nonblank name.',
  'invalid-arrangement-key': 'The folder order could not be saved; refresh and try again.',
  'invalid-subject-reference': 'The agent could not be placed; refresh and select it again.',
  'not-found': 'The folder layout or item no longer exists; refresh and choose another.',
  'validation-failed': 'This change could not be accepted; refresh and try again.',
  'idempotency-conflict': 'This change conflicts with an earlier change; refresh and try again.',
}
export const refusalText = (refusal: ArrangementRefusal): string => refusal.restageRefusal ??
  (refusal.awaitingVisibility ? undefined : refusal.reason._tag === 'Known' ? refusalSentences[refusal.reason.code] : undefined) ?? 'The change was not saved.'

const optimisticRevision = 'claim/optimistic'
export const optimisticArrangement = (base: Arrangement, operations: readonly SidebarOperation[]): Arrangement => {
  const body = structuredClone(base.body)
  for (const operation of operations) {
    switch (operation.op) {
      case 'folder.create':
        body.folders[operation.id] = { name: { value: operation.name, revision: optimisticRevision }, position: { value: { parent: operation.parent, key: operation.key }, revision: optimisticRevision }, tombstone: null }
        break
      case 'folder.rename': {
        const folder = body.folders[operation.id]
        if (folder !== undefined) folder.name = { value: operation.name, revision: optimisticRevision }
        break
      }
      case 'folder.move': {
        const folder = body.folders[operation.id]
        if (folder !== undefined) folder.position = { value: { parent: operation.parent, key: operation.key }, revision: optimisticRevision }
        break
      }
      case 'folder.delete': {
        const folder = body.folders[operation.id]
        if (folder !== undefined) folder.tombstone = { value: true, revision: optimisticRevision }
        break
      }
      case 'subject.place':
        // Version 2 layouts are folder-only; their members are ordered by membership actions.
        if (body.version === 1) body.placements[operation.subject] = { value: { folder: operation.folder, key: operation.key }, revision: optimisticRevision }
        break
    }
  }
  // Server-resolved locations describe the old registers, not this optimistic layout.
  const { resolved: _resolved, ...header } = base
  return { ...header, body }
}
const emptyArrangement = (owner: string, id: string): Arrangement => ({
  id, owner, kind: 'arrangement', deleted: false, revision: optimisticRevision, updated_at: new Date().toISOString(),
  body: { version: 1, name: { value: 'Sidebar', revision: optimisticRevision }, folders: {}, placements: {} },
})
export interface ArrangementEditorState {
  readonly arrangement?: Arrangement
  readonly phase: 'synced' | 'pending' | 'refused'
  readonly refusal?: ArrangementRefusal
  readonly retryReady: boolean
  readonly pendingTargets?: readonly string[]
  readonly sidebarCandidates?: readonly { readonly id: string; readonly label: string }[]
  readonly sidebarSubject?: string
  readonly restoreUnavailable?: boolean
}
export type EditOutcome = { readonly _tag: 'Success' } | ({ readonly _tag: 'Refused' } & ArrangementRefusal)
/** Recomputed structural intent: already satisfied is success; invalid against fresh state stays refused. */
export type RestageVerdict =
  | { readonly _tag: 'Operations'; readonly operations: readonly SidebarOperation[] }
  | { readonly _tag: 'Satisfied' }
  | { readonly _tag: 'Refused'; readonly sentence: string }
/** Recompute structural intent after a fresh inventory read, before staging a new request. */
export type SidebarRestage = (arrangement: Arrangement | undefined) => RestageVerdict
export interface StructuralIntent {
  readonly restage: SidebarRestage
  /** Continues a combined intent once this edit is applied, including after an explicit retry. */
  readonly onApplied?: () => void
}
export interface ArrangementEditor {
  readonly accept: (inventory: ArrangementInventory) => void
  readonly edit: (operations: readonly SidebarOperation[], intent?: StructuralIntent) => Promise<EditOutcome>
  readonly retryEdit: () => Promise<EditOutcome>
  readonly close: () => void
}

type MutationJob = EditJob & {
  abandon: Deferred.Deferred<void>
  sent?: boolean
  fiber?: Fiber.Fiber<void>
  entered?: boolean
  outcome?: EditOutcome
  revision?: string
  receiptFrame?: Arrangement
  satisfied?: boolean
  base?: Arrangement
  snapshot?: string
}

/** One owner, shared optimistic state, serialized transport, one reserved creation subject. */
export const createArrangementEditor = ({ owner, actions, read, onState }: {
  readonly owner: string
  readonly actions: ArrangementActionPort
  readonly read: (signal: AbortSignal) => Promise<ArrangementInventory>
  readonly onState: (state: ArrangementEditorState) => void
}): ArrangementEditor => {
  let inventory: ArrangementInventory | undefined
  const reservation = reservedSidebarSubject(owner)
  let reservedObserved = false
  let reservedRetired = false
  let creation: { readonly subject: string; readonly phase: 'acknowledged' | 'observed' } | undefined
  let closed = false
  let running = false
  let refusal: ArrangementRefusal | undefined
  let failedKey: string | undefined
  let retryReady = false
  const transport = Semaphore.makeUnsafe(1)
  const mutations = Effect.runSync(makeOptimisticMutations<Arrangement | undefined, MutationJob>({
    initial: undefined, nextKey: () => crypto.randomUUID(),
    send: ({ action: job }) => Effect.raceFirst(transport.withPermit(Effect.promise(async () => {
      job.entered = true
      job.outcome = await perform(job)
      return job.outcome
    }).pipe(Effect.flatMap((outcome) => outcome._tag === 'Success' ? Effect.void : Effect.fail(new MutationRefused({
      message: outcome.detail,
      nonApplication: !job.sent ? { _tag: 'NeverSent' } : job.requestRefused ? { _tag: 'Refused', code: outcome.error?.code ?? 'action-rejected' } : undefined,
    }))))), Deferred.await(job.abandon).pipe(Effect.andThen(Effect.fail(new MutationRefused({
      message: 'Not submitted because an earlier arrangement edit failed.', nonApplication: { _tag: 'NeverSent' },
    }))))),
    project: (frame, entry) => entry.phase === 'failed' ? frame : optimisticArrangement(frame ?? entry.action.base ?? emptyArrangement(owner, reservation), entry.action.operations),
    reconcile: (frame, { action: job }) => job.satisfied || (frame !== undefined && frame.id === job.request?.parameters.subject && (frame === job.receiptFrame || (job.revision !== undefined && frame.revision === job.revision))) ? 'applied' : 'pending',
  }))
  const pending = () => Effect.runSync(SubscriptionRef.get(mutations.state)).pending
  const submit = (job: MutationJob, retry = false) => {
    job.fiber = Effect.runFork((retry ? mutations.retry(job.key) : mutations.submit({ key: job.key, action: job, refusal: 'retain' })).pipe(
      Effect.andThen(Effect.promise(async () => {
        publish() // Roll back failed projections before a stale-fence refresh.
        const reason = job.outcome?._tag === 'Refused' ? job.outcome : undefined
        if (reason?.error?.code !== 'stale-fence' && reason?.error?.code !== 'arrangement-exists') return
        running = true
        try {
          const refreshed = await refresh()
          if (refreshed._tag === 'Success' && reason.error.code === 'arrangement-exists' &&
            sidebarWinner(inventory?.items ?? [], owner) === undefined && refusal !== undefined)
            refusal = { ...refusal, awaitingVisibility: true }
          retryReady = refreshed._tag === 'Success' && !reservedRetired
        } catch { /* A failed refresh keeps retry disabled until an explicit refresh succeeds. */ }
        finally { running = false }
      })),
      Effect.ensuring(Effect.sync(() => {
        publish()
        if (job.outcome !== undefined) job.done.resolve(job.outcome)
      })),
    ))
  }
  const abort = new AbortController()
  const targets = (job: EditJob) => [...new Set(job.operations.flatMap((operation) => operation.op === 'subject.place'
    ? [operation.subject, ...(operation.folder === null ? [] : [operation.folder])]
    : [operation.id]))]
  const unknown = (detail: string, job: EditJob): ArrangementRefusal => ({ reason: { _tag: 'Unknown' }, detail, targets: targets(job) })
  const publish = () => {
    if (closed) return
    const arrangement = Effect.runSync(mutations.view)
    const active = [...pending().values()].filter((entry) => entry.phase !== 'failed')
    const candidates = inventory === undefined || arrangement?.id === reservation ? [] : sidebarCandidates(inventory.items, owner)
    const selection = {
      ...(candidates.length > 1 ? { sidebarCandidates: candidates.map((item) => ({ id: item.id, label: `${item.body.name.value} (${item.id})` })) } : {}),
      ...(arrangement === undefined && !reservedRetired ? {} : { sidebarSubject: arrangement?.id ?? reservation }),
      ...(reservedRetired && arrangement === undefined ? { restoreUnavailable: true } : {}),
    }
    onState({ ...selection, ...(arrangement === undefined ? {} : { arrangement }), phase: active.length > 0 ? 'pending' : refusal === undefined ? 'synced' : 'refused', pendingTargets: [...new Set(active.flatMap((entry) => targets(entry.action)))], ...(refusal === undefined ? {} : { refusal }), retryReady: !selection.restoreUnavailable && retryReady })
  }
  const accept = (next: ArrangementInventory) => {
    if (closed || next.owner !== owner) return
    const previous = inventory?.snapshot
    const incoming = next.snapshot
    if (previous !== undefined && incoming !== undefined &&
      previous.host_id === incoming.host_id && previous.projection_version === incoming.projection_version &&
      incoming.store_index < previous.store_index) return
    const reserved = next.items.find((item) => item.id === reservation)
    if (reserved !== undefined && !reserved.deleted) { reservedObserved = true; reservedRetired = false }
    else if (reservedObserved || reserved?.deleted) reservedRetired = true
    for (const { action: job } of pending().values()) {
      if (job.request?.parameters.operations.some((operation) => operation.op === 'create') &&
        next.items.some((item) => item.id === job.request?.parameters.subject)) job.creationObserved = true
    }
    if (creation !== undefined) {
      const subject = creation.subject
      const observed = next.items.find((item) => item.id === subject)
      if (observed !== undefined) creation = { subject, phase: 'observed' }
      if (creation.phase === 'observed' && (observed === undefined || observed.deleted)) {
        // A renamed reserved subject keeps its identity; retirement is terminal.
        creation = undefined
      }
    }
    inventory = next
    const before = pending()
    Effect.runSync(mutations.accept(sidebarWinner(next.items, owner)))
    if (failedKey !== undefined && !pending().has(failedKey)) { failedKey = undefined; refusal = undefined; retryReady = false }
    for (const [key, { action: job }] of before) if (!pending().has(key)) job.intent?.onApplied?.()
    publish()
  }
  const fail = async (job: MutationJob, reason: ArrangementRefusal): Promise<EditOutcome> => {
    failedKey = reason.restageRefusal === undefined ? job.key : undefined
    const winner = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)
    const attempts = job.operations.map((operation): ArrangementAttempt => operation.op === 'subject.place'
      ? { id: operation.subject, label: operation.subject, kind: 'agent' }
      : {
        id: operation.id, kind: 'folder',
        label: operation.op === 'folder.create' || operation.op === 'folder.rename'
          ? operation.name : winner?.body.folders[operation.id]?.name.value ?? operation.id,
      })
    refusal = { ...reason, attemptedTargets: [...new Map(attempts.map((attempt) => [attempt.id, attempt])).values()] }
    retryReady = reason.restageRefusal === undefined && reason.error?.code !== 'stale-fence' && reason.error?.code !== 'arrangement-exists'
    for (const { action: later } of pending().values()) if (later !== job && !later.entered && later.fiber !== undefined) {
      later.outcome = { _tag: 'Refused', ...unknown('Not submitted because an earlier arrangement edit failed.', later) }
      Effect.runSync(Deferred.succeed(later.abandon, undefined))
      await Effect.runPromise(Fiber.join(later.fiber))
      Effect.runSync(mutations.discardFailed(later.key))
    }
    return { _tag: 'Refused', ...reason }
  }
  const refresh = async () => {
    const snapshot = await Effect.runPromise(Effect.result(actions.snapshot), { signal: abort.signal })
    if (snapshot._tag === 'Failure') return snapshot
    accept(await read(abort.signal))
    return snapshot
  }
  const perform = async (job: MutationJob): Promise<EditOutcome> => {
    running = true
    try {
      const fresh = job.snapshot === undefined ? await refresh() : { _tag: 'Success' as const, success: job.snapshot }
      job.snapshot = undefined
      if (closed) return { _tag: 'Refused', ...unknown('The arrangement editor is closed.', job) }
      if (fresh._tag === 'Failure') return await fail(job, arrangementRefusal(fresh.failure, targets(job)))
      const winner = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)
      job.base = winner ?? job.base
      if (winner === undefined && reservedRetired)
        return await fail(job, unknown('Restoring a removed Sidebar is not available yet.', job))
      const staged = stageArrangementEdit(job, owner, winner, reservation, fresh.success, creation?.phase === 'acknowledged')
      if (staged._tag === 'Refused')
        return await fail(job, { ...unknown(staged.detail, job), ...(staged.restageRefusal === undefined ? {} : { restageRefusal: staged.restageRefusal }) })
      if (staged._tag === 'Satisfied') {
        job.satisfied = true // The fresh authoritative inventory already satisfies this structural intent.
        accept(inventory!)
        return { _tag: 'Success' }
      }
      const request = staged.request
      job.requestRefused = false
      job.sent = true
      const result = await Effect.runPromise(Effect.result(actions.submitAction(request)), { signal: abort.signal })
      if (closed) return { _tag: 'Refused', ...unknown('The arrangement editor is closed.', job) }
      if (result._tag === 'Failure') {
        const reason = arrangementRefusal(result.failure, targets(job))
        job.requestRefused = result.failure._tag === 'ActionRefused' &&
          reason.error?.applied !== 'unknown' && reason.error?.details.applied !== 'unknown'
        if (reason.error?.code === 'arrangement-retired' && request.parameters.subject === reservation) reservedRetired = true
        return await fail(job, reason)
      }
      if (result.success.status === 'rejected') {
        job.requestRefused = true
        return await fail(job, unknown('The action result was rejected without a refusal reason.', job))
      }
      if (request.parameters.operations.some((operation) => operation.op === 'create'))
        creation = { subject: request.parameters.subject, phase: 'acknowledged' }
      if (typeof result.success.arrangement_revision === 'string') job.revision = result.success.arrangement_revision
      if (inventory !== undefined) accept(inventory)
      // A fresh complete read after this exact receipt includes accepted or merged edits.
      const observed = await read(abort.signal)
      job.receiptFrame = sidebarWinner(observed.items, owner)
      accept(observed)
      return { _tag: 'Success' }
    } catch (cause) {
      return await fail(job, unknown(cause instanceof Error ? cause.message : String(cause), job))
    } finally { running = false }
  }
  const edit = (operations: readonly SidebarOperation[], intent?: StructuralIntent): Promise<EditOutcome> => {
    const sidebar = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)?.id
    const job: MutationJob = { operations: structuredClone(operations), ...(intent === undefined ? {} : { intent }), ...(sidebar === undefined ? {} : { sidebar }), id: `action/${crypto.randomUUID()}`, key: crypto.randomUUID(), done: Promise.withResolvers<EditOutcome>(), abandon: Deferred.makeUnsafe<void>() }
    job.base = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)
    if (reservedRetired && sidebarWinner(inventory?.items ?? [], owner) === undefined)
      return Promise.resolve({ _tag: 'Refused', ...unknown('Restoring a removed Sidebar is not available yet.', job) })
    if (!running) failedKey = undefined // A new explicit edit is not a replay of the failed request.
    if (closed || inventory === undefined || failedKey !== undefined) {
      const reason = unknown(closed ? 'The arrangement editor is closed.' : inventory === undefined ? 'A complete owner inventory is required before editing.' : 'Retry or refresh the refused edit before submitting another edit.', job)
      return Promise.resolve({ _tag: 'Refused', ...reason })
    }
    refusal = undefined
    submit(job)
    publish() // Synchronous paint, before snapshot or HTTP work.
    return job.done.promise
  }
  const retryEdit = async (): Promise<EditOutcome> => {
    let job = failedKey === undefined ? undefined : pending().get(failedKey)?.action
    if (job === undefined) return { _tag: 'Success' }
    if (reservedRetired && sidebarWinner(inventory?.items ?? [], owner) === undefined)
      return { _tag: 'Refused', ...(refusal ?? unknown('Restoring a removed Sidebar is not available yet.', job)) }
    if (running) return { _tag: 'Refused', ...(refusal ?? unknown('The arrangement snapshot is still refreshing.', job)) }
    running = true
    try {
      const fresh = await refresh()
      if (fresh._tag === 'Failure') { running = false; return { _tag: 'Refused', ...arrangementRefusal(fresh.failure, targets(job)) } }
      if (!pending().has(job.key)) { running = false; return { _tag: 'Success' } }
      retryReady = true
      const prepared = job.requestRefused ? prepareArrangementRetry(job, inventory === undefined ? undefined : sidebarWinner(inventory.items, owner), reservation) : job
      if (prepared.key !== job.key) {
        Effect.runSync(mutations.discardFailed(job.key))
        job = { ...prepared, abandon: Deferred.makeUnsafe<void>(), base: job.base }
      }
      job.done = Promise.withResolvers<EditOutcome>()
      job.entered = false
      job.outcome = undefined
      job.snapshot = fresh.success
      failedKey = undefined
      refusal = undefined
      running = false
      submit(job, pending().has(job.key))
      publish()
    } catch (cause) { running = false; return { _tag: 'Refused', ...unknown(cause instanceof Error ? cause.message : String(cause), job) } }
    return job.done.promise
  }
  return { accept, edit, retryEdit, close: () => {
    closed = true
    abort.abort()
    for (const { action: job } of pending().values()) {
      job.done.resolve({ _tag: 'Refused', ...unknown('The arrangement editor is closed.', job) })
      if (job.fiber !== undefined) Effect.runFork(Fiber.interrupt(job.fiber))
    }
  } }
}
