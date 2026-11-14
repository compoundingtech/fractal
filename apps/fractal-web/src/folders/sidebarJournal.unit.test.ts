// @vitest-environment jsdom
import { beforeEach, expect, it } from 'vitest'
import { SidebarJournal, type SidebarJournalState } from './sidebarJournal.ts'

const scope = { owner: 'owner-a', container: 'container-a', endpoint: 'endpoint-a', host: 'host-a', projection: '1' }
const identity = { actor: 'actor-a', action_id: 'action-a', idempotency_key: 'key-a' }
const state = (): SidebarJournalState => ({
  scope, intent: { node: { kind: 'subject', subject: 'member-a' }, parent: 'folder-a', before: null },
  status: 'AwaitingPersistence', refusal: null, dependent: null, folder_completed: false, used: [identity],
  request: { identity, fence: { scope, ready: true, revision: 'revision-a', snapshot: { id: 'snapshot-a', host_id: 'host-a', store_index: 4, projection_version: '1', created_at: 'created', expires_at: 'expires' } },
    operations: { Memberships: [{ op: 'place', member: 'member-a', bucket: 'folder-a', key: 'a0' }] } },
})
beforeEach(() => localStorage.clear())

it.each(['AwaitingPersistence', 'Sending', 'UnknownDelivery'] as const)('restarts %s through a new persistence barrier and replays exact bytes', status => {
  const journal = new SidebarJournal(scope)
  const pending = { ...state(), status }
  expect(journal.run({ _tag: 'Send', delivery: 'Replay' }, { ...pending, status: 'Sending' })._tag).toBe('PersistenceRequired')
  expect(journal.run('PersistPending', pending)._tag).toBe('Applied')
  const restarted = new SidebarJournal(scope)
  const restored = restarted.restore()
  expect(restored._tag).toBe('Restored')
  if (restored._tag !== 'Restored' || !restored.state) throw new Error('missing restored state')
  expect(restored.effects).toEqual(['PersistPending'])
  expect(restored.state.status).toBe('AwaitingPersistence')
  expect(restarted.run({ _tag: 'Send', delivery: 'Replay' }, { ...restored.state, status: 'Sending' })._tag).toBe('PersistenceRequired')
  expect(restarted.run('PersistPending', restored.state)).toEqual({ _tag: 'Applied', effects: [], event: { _tag: 'Persisted', identity } })
  expect(restarted.run({ _tag: 'Send', delivery: 'Replay' }, { ...restored.state, status: 'Sending' })).toEqual({ _tag: 'Applied', effects: [{ _tag: 'Send', delivery: 'Replay' }], request: pending.request, delivery: 'Replay' })
  const changed = structuredClone(restored.state)
  expect(restarted.run({ _tag: 'Send', delivery: 'Replay' }, { ...changed, status: 'Sending', request: { ...changed.request!, identity: { ...identity, idempotency_key: 'changed' } } })._tag).toBe('PersistenceRequired')
})
it.each(['AwaitingObservation', 'AwaitingFreshBase', 'Refused', 'PartiallyApplied', 'Intent', 'Observed'] as const)('never resends restored %s', status => {
  const journal = new SidebarJournal(scope)
  const pending = { ...state(), status, folder_completed: true }
  expect(journal.run('PersistPending', pending)._tag).toBe('Applied')
  const restarted = new SidebarJournal(scope)
  const restored = restarted.restore()
  expect(restored).toEqual({ _tag: 'Restored', state: pending, effects: status === 'Intent' ? ['ReadBase', 'Replan'] : status === 'Observed' ? ['PersistRemoval'] : ['ReadBase'] })
  expect(restarted.run({ _tag: 'Send', delivery: 'Replay' }, { ...pending, status: 'Sending' })._tag).toBe('PersistenceRequired')
})
it('persists authoritative completion before allowing a dependent step and retains partial completion', () => {
  const journal = new SidebarJournal(scope)
  const completed = { ...state(), status: 'AwaitingCompletionPersistence' as const, folder_completed: true, dependent: { ...identity, action_id: 'action-b', idempotency_key: 'key-b' } }
  expect(journal.run('PersistCompletion', completed)).toEqual({ _tag: 'Applied', effects: [], event: { _tag: 'CompletionPersisted', identity } })
  const restarted = new SidebarJournal(scope)
  expect(restarted.restore()).toEqual({ _tag: 'Restored', state: completed, effects: ['PersistCompletion'] })
  expect(restarted.run({ _tag: 'Send', delivery: 'Replay' }, { ...completed, status: 'Sending' })._tag).toBe('PersistenceRequired')
  const partial = { ...completed, status: 'PartiallyApplied' as const, refusal: 'DestinationMissing' as const }
  journal.run('PersistPending', partial)
  expect(restarted.restore()).toEqual({ _tag: 'Restored', state: partial, effects: ['ReadBase'] })
  journal.run('PersistRemoval', { ...completed, status: 'Observed' })
  expect(restarted.restore()).toEqual({ _tag: 'Restored', effects: [] })
})
it('blocks sends and completion acknowledgements on failed writes', () => {
  const cause = new DOMException('quota', 'QuotaExceededError')
  const storage = { getItem: localStorage.getItem.bind(localStorage), removeItem: localStorage.removeItem.bind(localStorage), setItem: () => { throw cause } }
  const journal = new SidebarJournal(scope, storage)
  expect(journal.run('PersistPending', state())).toEqual({ _tag: 'StorageFailed', operation: 'write', cause, event: { _tag: 'StorageFailed', identity, phase: 'Pending' } })
  expect(journal.run({ _tag: 'Send', delivery: 'Replay' }, { ...state(), status: 'Sending' })._tag).toBe('PersistenceRequired')
  expect(journal.run('PersistCompletion', { ...state(), status: 'AwaitingCompletionPersistence' })._tag).toBe('StorageFailed')
})
it('refuses unknown versions before decoding state, corruption, foreign scopes and credential fields', () => {
  const journal = new SidebarJournal(scope)
  journal.run('PersistPending', state())
  const key = localStorage.key(0)!
  for (const [bytes, tag] of [
    ['{"version":2,"state":"not-v1"}', 'Version'], ['invalid', 'InvalidRecord'],
    [JSON.stringify({ version: 1, state: { ...state(), scope: { ...scope, owner: 'owner-b' } } }), 'ScopeMismatch'],
    [JSON.stringify({ version: 1, state: { ...state(), credentials: 'forbidden' } }), 'InvalidRecord'],
  ]) {
    localStorage.setItem(key, bytes!)
    expect(journal.restore()._tag).toBe(tag)
  }
  expect(journal.run('PersistPending', { ...state(), scope: { ...scope, endpoint: 'endpoint-b' } })._tag).toBe('ScopeMismatch')
})
it('isolates scoped records and reports read/removal failures without fallback success', () => {
  const journal = new SidebarJournal(scope)
  journal.run('PersistPending', state())
  expect(new SidebarJournal({ ...scope, container: 'container-b' }).restore()).toEqual({ _tag: 'Restored', effects: [] })
  const cause = new DOMException('denied', 'SecurityError')
  const storage = { getItem: () => { throw cause }, setItem: localStorage.setItem.bind(localStorage), removeItem: () => { throw cause } }
  const failed = new SidebarJournal(scope, storage)
  expect(failed.restore()).toEqual({ _tag: 'StorageFailed', operation: 'read', cause })
  expect(failed.run('PersistRemoval', { ...state(), status: 'Observed' })).toEqual({ _tag: 'StorageFailed', operation: 'remove', cause, event: { _tag: 'StorageFailed', identity, phase: 'Removal' } })
})
it('replays unknown delivery exactly in-process, but a later failed write revokes the send permit', () => {
  let fail = false
  const cause = new DOMException('quota', 'QuotaExceededError')
  const storage = { getItem: localStorage.getItem.bind(localStorage), removeItem: localStorage.removeItem.bind(localStorage), setItem: (key: string, bytes: string) => {
    if (fail) throw cause
    localStorage.setItem(key, bytes)
  } }
  const journal = new SidebarJournal(scope, storage)
  journal.run('PersistPending', state())
  const uncertain = { ...state(), status: 'UnknownDelivery' as const }
  journal.run('PersistPending', uncertain)
  expect(journal.run({ _tag: 'Send', delivery: 'Replay' }, { ...uncertain, status: 'Sending' })).toEqual({ _tag: 'Applied', effects: [{ _tag: 'Send', delivery: 'Replay' }], request: uncertain.request, delivery: 'Replay' })
  fail = true
  expect(journal.run('PersistPending', uncertain)._tag).toBe('StorageFailed')
  expect(journal.run({ _tag: 'Send', delivery: 'Replay' }, { ...uncertain, status: 'Sending' })._tag).toBe('PersistenceRequired')
  fail = false
  journal.run('PersistPending', { ...uncertain, status: 'AwaitingObservation' })
  expect(journal.run({ _tag: 'Send', delivery: 'Replay' }, { ...uncertain, status: 'Sending' })._tag).toBe('PersistenceRequired')
})
it.each([-1, 1.5, Number.MAX_SAFE_INTEGER + 1, Number.NaN])('refuses invalid durable numeric fields: %s', value => {
  const journal = new SidebarJournal(scope)
  const pending = state()
  journal.run('PersistPending', pending)
  const key = localStorage.key(0)!
  const bytes = localStorage.getItem(key)!
  for (const field of ['version', 'store_index']) {
    const invalid = bytes.replace(`"${field}":${field === 'version' ? 1 : 4}`, `"${field}":${value}`)
    localStorage.setItem(key, invalid)
    expect(journal.restore()._tag).toBe('InvalidRecord')
    expect(journal.run({ _tag: 'Send', delivery: 'Replay' }, { ...pending, status: 'Sending' })._tag).toBe('PersistenceRequired')
  }
  const request = pending.request!
  expect(journal.run('PersistPending', { ...pending, request: { ...request, fence: { ...request.fence, snapshot: { ...request.fence.snapshot, store_index: value } } } })._tag).toBe('InvalidRecord')
})
it('accepts the largest exact u64 browser index but refuses a version outside Rust u32', () => {
  const journal = new SidebarJournal(scope)
  const pending = state()
  const request = pending.request!
  const exact = { ...pending, request: { ...request, fence: { ...request.fence, snapshot: { ...request.fence.snapshot, store_index: Number.MAX_SAFE_INTEGER } } } }
  expect(journal.run('PersistPending', exact)._tag).toBe('Applied')
  expect(journal.restore()).toEqual({ _tag: 'Restored', state: exact, effects: ['PersistPending'] })
  const key = localStorage.key(0)!
  localStorage.setItem(key, '{"version":4294967296,"state":null}')
  expect(journal.restore()._tag).toBe('InvalidRecord')
})
