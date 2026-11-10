import { Schema } from 'effect'
import type { MembershipScope } from './memberships.ts'

const text = Schema.NonEmptyString
const nullableText = Schema.NullOr(text)
const Scope = Schema.Struct({ owner: text, container: text, endpoint: text, host: text, projection: text })
const Identity = Schema.Struct({ actor: text, action_id: text, idempotency_key: text })
const Snapshot = Schema.Struct({ id: text, host_id: text, store_index: Schema.Int.check(Schema.isBetween({ minimum: 0, maximum: Number.MAX_SAFE_INTEGER })), projection_version: text, created_at: text, expires_at: text })
const Base = Schema.Struct({ scope: Scope, snapshot: Snapshot, revision: text, ready: Schema.Boolean })
const Folder = Schema.Union([
  Schema.Struct({ op: Schema.Literal('folder.create'), id: text, name: text, parent: nullableText, key: text }),
  Schema.Struct({ op: Schema.Literal('folder.rename'), id: text, name: text }),
  Schema.Struct({ op: Schema.Literal('folder.move'), id: text, parent: nullableText, key: text }),
  Schema.Struct({ op: Schema.Literal('folder.delete'), id: text }),
])
const Request = Schema.Struct({ identity: Identity, fence: Base, operations: Schema.Union([
  Schema.Struct({ Folders: Schema.Array(Folder) }),
  Schema.Struct({ Memberships: Schema.Array(Schema.Struct({ op: Schema.Literal('place'), member: text, bucket: nullableText, key: text })) }),
]) })
const Node = Schema.Union([Schema.Struct({ kind: Schema.Literal('folder'), id: text }), Schema.Struct({ kind: Schema.Literal('subject'), subject: text })])
const State = Schema.Struct({
  scope: Scope,
  intent: Schema.Struct({ node: Node, parent: nullableText, before: Schema.NullOr(Node) }),
  status: Schema.Literals(['Intent', 'AwaitingPersistence', 'Sending', 'UnknownDelivery', 'AwaitingObservation', 'AwaitingCompletionPersistence', 'AwaitingFreshBase', 'Observed', 'Refused', 'PartiallyApplied']),
  refusal: Schema.NullOr(Schema.Literals(['InvalidPlan', 'StaleFence', 'Unauthorized', 'DestinationMissing', 'Conflict', 'Storage'])),
  request: Schema.NullOr(Request), dependent: Schema.NullOr(Identity), folder_completed: Schema.Boolean, used: Schema.Array(Identity),
})
const Record = Schema.Struct({ version: Schema.Literal(1), state: State })
const Envelope = Schema.fromJsonString(Schema.Struct({ version: Schema.Int.check(Schema.isBetween({ minimum: 0, maximum: 0xffff_ffff })), state: Schema.Unknown }))
const json = Schema.fromJsonString(Record)
const decode = Schema.decodeUnknownSync(json, { onExcessProperty: 'error' })
const encode = Schema.encodeSync(json, { onExcessProperty: 'error' })
export type SidebarJournalState = typeof State.Type
export type SidebarJournalRequest = typeof Request.Type
/** Per-send provenance is reducer-owned and never included in journal v1. */
export type MembershipDelivery = 'First' | 'Replay'
export type SidebarJournalEffect = 'PersistPending' | 'PersistCompletion' | 'PersistRemoval' | 'ReadBase' | 'Replan' | { readonly _tag: 'Send'; readonly delivery: MembershipDelivery }
export type SidebarPersistenceFailure = { readonly _tag: 'StorageFailed'; readonly identity: SidebarJournalRequest['identity']; readonly phase: 'Pending' | 'Completion' | 'Removal' }
export type SidebarJournalError =
  | { readonly _tag: 'Version'; readonly version: number }
  | { readonly _tag: 'InvalidRecord'; readonly cause: unknown }
  | { readonly _tag: 'ScopeMismatch' }
  | { readonly _tag: 'StorageFailed'; readonly operation: 'read' | 'write' | 'remove'; readonly cause: unknown; readonly event?: SidebarPersistenceFailure }
  | { readonly _tag: 'PersistenceRequired' }
export type SidebarJournalResult = { readonly _tag: 'Applied'; readonly effects: readonly SidebarJournalEffect[]; readonly event?: { readonly _tag: 'Persisted' | 'CompletionPersisted'; readonly identity: typeof Identity.Type }; readonly request?: SidebarJournalRequest; readonly delivery?: MembershipDelivery }
  | SidebarJournalError
export type SidebarJournalRestore = { readonly _tag: 'Restored'; readonly state?: SidebarJournalState; readonly effects: readonly SidebarJournalEffect[] } | SidebarJournalError
const sameScope = (a: MembershipScope, b: MembershipScope): boolean => a.owner === b.owner && a.container === b.container && a.endpoint === b.endpoint && a.host === b.host && a.projection === b.projection

