import { SidebarJournal, type MembershipDelivery, type SidebarJournalState, type SidebarJournalRequest, type SidebarJournalEffect, type SidebarJournalResult, type SidebarPersistenceFailure } from './sidebarJournal.ts'
import type { ActionOf, Arrangement, ArrangementOperation, ErrorEnvelope } from '@fractal/client'
import { ErrorCode, decodeUnknownSync } from '@fractal/client/schema/read'
import type { ActionResult } from '@fractal/client/schema'
import type { ActionFailure } from '@fractal/sdk/mutations'
import type { SdkChunkFailure } from '../data/lazySdk.ts'
import { loadArrangementSchema } from '../data/lazySdk.ts'
import { Effect, Schema } from 'effect'
import { reservedSidebarSubject, sidebarCandidates, sidebarWinner, type ArrangementInventory } from '../data/arrangements.ts'

/** Loading errors are refused before submission, and the existing edit retry reacquires the chunk. */
export interface ArrangementActionPort {
  readonly snapshot: Effect.Effect<string, ActionFailure | SdkChunkFailure>
  readonly submitAction: (request: ActionOf<'arrangement.edit'>) =>
    Effect.Effect<ActionResult, ActionFailure | SdkChunkFailure>
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
  readonly reason: { readonly _tag: 'Known'; readonly code: Extract<ErrorCode, string> } | { readonly _tag: 'Unknown'; readonly code?: string }
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
const refusalSentences: Readonly<Partial<Record<Extract<ErrorCode, string>, string>>> = {
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
interface EditJob {
  operations: readonly SidebarOperation[]
  readonly intent?: StructuralIntent
  /** Sidebar selected when the gesture was accepted; undefined only for a bootstrap with no Sidebar. */
  readonly sidebar?: string
  readonly id: string
  readonly key: string
  readonly done: PromiseWithResolvers<EditOutcome>
  request?: ActionOf<'arrangement.edit'>
  /** The last submitted request was authoritatively refused, not transport-uncertain. */
  requestRefused?: boolean
  creationObserved?: boolean
}
export interface ArrangementEditor {
  readonly accept: (inventory: ArrangementInventory) => void
  readonly edit: (operations: readonly SidebarOperation[], intent?: StructuralIntent) => Promise<EditOutcome>
  readonly retryEdit: () => Promise<EditOutcome>
  readonly close: () => void
}

/** Legacy v1 serialized queue. V2 persistence effects live in the unwired sidebarJournal.ts host. */
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
  let failed: EditJob | undefined
  let retryReady = false
  const prepareRetry = (job: EditJob): EditJob => {
    const winner = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)
    if (job.intent !== undefined) {
      const { request: _request, ...fields } = job
      return { ...fields, id: `action/${crypto.randomUUID()}`, key: crypto.randomUUID() }
    }
    const request = job.request
    if (request === undefined || (request.parameters.subject === (winner?.id ?? reservation) &&
      request.parameters.operations.some((operation) => operation.op === 'create') === (winner === undefined))) return job
    // Changed parameters after a known non-applied refusal need a new action identity/key.
    const { request: _request, ...fields } = job
    return { ...fields, id: `action/${crypto.randomUUID()}`, key: crypto.randomUUID() }
  }
  const queue: EditJob[] = []
  const abort = new AbortController()
  const targets = (job: EditJob) => [...new Set(job.operations.flatMap((operation) => operation.op === 'subject.place'
    ? [operation.subject, ...(operation.folder === null ? [] : [operation.folder])]
    : [operation.id]))]
  const unknown = (detail: string, job: EditJob): ArrangementRefusal => ({ reason: { _tag: 'Unknown' }, detail, targets: targets(job) })
  const publish = () => {
    if (closed) return
    let arrangement = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)
    for (const job of queue) {
      arrangement ??= emptyArrangement(owner, reservation)
      arrangement = optimisticArrangement(arrangement, job.operations)
    }
    const candidates = inventory === undefined || arrangement?.id === reservation ? [] : sidebarCandidates(inventory.items, owner)
    const selection = {
      ...(candidates.length > 1 ? { sidebarCandidates: candidates.map((item) => ({ id: item.id, label: `${item.body.name.value} (${item.id})` })) } : {}),
      ...(arrangement === undefined && !reservedRetired ? {} : { sidebarSubject: arrangement?.id ?? reservation }),
      ...(reservedRetired && arrangement === undefined ? { restoreUnavailable: true } : {}),
    }
    onState({ ...selection, ...(arrangement === undefined ? {} : { arrangement }), phase: queue.length > 0 ? 'pending' : refusal === undefined ? 'synced' : 'refused', pendingTargets: [...new Set(queue.flatMap(targets))], ...(refusal === undefined ? {} : { refusal }), retryReady: !selection.restoreUnavailable && retryReady })
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
    for (const job of failed === undefined ? queue : [failed, ...queue]) {
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
    publish()
  }
  const fail = (job: EditJob, reason: ArrangementRefusal, waitForRefresh = false) => {
    failed = reason.restageRefusal === undefined ? job : undefined
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
    const abandoned = queue.splice(0)
    publish() // Remove every optimistic edit before returning the refusal.
    if (!waitForRefresh) job.done.resolve({ _tag: 'Refused', ...reason })
    for (const later of abandoned) if (later !== job)
      later.done.resolve({ _tag: 'Refused', ...unknown('Not submitted because an earlier arrangement edit failed.', later) })
  }
  const refresh = async () => {
    const snapshot = await Effect.runPromise(Effect.result(actions.snapshot), { signal: abort.signal })
    if (snapshot._tag === 'Failure') return snapshot
    accept(await read(abort.signal))
    return snapshot
  }
  const drain = async () => {
    if (running || closed || failed !== undefined) return
    running = true
    try {
      while (queue.length > 0 && !closed) {
        let job = queue[0]!
        const schema = await Effect.runPromise(Effect.result(loadArrangementSchema), { signal: abort.signal })
        if (closed) return
        if (schema._tag === 'Failure') { fail(job, arrangementRefusal(schema.failure, targets(job))); return }
        const fresh = await refresh()
        if (closed) return
        if (fresh._tag === 'Failure') { fail(job, arrangementRefusal(fresh.failure, targets(job))); return }
        const winner = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)
        if (winner === undefined && reservedRetired) {
          fail(job, unknown('Restoring a removed Sidebar is not available yet.', job))
          return
        }
        // Retarget only a definitive non-applied refusal, after the fresh inventory is accepted.
        if (job.requestRefused) { job = prepareRetry(job); queue[0] = job }
        // A structural intent uses fresh siblings, but only within the Sidebar the user edited.
        // Bootstrap is the narrow exception: no Sidebar existed, so only the reserved creation target qualifies.
        if (job.request === undefined && job.intent !== undefined) {
          const bound = job.sidebar === undefined ? winner === undefined || winner.id === reservation : winner?.id === job.sidebar
          const verdict: RestageVerdict = bound ? job.intent.restage(winner) : { _tag: 'Refused', sentence: 'The folder layout changed elsewhere; make the change again.' }
          if (verdict._tag === 'Refused') {
            fail(job, { ...unknown(verdict.sentence, job), restageRefusal: verdict.sentence })
            return
          }
          if (verdict._tag === 'Satisfied') {
            queue.shift()
            publish()
            job.done.resolve({ _tag: 'Success' })
            job.intent.onApplied?.()
            continue
          }
          job.operations = structuredClone(verdict.operations)
        }
        if (job.request === undefined) {
          if (winner === undefined && creation?.phase === 'acknowledged') {
            fail(job, unknown('The created Sidebar is not visible in the complete inventory yet. Refresh before retrying.', job))
            return
          }
          const parameters = { owner, subject: winner?.id ?? reservation, operations: winner === undefined ? [{ op: 'create', name: 'Sidebar' } as const, ...job.operations] : [...job.operations] }
          if (!Schema.is(Schema.toEncoded(schema.success.ArrangementEditParameters))(parameters as unknown)) {
            fail(job, unknown('The edit does not match the generated arrangement operation contract.', job))
            return
          }
          // TODO(live-migration: arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both): keep old names until all members accept aliases; sunset 60 days after first shipping release.
          job.request = { api_version: 'st3.client.v0', type: 'arrangement.edit', id: job.id, idempotency_key: job.key, parameters, fence: { snapshot_id: fresh.success, subject_revisions: winner === undefined ? {} : { [winner.id]: winner.revision } } }
        } else {
          // Unknown delivery retries must replay exactly the original identity and parameters.
          const unobservedCreation = job.request.parameters.operations.some((operation) => operation.op === 'create') && !job.creationObserved
          if (winner?.id !== job.request.parameters.subject && !(winner === undefined && unobservedCreation)) {
            fail(job, unknown('The folder layout changed elsewhere; refresh and try again.', job))
            return
          }
          job.request = { ...job.request, fence: { snapshot_id: fresh.success, subject_revisions: winner === undefined ? {} : { [winner.id]: winner.revision } } }
        }
        job.requestRefused = false
        const result = await Effect.runPromise(Effect.result(actions.submitAction(job.request)), { signal: abort.signal })
        if (closed) return
        if (result._tag === 'Failure') {
          job.requestRefused = result.failure._tag === 'ActionRefused'
          const reason = arrangementRefusal(result.failure, targets(job))
          if (reason.error?.code === 'arrangement-retired' && job.request.parameters.subject === reservation) reservedRetired = true
          const refreshRequired = reason.error?.code === 'stale-fence' || reason.error?.code === 'arrangement-exists'
          fail(job, reason, refreshRequired)
          if (refreshRequired) {
            // Refetch, but NEVER automatically replay a refused edit.
            try {
              const refreshed = await refresh()
              if (refreshed._tag === 'Success' && reason.error?.code === 'arrangement-exists' &&
                sidebarWinner(inventory?.items ?? [], owner) === undefined && refusal !== undefined)
                refusal = { ...refusal, awaitingVisibility: true }
              retryReady = refreshed._tag === 'Success' && !reservedRetired
              if (retryReady) failed = prepareRetry(job)
              publish()
            } finally { job.done.resolve({ _tag: 'Refused', ...reason }) }
          }
          return
        }
        if (result.success.status === 'rejected') {
          job.requestRefused = true
          fail(job, unknown('The action result was rejected without a refusal reason.', job))
          return
        }
        if (job.request.parameters.operations.some((operation) => operation.op === 'create'))
          creation = { subject: job.request.parameters.subject, phase: 'acknowledged' }
        // Replace optimism only with a complete inventory read, not an assumed register winner.
        accept(await read(abort.signal))
        queue.shift()
        publish()
        job.done.resolve({ _tag: 'Success' })
        job.intent?.onApplied?.()
      }
    } catch (cause) {
      if (!closed && queue[0] !== undefined) fail(queue[0], unknown(cause instanceof Error ? cause.message : String(cause), queue[0]))
      // A failed stale-fence refresh keeps retry disabled until an explicit refresh succeeds.
    } finally { running = false }
  }
  const edit = (operations: readonly SidebarOperation[], intent?: StructuralIntent): Promise<EditOutcome> => {
    const sidebar = inventory === undefined ? undefined : sidebarWinner(inventory.items, owner)?.id
    const job: EditJob = { operations: structuredClone(operations), ...(intent === undefined ? {} : { intent }), ...(sidebar === undefined ? {} : { sidebar }), id: `action/${crypto.randomUUID()}`, key: crypto.randomUUID(), done: Promise.withResolvers<EditOutcome>() }
    if (reservedRetired && sidebarWinner(inventory?.items ?? [], owner) === undefined)
      return Promise.resolve({ _tag: 'Refused', ...unknown('Restoring a removed Sidebar is not available yet.', job) })
    if (!running) failed = undefined // A new explicit edit is not a replay of the failed request.
    if (closed || inventory === undefined || failed !== undefined) {
      const reason = unknown(closed ? 'The arrangement editor is closed.' : inventory === undefined ? 'A complete owner inventory is required before editing.' : 'Retry or refresh the refused edit before submitting another edit.', job)
      return Promise.resolve({ _tag: 'Refused', ...reason })
    }
    queue.push(job)
    refusal = undefined
    publish() // Synchronous paint, before snapshot or HTTP work.
    void drain()
    return job.done.promise
  }
  const retryEdit = async (): Promise<EditOutcome> => {
    let job = failed
    if (job === undefined) return { _tag: 'Success' }
    if (reservedRetired && sidebarWinner(inventory?.items ?? [], owner) === undefined)
      return { _tag: 'Refused', ...(refusal ?? unknown('Restoring a removed Sidebar is not available yet.', job)) }
    if (running) return { _tag: 'Refused', ...(refusal ?? unknown('The arrangement snapshot is still refreshing.', job)) }
    if (!retryReady) {
      try {
        const fresh = await refresh()
        if (fresh._tag === 'Failure') return { _tag: 'Refused', ...arrangementRefusal(fresh.failure, targets(job)) }
        retryReady = true
        if (job.requestRefused) job = prepareRetry(job)
      } catch (cause) { return { _tag: 'Refused', ...unknown(cause instanceof Error ? cause.message : String(cause), job) } }
    }
    failed = undefined
    refusal = undefined
    const retried = { ...job, done: Promise.withResolvers<EditOutcome>() }
    queue.push(retried)
    publish()
    void drain()
    return retried.done.promise
  }
  return { accept, edit, retryEdit, close: () => {
    closed = true
    abort.abort()
    for (const job of queue.splice(0)) job.done.resolve({ _tag: 'Refused', ...unknown('The arrangement editor is closed.', job) })
  } }
}

