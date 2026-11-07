import { expect, it } from '@effect/vitest'
import type { Arrangement, OrderedMembership } from '@fractal/client'
import { Schema } from 'effect'
import vectors from '../../../../fixtures/clients/sidebar/behavior-v2.json'
import { loadSidebarBehavior } from '../../../../scripts/sidebar-behavior-fixtures.mjs'
import { planMembershipMove, projectMemberships, spreadMembershipKeys, membershipWriteRequest, type MembershipIntent } from './sidebarAdapter.ts'
import { MembershipEditor } from './edit.ts'
import { SidebarJournal } from './sidebarJournal.ts'

loadSidebarBehavior(vectors)
// Keep the production SDK-schema loading boundary lazy in this runner too.
const schema = await import('@fractal/client/schema')
const arrangement = (value: unknown): Arrangement => {
  const row = Schema.decodeUnknownSync(Schema.toEncoded(schema.Arrangement))(value)
  return { id: row.id, kind: row.kind, owner: row.owner, revision: row.revision, updated_at: row.updated_at, deleted: row.deleted, body: row.body, resolved: row.resolved ?? undefined }
}
const node = (ref: { _tag: string; id?: string; subject?: string }): MembershipIntent['node'] => ref._tag === 'Folder' ? { kind: 'folder', id: ref.id! } : { kind: 'subject', subject: ref.subject! }
for (const vector of vectors.cases) it(`v2 planner ${vector.name}`, () => {
  const base = arrangement(vector.initial.arrangement)
  for (const [eventIndex, event] of vector.events.entries()) {
    for (const [index, move] of event.moves.entries()) {
      const intent = { node: node(move.node), parent: 'id' in move.parent ? move.parent.id! : null, before: move.before === null ? null : node(move.before) }
      const plan = planMembershipMove(projectMemberships(base, vector.initial.memberships.map(row => ({ ...row, id: `${row.container}/${row.member}` }))), intent)
      expect(plan).toEqual(vector.checkpoints[eventIndex]!.moves[index])
      if (plan._tag !== 'PlannedV2') continue
      const scope = { owner: base.owner, container: base.id, endpoint: 'endpoint/test', host: 'host/test', projection: '1' }
      const editor = new MembershipEditor(new SidebarJournal(scope, { getItem: () => null, setItem: () => {}, removeItem: () => {} }), intent)
      const identity = { actor: base.owner, action_id: 'action/test', idempotency_key: 'key/test' }
      editor.reduce({ _tag: 'Plan', base: { scope, ready: true, revision: base.revision, snapshot: { id: 'snapshot/test', host_id: scope.host, store_index: 1, projection_version: '1', created_at: 'created', expires_at: 'expires' } }, ...plan.edit, folder: { ...identity, action_id: 'action/folder', idempotency_key: 'key/folder' }, membership: identity })
      editor.execute('PersistPending')
      const sent = editor.execute({ _tag: 'Send', delivery: 'Replay' })
      if (sent._tag !== 'Applied' || !sent.request) throw new Error('request not journaled')
      expect(membershipWriteRequest(sent.request).parameters.operations).toEqual(plan.edit.folders.length ? plan.edit.folders : plan.edit.memberships)
    }
  }
})
const base = arrangement(vectors.cases[0]!.initial.arrangement)
const folder = Object.keys(base.body.folders)[0]!
const member = (name: string, key: string, bucket: string | null = folder): OrderedMembership => ({ id: `${base.id}/${name}`, container: base.id, member: name, revision: 'claim/test', position: { bucket, key } })
const intent: MembershipIntent = { node: { kind: 'subject', subject: 'machine/mover' }, parent: folder, before: { kind: 'subject', subject: 'machine/b' } }
it('exact capacity includes short prefix plus descendant', () => {
  const lower = `a0V${'z'.repeat(125)}`
  const upper = `a0W${'0'.repeat(124)}2`
  expect(spreadMembershipKeys(lower, upper, 2)).toEqual(['a0W', `a0W${'0'.repeat(124)}1`])
  expect(spreadMembershipKeys(lower, upper, 3)).toBeUndefined()
  expect(spreadMembershipKeys('a0', 'a0', 1)).toBeUndefined()
})
it('ties use minimal forward one-sided rekey and Place only', () => {
  const result = planMembershipMove(projectMemberships(base, [member('machine/a', 'a0'), member('machine/b', 'a0'), member('machine/mover', 'a1', null)]), intent)
  expect(result).toEqual({ _tag: 'PlannedV2', edit: { folders: [], memberships: [
    { op: 'place', member: 'machine/mover', bucket: folder, key: 'aL' },
    { op: 'place', member: 'machine/b', bucket: folder, key: 'af' },
  ] } })
})
it('no-op, cycle, invalid ancestry and incomplete projections refuse', () => {
  const rows = [member('machine/mover', 'a0')]
  expect(planMembershipMove(projectMemberships(base, rows), { ...intent, before: null })).toEqual({ _tag: 'RefusedV2', refusal: 'NoChange' })
  expect(planMembershipMove(projectMemberships(base, rows), { node: { kind: 'folder', id: folder }, parent: folder, before: null })).toEqual({ _tag: 'RefusedV2', refusal: 'Self' })
  const child = Object.keys(base.body.folders)[1]!
  expect(planMembershipMove(projectMemberships(base, rows), { node: { kind: 'folder', id: folder }, parent: child, before: null })).toEqual({ _tag: 'RefusedV2', refusal: 'Cycle' })
  expect(planMembershipMove(projectMemberships({ ...base, resolved: undefined }, rows), intent)).toEqual({ _tag: 'RefusedV2', refusal: 'NotReady' })
  expect(planMembershipMove(projectMemberships(base, [member('machine/mover', 'a0', 'missing')]), intent)).toEqual({ _tag: 'RefusedV2', refusal: 'InvalidResolution' })
})
it('operation and encoded UTF-8 byte budgets refuse oversized edits', () => {
  const rows = Array.from({ length: 2048 }, (_, i) => member(`machine/${i.toString().padStart(4, '0')}`, 'a0'))
  const move: MembershipIntent = { ...intent, before: { kind: 'subject', subject: rows[1024]!.member } }
  expect(planMembershipMove(projectMemberships(base, [...rows, member('machine/mover', 'a1', null)]), move)).toEqual({ _tag: 'RefusedV2', refusal: 'TooManyOperations' })
  const long = `machine/${'界'.repeat(350000)}`
  expect(planMembershipMove(projectMemberships(base, [member(long, 'a0', null)]), { node: { kind: 'subject', subject: long }, parent: folder, before: null })).toEqual({ _tag: 'RefusedV2', refusal: 'TooManyBytes' })
})
