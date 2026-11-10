import type { ArrangementOperation } from '../../clients/typescript/fractal-client';
import type { JournalEffect, JournalPersistence, JournalReducer, JournalStatus, WriteDelivery } from './sidebarJournal';
import type { MembershipScope } from './sidebarMemberships';
import type { Intent } from './sidebarPlanning';

export type WriteIdentity = { actor: string; action_id: string; idempotency_key: string };
export type WriteBase = { scope: MembershipScope; snapshot: { id: string; host_id: string; store_index: number; projection_version: string; created_at: string; published_at?: string | null }; revision: string; ready: boolean };
export type MembershipOperation = { op: 'place'; member: string; bucket: string | null; key: string };
export type WritePlan = { folders: ArrangementOperation[]; memberships: MembershipOperation[] };
export type WriteRequest = { identity: WriteIdentity; fence: WriteBase; operations: { Folders: ArrangementOperation[] } | { Memberships: MembershipOperation[] } };
export type WriteRefusal = 'InvalidPlan' | 'StaleFence' | 'Unauthorized' | 'DestinationMissing' | 'Conflict' | 'Storage';
// next_delivery is transient: missing after journal decode means the identity may already have sent.
export type WriteState = { scope: MembershipScope; intent: Intent; status: JournalStatus; refusal: WriteRefusal | null; request: WriteRequest | null; dependent: WriteIdentity | null; folder_completed: boolean; used: WriteIdentity[]; next_delivery?: WriteDelivery };
export type WriteEvent = { type: 'Plan'; base: WriteBase; plan: WritePlan; folder: WriteIdentity; membership: WriteIdentity }
  | { type: 'Fresh'; base: WriteBase; operations: MembershipOperation[] | { error: WriteRefusal } }
  | { type: 'Persisted' | 'CompletionPersisted' | 'Ack' | 'Unknown' | 'Observed'; identity: WriteIdentity }
  | { type: 'Refused'; identity: WriteIdentity; reason: WriteRefusal }
  | { type: 'StorageFailed'; identity: WriteIdentity; phase: JournalPersistence }
  | { type: 'Replay' | 'Retry' | 'StoredRefusal' };