/** Unwired v2 executor for the Rust edits.rs journal contract. Run effects serially, feeding
 * returned persistence events back into the editor. Only authoritative readback emits Observed.
 * endpoint is an opaque connection identity, never a URL with credentials. Transport/auth stay outside.
 * localStorage.setItem atomically replaces one scoped record (the existing web storage precedent).
 */
export class SidebarJournal {
  readonly #key: string
  #permit: string | undefined
  constructor(readonly scope: MembershipScope, readonly storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>) {
    this.#key = `fractal.sidebar-journal@1:${JSON.stringify([scope.owner, scope.container, scope.endpoint, scope.host, scope.projection])}`
  }
  restore(): SidebarJournalRestore {
    this.#permit = undefined
    let bytes: string | null
    try { bytes = (this.storage ?? globalThis.localStorage).getItem(this.#key) }
    catch (cause) { return { _tag: 'StorageFailed', operation: 'read', cause } }
    if (bytes === null) return { _tag: 'Restored', effects: [] }
    try {
      // Inspect the envelope before decoding any version-specific state.
      const envelope = Schema.decodeUnknownSync(Envelope)(bytes)
      if (envelope.version !== 1) return { _tag: 'Version', version: envelope.version }
      const { state } = decode(bytes)
      if (!sameScope(state.scope, this.scope) || (state.request && !sameScope(state.request.fence.scope, this.scope))) return { _tag: 'ScopeMismatch' }
      switch (state.status) {
        case 'AwaitingPersistence': case 'Sending': case 'UnknownDelivery':
          return { _tag: 'Restored', state: { ...state, status: 'AwaitingPersistence' }, effects: ['PersistPending'] }
        case 'AwaitingCompletionPersistence': return { _tag: 'Restored', state, effects: ['PersistCompletion'] }
        case 'Observed': return { _tag: 'Restored', state, effects: ['PersistRemoval'] }
        case 'Intent': return { _tag: 'Restored', state, effects: ['ReadBase', 'Replan'] }
        default: return { _tag: 'Restored', state, effects: ['ReadBase'] }
      }
    } catch (cause) { return { _tag: 'InvalidRecord', cause } }
  }
  run(effect: SidebarJournalEffect, state: SidebarJournalState): SidebarJournalResult {
    if (!sameScope(state.scope, this.scope) || (state.request && !sameScope(state.request.fence.scope, this.scope))) { this.#permit = undefined; return { _tag: 'ScopeMismatch' } }
    if (effect === 'ReadBase' || effect === 'Replan') return { _tag: 'Applied', effects: [effect] }
    if (typeof effect === 'object') {
      if (!state.request || this.#permit !== JSON.stringify(state.request) || !['Sending', 'UnknownDelivery'].includes(state.status)) return { _tag: 'PersistenceRequired' }
      return { _tag: 'Applied', effects: [effect], request: structuredClone(state.request), delivery: effect.delivery }
    }
    this.#permit = undefined
    const event: SidebarPersistenceFailure | undefined = state.request ? { _tag: 'StorageFailed', identity: structuredClone(state.request.identity), phase: effect === 'PersistRemoval' ? 'Removal' : effect === 'PersistCompletion' ? 'Completion' : 'Pending' } : undefined
    if (effect === 'PersistRemoval') {
      if (state.status !== 'Observed') return { _tag: 'PersistenceRequired' }
      try { (this.storage ?? globalThis.localStorage).removeItem(this.#key); return { _tag: 'Applied', effects: [] } }
      catch (cause) { return { _tag: 'StorageFailed', operation: 'remove', cause, event } }
    }
    let bytes: string
    try { bytes = encode({ version: 1, state }); decode(bytes) }
    catch (cause) { return { _tag: 'InvalidRecord', cause } }
    try { (this.storage ?? globalThis.localStorage).setItem(this.#key, bytes) }
    catch (cause) { return { _tag: 'StorageFailed', operation: 'write', cause, event } }
    if (effect === 'PersistPending' && state.request && ['AwaitingPersistence', 'Sending', 'UnknownDelivery'].includes(state.status)) this.#permit = JSON.stringify(state.request)
    if (effect === 'PersistPending' && state.status === 'AwaitingPersistence' && state.request) {
      // The reducer alone advances AwaitingPersistence to Sending on this acknowledgement.
      return { _tag: 'Applied', effects: [], event: { _tag: 'Persisted', identity: state.request.identity } }
    }
    if (effect === 'PersistCompletion' && state.status === 'AwaitingCompletionPersistence' && state.request) {
      return { _tag: 'Applied', effects: [], event: { _tag: 'CompletionPersisted', identity: state.request.identity } }
    }
    return { _tag: 'Applied', effects: [] }
  }
}
