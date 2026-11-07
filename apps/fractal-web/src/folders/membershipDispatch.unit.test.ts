import { expect, it } from '@effect/vitest'
import { ClientError } from '@fractal/client'
import { MembershipEditor } from './edit.ts'
import { SidebarJournal, type SidebarJournalRequest, type SidebarJournalState } from './sidebarJournal.ts'
import { membershipWriteRequest, membershipWriteResult } from './sidebarAdapter.ts'
const scope = { owner: 'person/test', container: 'arrangement/test', endpoint: 'endpoint/test', host: 'host/test', projection: '1' }
const identity = (n: number) => ({ actor: 'person/test', action_id: `action/${n}`, idempotency_key: `key/${n}` })
const base = (n: number): SidebarJournalRequest['fence'] => ({ scope, revision: `claim/${n}`, ready: true, snapshot: { id: `snapshot/${n}`, host_id: scope.host, store_index: n, projection_version: '1', created_at: 'created', expires_at: 'expires' } })
const intent: SidebarJournalState['intent'] = { node: { kind: 'subject', subject: 'machine/test' }, parent: 'folder/test', before: null }
const memberships = [{ op: 'place', member: 'machine/test', bucket: 'folder/test', key: 'a0' }] as const
const folders = [{ op: 'folder.create', id: 'folder/test', name: 'Folder', parent: null, key: 'a0' }] as const
const setup = () => {
  const values = new Map<string, string>()
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value) }, removeItem: (key: string) => { values.delete(key) } }
  const journal = new SidebarJournal(scope, storage)
  return { journal, storage, editor: new MembershipEditor(journal, intent) }
}
const stage = (editor: MembershipEditor, compound = false) => editor.reduce({ _tag: 'Plan', base: base(1), folders: compound ? folders : [], memberships, folder: identity(1), membership: identity(2) })
it('folder-first wire payload and fresh membership fence require durable authoritative completion', () => {
  const { editor } = setup()
  expect(stage(editor, true)).toEqual(['PersistPending'])
  expect(editor.execute('Send')._tag).toBe('PersistenceRequired')
  expect(editor.execute('PersistPending')).toMatchObject({ _tag: 'Applied', effects: ['Send'] })
  const sent = editor.execute('Send')
  if (sent._tag !== 'Applied' || !sent.request) throw new Error('journal did not release persisted request')
  expect(membershipWriteRequest(sent.request)).toEqual({ api_version: 'st3.client.v0', type: 'arrangement.edit', id: 'action/1', idempotency_key: 'key/1', parameters: { subject: scope.container, owner: scope.owner, operations: folders }, fence: { snapshot_id: 'snapshot/1', subject_revisions: { [scope.container]: 'claim/1' } } })
  editor.reduce(membershipWriteResult(sent.request, { action_id: 'action/1', status: 'completed' }))
  expect(editor.state.status).toBe('AwaitingObservation')
  expect(editor.reduce({ _tag: 'Fresh', base: base(2), memberships })).toEqual([])
  expect(editor.reduce({ _tag: 'Observed', identity: identity(1) })).toEqual(['PersistCompletion'])
  expect(editor.reduce({ _tag: 'Fresh', base: base(2), memberships })).toEqual([])
  expect(editor.execute('PersistCompletion')).toMatchObject({ effects: ['ReadBase'] })
  for (const stale of [base(1), { ...base(2), revision: 'claim/1' }, { ...base(2), ready: false }, { ...base(2), snapshot: { ...base(2).snapshot, store_index: 0 } }]) expect(editor.reduce({ _tag: 'Fresh', base: stale, memberships })).toEqual(['ReadBase'])
  expect(editor.reduce({ _tag: 'Fresh', base: base(2), memberships })).toEqual(['PersistPending'])
  editor.execute('PersistPending')
  const second = editor.execute('Send')
  if (second._tag !== 'Applied' || !second.request) throw new Error('missing dependent request')
  expect(membershipWriteRequest(second.request)).toEqual({ api_version: 'st3.client.v0', type: 'arrangement.membership.edit', id: 'action/2', idempotency_key: 'key/2', parameters: { subject: scope.container, owner: scope.owner, operations: memberships }, fence: { snapshot_id: 'snapshot/2', subject_revisions: { [scope.container]: 'claim/2' } } })
  editor.reduce({ _tag: 'Observed', identity: identity(2) })
  expect(editor.execute('PersistCompletion')).toMatchObject({ effects: ['PersistRemoval'] })
  expect(editor.execute('PersistRemoval')._tag).toBe('Applied')
})
it('unknown delivery restores and replays exact identity, parameters and fence', () => {
  const { editor, journal } = setup()
  stage(editor); editor.execute('PersistPending')
  const request = editor.state.request!
  editor.reduce(membershipWriteResult(request, new Error('transport lost')))
  editor.execute('PersistPending')
  const restored = journal.restore()
  if (restored._tag !== 'Restored' || !restored.state) throw new Error('restore failed')
  const replay = new MembershipEditor(journal, intent, restored.state)
  expect(replay.reduce({ _tag: 'Retry' })).toEqual([])
  replay.execute('PersistPending')
  expect(replay.execute('Send')).toMatchObject({ request })
  expect(replay.reduce({ _tag: 'Ack', identity: identity(9) })).toEqual([])
})
it('folder success plus destination refusal persists partial success without replaying folders', () => {
  const { editor, journal } = setup()
  stage(editor, true); editor.execute('PersistPending')
  editor.reduce({ _tag: 'Observed', identity: identity(1) }); editor.execute('PersistCompletion')
  editor.reduce({ _tag: 'Fresh', base: base(2), memberships: [], refusal: 'DestinationMissing' })
  expect(editor.state).toMatchObject({ status: 'PartiallyApplied', folder_completed: true, refusal: 'DestinationMissing' })
  editor.execute('PersistPending')
  const restored = journal.restore()
  expect(restored).toMatchObject({ effects: ['ReadBase'], state: { status: 'PartiallyApplied', folder_completed: true } })
  expect(editor.reduce({ _tag: 'Replay' })).toEqual([])
  editor.reduce({ _tag: 'Retry' })
  expect(editor.reduce({ _tag: 'Plan', base: base(2), folders: [], memberships, folder: identity(3), membership: identity(4) })).toEqual(['PersistPending'])
  expect(editor.state.request?.operations).toEqual({ Memberships: memberships })
})
it('storage failure never sends and response classification defaults to unknown', () => {
  const broken = new MembershipEditor(new SidebarJournal(scope, { getItem: () => null, setItem: () => { throw new Error('storage failed') }, removeItem: () => {} }), intent)
  stage(broken)
  expect(broken.execute('PersistPending')._tag).toBe('StorageFailed')
  expect(broken.execute('Send')._tag).toBe('PersistenceRequired')
  const request = broken.state.request!
  for (const result of [{ action_id: 'other', status: 'completed' }, { action_id: 'action/2', status: 'future' }, new Error('lost')]) expect(membershipWriteResult(request, result)._tag).toBe('Unknown')
  const error = (code: ClientError['response']['code'], applied?: 'none' | 'unknown') => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/test', code, message: 'refused', retryable: false, details: applied ? { applied } : {} }, 409)
  expect(membershipWriteResult(request, error('stale-fence'))).toMatchObject({ _tag: 'Refused', refusal: 'StaleFence' })
  expect(membershipWriteResult(request, error('stale-fence', 'unknown'))._tag).toBe('Unknown')
  expect(membershipWriteResult(request, error('idempotency-conflict'))._tag).toBe('Unknown')
  expect(membershipWriteResult(request, error('idempotency-conflict', 'none'))).toMatchObject({ _tag: 'Refused', refusal: 'Conflict' })
})
it('storage recovery effects reach the host and re-enter the scoped journal executor', () => {
  const { journal, storage } = setup()
  const editor = new MembershipEditor(new SidebarJournal(scope, { ...storage, setItem: () => { throw new Error('storage failed') } }), intent)
  stage(editor)
  const failed = editor.execute('PersistPending')
  expect(failed).toMatchObject({ _tag: 'StorageFailed', effects: ['PersistPending', 'ReadBase'] })
  expect(editor.state).toMatchObject({ status: 'Refused', refusal: 'Storage' })
  const recovered = new MembershipEditor(journal, intent, editor.state)
  const results = (failed.effects ?? []).map(effect => recovered.execute(effect))
  expect(results).toEqual([{ _tag: 'Applied', effects: [] }, { _tag: 'Applied', effects: ['ReadBase'] }])
  expect(journal.restore()).toMatchObject({ _tag: 'Restored', state: { status: 'Refused', refusal: 'Storage' }, effects: ['ReadBase'] })
  expect(recovered.execute('Send')._tag).toBe('PersistenceRequired')
  const wrongScope = new MembershipEditor(new SidebarJournal({ ...scope, host: 'host/other' }, storage), intent, editor.state)
  for (const effect of failed.effects ?? []) expect(wrongScope.execute(effect)._tag).toBe('ScopeMismatch')
})