export const createWriteState = (scope: MembershipScope, intent: Intent): WriteState => ({ scope: structuredClone(scope), intent: structuredClone(intent), status: 'Intent', refusal: null, request: null, dependent: null, folder_completed: false, used: [] });
const sameScope = (a: MembershipScope, b: MembershipScope): boolean => a.owner === b.owner && a.container === b.container && a.endpoint === b.endpoint && a.host === b.host && a.projection === b.projection;
const sameId = (a: WriteIdentity, b: WriteIdentity): boolean => a.actor === b.actor && a.action_id === b.action_id && a.idempotency_key === b.idempotency_key;
const validBase = (s: WriteState, b: WriteBase): boolean => b.ready && sameScope(s.scope, b.scope) && b.snapshot.host_id === s.scope.host && b.snapshot.projection_version === s.scope.projection;
const unused = (s: WriteState, id: WriteIdentity): boolean => !!id.actor && !!id.action_id && !!id.idempotency_key && !s.used.some(u => u.action_id === id.action_id || u.idempotency_key === id.idempotency_key);
const refuse = (s: WriteState, reason: WriteRefusal): JournalEffect[] => { s.refusal = reason; s.status = s.folder_completed ? 'PartiallyApplied' : 'Refused'; return ['PersistPending', 'ReadBase']; };
const stage = (s: WriteState, identity: WriteIdentity, fence: WriteBase, operations: WriteRequest['operations']): JournalEffect[] => {
  if (!unused(s, identity)) return refuse(s, 'InvalidPlan');
  s.used.push(structuredClone(identity)); s.request = structuredClone({ identity, fence, operations }); s.next_delivery = 'First'; s.status = 'AwaitingPersistence'; return ['PersistPending'];
};
const reduce = (s: WriteState, e: WriteEvent): JournalEffect[] => {
  if (e.type === 'Plan' && s.status === 'Intent') {
    if (!validBase(s, e.base) || s.request && (e.base.snapshot.id === s.request.fence.snapshot.id || e.base.snapshot.store_index < s.request.fence.snapshot.store_index)) return ['ReadBase'];
    if (e.plan.folders.some(o => !['folder.create', 'folder.rename', 'folder.move', 'folder.delete'].includes(o.op)) || !e.plan.folders.length && !e.plan.memberships.length) return refuse(s, 'InvalidPlan');
    if (e.plan.folders.length) {
      if (e.plan.memberships.length) {
        if (!unused(s, e.membership) || e.folder.action_id === e.membership.action_id || e.folder.idempotency_key === e.membership.idempotency_key) return refuse(s, 'InvalidPlan');
        s.dependent = structuredClone(e.membership);
      }
      return stage(s, e.folder, e.base, { Folders: e.plan.folders });
    }
    return stage(s, e.membership, e.base, { Memberships: e.plan.memberships });
  }
  if (e.type === 'Fresh' && s.status === 'AwaitingFreshBase') {
    const previous = s.request!.fence;
    if (!validBase(s, e.base) || e.base.snapshot.id === previous.snapshot.id || e.base.revision === previous.revision || e.base.snapshot.store_index < previous.snapshot.store_index) return ['ReadBase'];
    if ('error' in e.operations) return refuse(s, e.operations.error);
    if (!e.operations.length) return refuse(s, 'InvalidPlan');
    const identity = s.dependent!; s.dependent = null; return stage(s, identity, e.base, { Memberships: e.operations });
  }
  if (e.type === 'Retry' && (s.status === 'Refused' || s.status === 'PartiallyApplied')) { s.status = 'Intent'; s.refusal = null; s.dependent = null; return ['ReadBase', 'Replan']; }
  if (e.type === 'Replay' && s.status === 'UnknownDelivery') { s.status = 'Sending'; s.next_delivery = 'Replay'; return [{ type: 'Send', delivery: 'Replay' }]; }
  if (!('identity' in e) || !s.request || !sameId(s.request.identity, e.identity)) return [];
  if (e.type === 'StorageFailed' && (s.status === 'AwaitingPersistence' && e.phase === 'Pending' || s.status === 'AwaitingCompletionPersistence' && e.phase === 'Completion')) return refuse(s, 'Storage');
  if (e.type === 'Persisted' && s.status === 'AwaitingPersistence') { s.status = 'Sending'; const delivery = s.next_delivery ?? 'Replay'; s.next_delivery = 'Replay'; return [{ type: 'Send', delivery }]; }
  if (e.type === 'CompletionPersisted' && s.status === 'AwaitingCompletionPersistence') { s.status = s.dependent ? 'AwaitingFreshBase' : 'Observed'; return s.dependent ? ['ReadBase'] : ['PersistRemoval']; }
  if (e.type === 'Ack' && (s.status === 'Sending' || s.status === 'UnknownDelivery')) { s.status = 'AwaitingObservation'; return ['PersistPending', 'ReadBase']; }
  if (e.type === 'Unknown' && s.status === 'Sending') { s.status = 'UnknownDelivery'; return ['PersistPending']; }
  if (['Sending', 'UnknownDelivery', 'AwaitingObservation'].includes(s.status)) {
    if (e.type === 'Observed') { if ('Folders' in s.request.operations) s.folder_completed = true; s.status = 'AwaitingCompletionPersistence'; return ['PersistCompletion']; }
    if (e.type === 'Refused') return refuse(s, e.reason);
  }
  return [];
};

