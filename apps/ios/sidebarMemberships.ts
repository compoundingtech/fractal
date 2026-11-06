import type { OrderedMembership, Snapshot } from '../../clients/typescript/fractal-client';

export type MembershipScope = { owner: string; container: string; endpoint: string; host: string; projection: string };
export type MembershipToken = { scope: MembershipScope; generation: number; attempt: number };
export type MembershipFrontier = { epoch: string; changed_index: number; live_count: number };
export type MembershipPage = { frontier: MembershipFrontier; snapshot: Snapshot; members: OrderedMembership[]; has_more: boolean; next?: string };
export type MembershipComplete = { scope: MembershipScope; frontier: MembershipFrontier; snapshots: Snapshot[]; members: OrderedMembership[] };
export type MembershipEvent = { type: 'Refresh'; scope: MembershipScope; generation: number } | { type: 'Disconnect'; token?: MembershipToken }
  | { type: 'Page'; token: MembershipToken; page: MembershipPage } | { type: 'Metadata'; token: MembershipToken; frontier: MembershipFrontier }
  | { type: 'Expired'; token: MembershipToken } | { type: 'Refused'; token: MembershipToken };
export type MembershipEffect = { type: 'SubscribeAndRead'; token: MembershipToken } | { type: 'Read'; token: MembershipToken; cursor: string } | { type: 'PersistBase' };
const sameScope = (a: MembershipScope, b: MembershipScope): boolean => a.owner === b.owner && a.container === b.container && a.endpoint === b.endpoint && a.host === b.host && a.projection === b.projection;
const sameFrontier = (a: MembershipFrontier, b: MembershipFrontier): boolean => a.epoch === b.epoch && a.changed_index === b.changed_index && a.live_count === b.live_count;
const conflict = (a: MembershipFrontier, b: MembershipFrontier): boolean => a.epoch === b.epoch && a.changed_index === b.changed_index && a.live_count !== b.live_count;
/** Unwired: SubscribeAndRead executors must subscribe BEFORE dispatching the first read.
 * Restored/last-complete data stays stale until a complete, consistent attempt publishes. */
export class SidebarMemberships {
  status: 'Unloaded' | 'Paging' | 'Ready' | 'Disconnected' | 'Refused' | 'Exhausted' = 'Unloaded';
  private token?: MembershipToken;
  private serial = 0;
  private attempts = 0;
  private newest?: MembershipFrontier;
  private frontier?: MembershipFrontier;
  private rows = new Map<string, OrderedMembership>();
  private cursors = new Set<string>();
  private snapshots: Snapshot[] = [];
  constructor(public complete?: MembershipComplete) {}
  get stale(): boolean { return this.status !== 'Ready'; }
  private clear(): void { this.frontier = undefined; this.rows.clear(); this.cursors.clear(); this.snapshots = []; }
  private stop(status: typeof this.status): MembershipEffect[] { this.status = status; this.token = undefined; this.clear(); return []; }
  private start(token: MembershipToken): MembershipEffect[] {
    this.clear();
    if (this.attempts === 3 || this.serial === Number.MAX_SAFE_INTEGER) return this.stop('Exhausted');
    this.attempts++; this.token = { ...token, attempt: ++this.serial }; this.status = 'Paging';
    return [{ type: 'SubscribeAndRead', token: this.token }];
  }
  reduce(event: MembershipEvent): MembershipEffect[] {
    if (event.type === 'Refresh') {
      if (this.complete && !sameScope(this.complete.scope, event.scope)) this.complete = undefined;
      this.attempts = 0; this.newest = undefined;
      return this.start({ scope: event.scope, generation: event.generation, attempt: 0 });
    }
    if (event.type === 'Disconnect' && !event.token) { this.newest = undefined; return this.stop('Disconnected'); }
    const { token } = event;
    if (!token || !this.token || token.generation !== this.token.generation || token.attempt !== this.token.attempt || !sameScope(token.scope, this.token.scope)) return [];
    if (event.type === 'Disconnect') { this.newest = undefined; return this.stop('Disconnected'); }
    if (event.type === 'Refused') return this.stop('Refused');
    if (event.type === 'Expired') return this.start(token);
    if (event.type === 'Page' && this.status !== 'Paging') return [];
    const incoming = event.type === 'Metadata' ? event.frontier : event.page.frontier;
    if (!incoming.epoch || !Number.isSafeInteger(incoming.changed_index) || incoming.changed_index < 0 || !Number.isSafeInteger(incoming.live_count) || incoming.live_count < 0) return this.stop('Refused');
    if (event.type === 'Metadata') {
      const held = this.newest ?? this.frontier ?? this.complete?.frontier;
      if (held) {
        if (conflict(held, incoming)) return this.stop('Refused');
        if (held.epoch === incoming.epoch && incoming.changed_index < held.changed_index || sameFrontier(held, incoming)) return [];
      }
      const restart = this.status === 'Ready' || this.frontier !== undefined && !sameFrontier(this.frontier, incoming);
      if (this.status === 'Ready') this.attempts = 0;
      this.newest = incoming;
      return restart ? this.start(token) : [];
    }
    const page = event.page;
    if (page.snapshot.host_id !== token.scope.host || page.snapshot.projection_version !== token.scope.projection || this.newest && this.newest.epoch !== incoming.epoch) {
      this.newest = undefined; return this.start(token);
    }
    if (this.frontier && conflict(this.frontier, incoming)) return this.stop('Refused');
    if (this.frontier && !sameFrontier(this.frontier, incoming)) return this.start(token);
    if (this.newest) {
      if (this.newest.changed_index > incoming.changed_index) return this.start(token);
      if (conflict(this.newest, incoming)) return this.stop('Refused');
    }
    if (page.has_more && page.members.length === 0 || page.has_more !== (page.next !== undefined) || page.next !== undefined && (!page.next || this.cursors.has(page.next))) return this.stop('Refused');
    if (page.next !== undefined) this.cursors.add(page.next);
    for (const row of page.members) {
      if (row.container !== token.scope.container || this.rows.has(row.member)) return this.stop('Refused');
      this.rows.set(row.member, row);
    }
    if (this.rows.size > incoming.live_count) return this.stop('Refused');
    this.frontier = incoming; this.newest = incoming; this.snapshots.push(page.snapshot);
    if (page.next !== undefined) return [{ type: 'Read', token, cursor: page.next }];
    if (this.rows.size !== incoming.live_count) return this.stop('Refused');
    this.complete = { scope: token.scope, frontier: incoming, snapshots: this.snapshots, members: [...this.rows.values()] };
    this.status = 'Ready'; this.clear();
    return [{ type: 'PersistBase' }];
  }
}
