import { ClientError } from '@fractal/client'
import type { OrderedMembership, OrderedMembershipFrame, OrderedMembershipPage, OrderedMembershipState, Snapshot } from '@fractal/client'

export interface MembershipScope { readonly owner: string; readonly container: string; readonly endpoint: string; readonly host: string; readonly projection: string }
export interface MembershipToken { readonly scope: MembershipScope; readonly generation: number; readonly attempt: number }
export interface MembershipFrontier { readonly epoch: string; readonly changed_index: number; readonly live_count: number }
export interface MembershipComplete { readonly scope: MembershipScope; readonly frontier: MembershipFrontier; readonly snapshots: readonly Snapshot[]; readonly members: readonly OrderedMembership[] }
export interface MembershipPage { readonly frontier: MembershipFrontier; readonly snapshot: Snapshot; readonly members: readonly OrderedMembership[]; readonly has_more: boolean; readonly next?: string }
export type MembershipStatus = 'Unloaded' | 'Paging' | 'Ready' | 'Disconnected' | 'Refused' | 'Unsupported' | 'Exhausted'
export type MembershipReadEffect = { readonly _tag: 'SubscribeAndRead'; readonly token: MembershipToken } | { readonly _tag: 'Read'; readonly token: MembershipToken; readonly cursor: string }
export type MembershipEffect = MembershipReadEffect | { readonly _tag: 'PersistBase' }
export type MembershipEvent =
  | { readonly _tag: 'Refresh'; readonly scope: MembershipScope; readonly generation: number }
  | { readonly _tag: 'Disconnect' }
  | { readonly _tag: 'Page'; readonly token: MembershipToken; readonly page: MembershipPage }
  | { readonly _tag: 'Metadata'; readonly token: MembershipToken; readonly frontier: MembershipFrontier }
  | { readonly _tag: 'Expired'; readonly token: MembershipToken }
  | { readonly _tag: 'Refused'; readonly token: MembershipToken }
  | { readonly _tag: 'Unsupported'; readonly token: MembershipToken }
  | { readonly _tag: 'TransportLost'; readonly token: MembershipToken }
const sameScope = (a: MembershipScope, b: MembershipScope): boolean => a.owner === b.owner && a.container === b.container && a.endpoint === b.endpoint && a.host === b.host && a.projection === b.projection
const sameFrontier = (a: MembershipFrontier, b: MembershipFrontier): boolean => a.epoch === b.epoch && a.changed_index === b.changed_index && a.live_count === b.live_count
const conflict = (a: MembershipFrontier, b: MembershipFrontier): boolean => a.epoch === b.epoch && a.changed_index === b.changed_index && a.live_count !== b.live_count
const valid = (f: MembershipFrontier): boolean => f.epoch.length > 0 && Number.isSafeInteger(f.changed_index) && f.changed_index >= 0 && Number.isSafeInteger(f.live_count) && f.live_count >= 0 && f.live_count <= 4096