// Decode rebuilds only durable fields, so nested transport credentials cannot leak into writes.
type Shape = 'string' | 'boolean' | 'u64' | readonly string[] | { fields: Record<string, Shape>; exact?: boolean } | { array: Shape } | { nullable: Shape } | { optional: Shape } | { union: Shape[] };
const object = (fields: Record<string, Shape>, exact = false): Shape => ({ fields, exact });
const nullable = (shape: Shape): Shape => ({ nullable: shape });
const list = (shape: Shape): Shape => ({ array: shape });
const node = { union: [object({ _tag: ['Folder'], id: 'string' }), object({ _tag: ['Subject'], subject: 'string' })] } satisfies Shape;
const parent = { union: [node, object({ _tag: ['Root'] })] } satisfies Shape;
const scope = object({ owner: 'string', container: 'string', endpoint: 'string', host: 'string', projection: 'string' });
const identity = object({ actor: 'string', action_id: 'string', idempotency_key: 'string' });
const base = object({ scope, snapshot: object({ id: 'string', host_id: 'string', store_index: 'u64', projection_version: 'string', created_at: 'string', published_at: { optional: nullable('string') } }), revision: 'string', ready: 'boolean' });
const membership = object({ op: ['place'], member: 'string', bucket: nullable('string'), key: 'string' });
const folder = { union: [object({ op: ['folder.create'], id: 'string', name: 'string', parent: nullable('string'), key: 'string' }), object({ op: ['folder.rename'], id: 'string', name: 'string' }), object({ op: ['folder.move'], id: 'string', parent: nullable('string'), key: 'string' }), object({ op: ['folder.delete'], id: 'string' })] } satisfies Shape;
const refusal: Shape = ['InvalidPlan', 'StaleFence', 'Unauthorized', 'DestinationMissing', 'Conflict', 'Storage'];
const stateShape = object({ scope, intent: object({ node, parent, before: nullable(node) }), status: ['Intent', 'AwaitingPersistence', 'Sending', 'UnknownDelivery', 'AwaitingObservation', 'AwaitingCompletionPersistence', 'AwaitingFreshBase', 'Observed', 'Refused', 'PartiallyApplied'], refusal: nullable(refusal), request: nullable(object({ identity, fence: base, operations: { union: [object({ Folders: list(folder) }, true), object({ Memberships: list(membership) }, true)] } })), dependent: nullable(identity), folder_completed: 'boolean', used: list(identity) }, true);
const parse = (shape: Shape, value: unknown): unknown => {
  if (typeof shape === 'string') {
    if (shape === 'u64') return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
    return typeof value === shape ? value : undefined;
  }
  if (Array.isArray(shape)) return shape.includes(value) ? value : undefined;
  if ('optional' in shape) return parse(shape.optional, value);
  if ('nullable' in shape) return value === null ? null : parse(shape.nullable, value);
  if ('union' in shape) { for (const alternative of shape.union) { const parsed = parse(alternative, value); if (parsed !== undefined) return parsed; } return undefined; }
  if ('array' in shape) {
    if (!Array.isArray(value)) return undefined;
    const parsed = value.map(v => parse(shape.array, v)); return parsed.includes(undefined) ? undefined : parsed;
  }
  if (!('fields' in shape) || !value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  if (shape.exact && Object.keys(value).some(k => !(k in shape.fields))) return undefined;
  const result: Record<string, unknown> = {};
  for (const [key, field] of Object.entries(shape.fields)) {
    if (typeof field === 'object' && 'optional' in field && !Reflect.has(value, key)) continue;
    const parsed = parse(field, Reflect.get(value, key)); if (parsed === undefined) return undefined; result[key] = parsed;
  }
  return result;
};
export const writeReducer: JournalReducer<WriteState, WriteEvent> = {
  reduce,
  encode: state => {
    const record = parse(stateShape, { scope: state.scope, intent: state.intent, status: state.status, refusal: state.refusal, request: state.request, dependent: state.dependent, folder_completed: state.folder_completed, used: state.used });
    if (record === undefined) throw new Error('Invalid sidebar write state');
    return record;
  },
  decode: value => parse(stateShape, value) as WriteState | undefined,
  persisted: (state, completion) => state.request ? { type: completion ? 'CompletionPersisted' : 'Persisted', identity: structuredClone(state.request.identity) } : { type: 'StoredRefusal' },
  storageFailed: (state, phase) => state.request ? { type: 'StorageFailed', identity: structuredClone(state.request.identity), phase } : { type: 'StoredRefusal' },
};