type MembershipIdentity = SidebarJournalRequest['identity']
type MembershipBase = SidebarJournalRequest['fence']
type MembershipRefusal = NonNullable<SidebarJournalState['refusal']>
type MembershipOperations = SidebarJournalRequest['operations']
export type MembershipEditEvent =
  | { readonly _tag: 'Plan'; readonly base: MembershipBase; readonly folders: Extract<MembershipOperations, { Folders: unknown }>['Folders']; readonly memberships: Extract<MembershipOperations, { Memberships: unknown }>['Memberships']; readonly folder: MembershipIdentity; readonly membership: MembershipIdentity }
  | { readonly _tag: 'Fresh'; readonly base: MembershipBase; readonly memberships: Extract<MembershipOperations, { Memberships: unknown }>['Memberships']; readonly refusal?: MembershipRefusal }
  | { readonly _tag: 'Persisted' | 'CompletionPersisted' | 'Ack' | 'Unknown' | 'Observed'; readonly identity: MembershipIdentity }
  | { readonly _tag: 'Refused'; readonly identity: MembershipIdentity; readonly refusal: MembershipRefusal }
  | SidebarPersistenceFailure
  | { readonly _tag: 'Replay' | 'Retry' }
const sameMembershipIdentity = (a: MembershipIdentity, b: MembershipIdentity): boolean => a.actor === b.actor && a.action_id === b.action_id && a.idempotency_key === b.idempotency_key
/** Unwired Rust edits.rs port. Host serializes events and executes every journal effect. */
export class MembershipEditor {
  state: SidebarJournalState
  // Restored requests may already have committed, even before their first recorded send.
  #nextDelivery: MembershipDelivery = 'Replay'
  constructor(readonly journal: SidebarJournal, intent: SidebarJournalState['intent'], restored?: SidebarJournalState) {
    this.state = restored ?? { scope: journal.scope, intent, status: 'Intent', refusal: null, request: null, dependent: null, folder_completed: false, used: [] }
  }
  #valid(base: MembershipBase): boolean {
    const s = this.state.scope
    return base.ready && base.scope.owner === s.owner && base.scope.container === s.container && base.scope.endpoint === s.endpoint && base.scope.host === s.host && base.scope.projection === s.projection && base.snapshot.host_id === s.host && base.snapshot.projection_version === s.projection
  }
  #unused(id: MembershipIdentity): boolean {
    return id.actor !== '' && id.action_id !== '' && id.idempotency_key !== '' && !this.state.used.some(used => used.action_id === id.action_id || used.idempotency_key === id.idempotency_key)
  }
  #refuse(refusal: MembershipRefusal): SidebarJournalEffect[] {
    this.state = { ...this.state, refusal, status: this.state.folder_completed ? 'PartiallyApplied' : 'Refused' }
    return ['PersistPending', 'ReadBase']
  }
  #stage(identity: MembershipIdentity, fence: MembershipBase, operations: MembershipOperations): SidebarJournalEffect[] {
    if (!this.#unused(identity)) return this.#refuse('InvalidPlan')
    this.state = { ...this.state, used: [...this.state.used, identity], request: structuredClone({ identity, fence, operations }), status: 'AwaitingPersistence' }
    this.#nextDelivery = 'First'
    return ['PersistPending']
  }
  /** Persistence acknowledgements come only from this journal executor, never transport. */
  execute(effect: SidebarJournalEffect): SidebarJournalResult & { readonly effects?: readonly SidebarJournalEffect[] } {
    const result = this.journal.run(effect, this.state)
    if (result._tag === 'Applied' && result.event) return { ...result, effects: this.reduce(result.event) }
    if (result._tag === 'StorageFailed' && result.event) return { ...result, effects: this.reduce(result.event) }
    return result
  }
  reduce(event: MembershipEditEvent): SidebarJournalEffect[] {
    const state = this.state
    const request = state.request
    if ('identity' in event && (!request || !sameMembershipIdentity(request.identity, event.identity))) return []
    if (event._tag === 'Plan' && state.status === 'Intent') {
      if (!this.#valid(event.base) || request && (event.base.snapshot.id === request.fence.snapshot.id || event.base.snapshot.store_index < request.fence.snapshot.store_index)) return ['ReadBase']
      if (!event.folders.length && !event.memberships.length) return this.#refuse('InvalidPlan')
      if (event.folders.length) {
        if (event.memberships.length) {
          if (!this.#unused(event.membership) || event.folder.action_id === event.membership.action_id || event.folder.idempotency_key === event.membership.idempotency_key) return this.#refuse('InvalidPlan')
          this.state = { ...state, dependent: event.membership }
        }
        return this.#stage(event.folder, event.base, { Folders: event.folders })
      }
      return this.#stage(event.membership, event.base, { Memberships: event.memberships })
    }
    if (event._tag === 'Fresh' && state.status === 'AwaitingFreshBase' && request && state.dependent) {
      const previous = request.fence
      if (!this.#valid(event.base) || event.base.snapshot.id === previous.snapshot.id || event.base.revision === previous.revision || event.base.snapshot.store_index < previous.snapshot.store_index) return ['ReadBase']
      if (event.refusal || !event.memberships.length) return this.#refuse(event.refusal ?? 'InvalidPlan')
      this.state = { ...state, dependent: null }
      return this.#stage(state.dependent, event.base, { Memberships: event.memberships })
    }
    if (event._tag === 'Retry' && ['Refused', 'PartiallyApplied'].includes(state.status)) {
      this.state = { ...state, status: 'Intent', refusal: null, dependent: null }; return ['ReadBase', 'Replan']
    }
    if (event._tag === 'StorageFailed' && (state.status === 'AwaitingPersistence' && event.phase === 'Pending' || state.status === 'AwaitingCompletionPersistence' && event.phase === 'Completion')) return this.#refuse('Storage')
    if (event._tag === 'Replay' && state.status === 'UnknownDelivery') {
      this.state = { ...state, status: 'Sending' }; return [{ _tag: 'Send', delivery: 'Replay' }]
    }
    if (event._tag === 'Persisted' && state.status === 'AwaitingPersistence') {
      const delivery = this.#nextDelivery
      this.#nextDelivery = 'Replay'
      this.state = { ...state, status: 'Sending' }; return [{ _tag: 'Send', delivery }]
    }
    if (event._tag === 'CompletionPersisted' && state.status === 'AwaitingCompletionPersistence') {
      this.state = { ...state, status: state.dependent ? 'AwaitingFreshBase' : 'Observed' }
      return state.dependent ? ['ReadBase'] : ['PersistRemoval']
    }
    if (event._tag === 'Ack' && ['Sending', 'UnknownDelivery'].includes(state.status)) {
      this.state = { ...state, status: 'AwaitingObservation' }; return ['PersistPending', 'ReadBase']
    }
    if (event._tag === 'Unknown' && state.status === 'Sending') {
      this.state = { ...state, status: 'UnknownDelivery' }; return ['PersistPending']
    }
    if (['Sending', 'UnknownDelivery', 'AwaitingObservation'].includes(state.status)) {
      if (event._tag === 'Observed' && request) {
        this.state = { ...state, status: 'AwaitingCompletionPersistence', folder_completed: state.folder_completed || 'Folders' in request.operations }; return ['PersistCompletion']
      }
      if (event._tag === 'Refused') return this.#refuse(event.refusal)
    }
    return []
  }
}