/** Unwired: the host executes effects serially; only complete sets may be persisted or projected. */
export class MembershipPaging {
  status: MembershipStatus = 'Unloaded'
  complete: MembershipComplete | undefined
  #token: MembershipToken | undefined
  #serial = 0
  #attempts = 0
  #newest: MembershipFrontier | undefined
  #frontier: MembershipFrontier | undefined
  #rows = new Map<string, OrderedMembership>()
  #cursors = new Set<string>()
  #snapshots: Snapshot[] = []
  constructor(restored?: MembershipComplete) { this.complete = restored }
  get stale(): boolean { return this.status !== 'Ready' }
  #clear(): void { this.#frontier = undefined; this.#rows.clear(); this.#cursors.clear(); this.#snapshots = [] }
  #stop(status: MembershipStatus): MembershipEffect[] { this.status = status; this.#token = undefined; this.#clear(); return [] }
  #start(token: MembershipToken): MembershipEffect[] {
    this.#clear()
    if (this.#attempts === 3 || this.#serial === Number.MAX_SAFE_INTEGER) return this.#stop('Exhausted')
    this.#attempts++; this.#serial++
    this.#token = { ...token, attempt: this.#serial }; this.status = 'Paging'
    return [{ _tag: 'SubscribeAndRead', token: this.#token }]
  }
  reduce(event: MembershipEvent): MembershipEffect[] {
    if (event._tag === 'Refresh') {
      if (this.complete && !sameScope(this.complete.scope, event.scope)) this.complete = undefined
      this.#attempts = 0; this.#newest = undefined
      return this.#start({ scope: event.scope, generation: event.generation, attempt: 0 })
    }
    if (event._tag === 'Disconnect') { this.#newest = undefined; return this.#stop('Disconnected') }
    const { token } = event
    const current = this.#token
    if (!current || token.generation !== current.generation || token.attempt !== current.attempt || !sameScope(token.scope, current.scope)) return []
    if (event._tag === 'Refused') return this.#stop('Refused')
    if (event._tag === 'Unsupported') return this.#stop('Unsupported')
    if (event._tag === 'TransportLost') { this.#newest = undefined; return this.#stop('Disconnected') }
    if (event._tag === 'Expired') return this.#start(token)
    if (event._tag === 'Metadata') {
      const frontier = event.frontier
      if (!valid(frontier)) return this.#stop('Refused')
      const held = this.#newest ?? this.#frontier ?? this.complete?.frontier
      if (held) {
        if (conflict(held, frontier)) return this.#stop('Refused')
        if (held.epoch === frontier.epoch && frontier.changed_index < held.changed_index || sameFrontier(held, frontier)) return []
      }
      const restart = this.status === 'Ready' || this.#frontier !== undefined && !sameFrontier(this.#frontier, frontier)
      if (this.status === 'Ready') this.#attempts = 0
      this.#newest = frontier
      return restart ? this.#start(token) : []
    }
    if (this.status !== 'Paging') return []
    const { page } = event
    const f = page.frontier
    if (!valid(f)) return this.#stop('Refused')
    if (page.snapshot.host_id !== token.scope.host || page.snapshot.projection_version !== token.scope.projection || this.#newest && this.#newest.epoch !== f.epoch) {
      this.#newest = undefined; return this.#start(token)
    }
    if (this.#frontier && conflict(this.#frontier, f)) return this.#stop('Refused')
    if (this.#frontier && !sameFrontier(this.#frontier, f)) return this.#start(token)
    if (this.#newest) {
      if (this.#newest.changed_index > f.changed_index) return this.#start(token)
      if (conflict(this.#newest, f)) return this.#stop('Refused')
    }
    if (page.has_more && page.members.length === 0 || page.has_more !== (page.next !== undefined) || page.next !== undefined && (page.next === '' || this.#cursors.has(page.next))) return this.#stop('Refused')
    for (const row of page.members) {
      if (row.container !== token.scope.container || this.#rows.has(row.member)) return this.#stop('Refused')
      this.#rows.set(row.member, row)
    }
    if (this.#rows.size > f.live_count) return this.#stop('Refused')
    this.#frontier = f; this.#newest = f; this.#snapshots.push(page.snapshot)
    if (page.next !== undefined) { this.#cursors.add(page.next); return [{ _tag: 'Read', token, cursor: page.next }] }
    if (this.#rows.size !== f.live_count) return this.#stop('Refused')
    const members = [...this.#rows.values()].sort((a, b) => {
      const left = Array.from(a.member, c => c.codePointAt(0)!)
      const right = Array.from(b.member, c => c.codePointAt(0)!)
      for (let i = 0; i < Math.min(left.length, right.length); i++) if (left[i] !== right[i]) return left[i]! - right[i]!
      return left.length - right.length
    })
    this.complete = { scope: token.scope, frontier: f, snapshots: this.#snapshots, members }
    this.status = 'Ready'; this.#clear()
    return [{ _tag: 'PersistBase' }]
  }
}

// D must supply this metadata on every page/frame; current clients cannot invent an epoch.
type WireMetadata = OrderedMembershipState & { readonly epoch?: string }
export type MembershipWirePage = OrderedMembershipPage & { readonly membership?: WireMetadata }
export const membershipMetadata = (token: MembershipToken, metadata: WireMetadata): MembershipEvent =>
  metadata.container !== token.scope.container || !metadata.epoch ? { _tag: 'Refused', token }
    : { _tag: 'Metadata', token, frontier: { epoch: metadata.epoch, changed_index: metadata.changed_index, live_count: metadata.live_count } }
export const membershipFrame = (token: MembershipToken, frame: OrderedMembershipFrame): MembershipEvent =>
  frame.snapshot.host_id !== token.scope.host || frame.snapshot.projection_version !== token.scope.projection
    ? { _tag: 'Expired', token } : membershipMetadata(token, frame.membership)
export const membershipHttpFailure = (token: MembershipToken, status: number, code: string): MembershipEvent => {
  const restart = status === 409 && (code === 'membership-frontier-reset' || code === 'membership-collection-changed')
    || status === 410 && code === 'page-cursor-expired'
  return { _tag: status === 404 ? 'Unsupported' : restart ? 'Expired' : 'Refused', token }
}
export interface MembershipReadPort {
  /** Resolve only after the attempt's metadata subscription is installed; host owns cancellation. */
  readonly subscribe: (token: MembershipToken) => Promise<void>
  readonly read: (token: MembershipToken, cursor?: string) => Promise<{ readonly value: MembershipWirePage; readonly snapshot: Snapshot }>
}
export const executeMembershipRead = async (effect: MembershipReadEffect, port: MembershipReadPort): Promise<MembershipEvent> => {
  const { token } = effect
  try {
    if (effect._tag === 'SubscribeAndRead') await port.subscribe(token)
    const { value, snapshot } = await port.read(token, effect._tag === 'Read' ? effect.cursor : undefined)
    if (value.collection !== 'ordered-memberships' || value.filters.person !== token.scope.owner || value.filters.subject !== token.scope.container || !value.membership) return { _tag: 'Refused', token }
    const metadata = membershipMetadata(token, value.membership)
    return metadata._tag !== 'Metadata' ? metadata : { _tag: 'Page', token, page: { frontier: metadata.frontier, snapshot, members: value.items, has_more: value.page.has_more, ...(value.page.next_cursor == null ? {} : { next: value.page.next_cursor }) } }
  } catch (error) {
    return error instanceof ClientError ? membershipHttpFailure(token, error.status, error.response.code) : { _tag: 'TransportLost', token }
  }
}
