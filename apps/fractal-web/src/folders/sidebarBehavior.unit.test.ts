import { describe, expect, it } from '@effect/vitest'
import type { Arrangement as ClientArrangement, OrderedMembership, Snapshot } from '@fractal/client'
import { Schema } from 'effect'
import vectors from '../../../../fixtures/clients/sidebar/behavior-v1.json'
import { loadSidebarBehavior } from '../../../../scripts/sidebar-behavior-fixtures.mjs'
import { arrangementSidebarDoc } from './client.ts'
import { MembershipEditor, optimisticArrangement } from './edit.ts'
import { canDrop, MembershipProjectionCache, membershipWriteRequest, membershipWriteResult, operationsForMove, planMembershipMove, projectMemberships, restageMove, sidebarProjection, sidebarTree, type MembershipIntent } from './sidebarAdapter.ts'
import type { SidebarMove, SidebarNode } from './sidebarContract.ts'
import v2Vectors from '../../../../fixtures/clients/sidebar/behavior-v2.json'
import { executeMembershipRead, MembershipPaging, type MembershipComplete, type MembershipReadPort } from './memberships.ts'
import { SidebarJournal, type SidebarJournalRequest } from './sidebarJournal.ts'

loadSidebarBehavior(vectors)
// Preserve the web's lazy SDK-schema boundary; this runner must not add an eager schema import.
const { Arrangement } = await import('@fractal/client/schema')
const fixtureArrangement = (value: unknown): ClientArrangement => {
  // The rich schema admits nullable optional headers; this fixture contract contains only these fields.
  const decoded = Schema.decodeUnknownSync(Schema.toEncoded(Arrangement))(value)
  return {
    id: decoded.id, kind: decoded.kind, owner: decoded.owner, revision: decoded.revision,
    updated_at: decoded.updated_at, deleted: decoded.deleted, body: decoded.body, resolved: decoded.resolved ?? undefined,
  }
}
type Ref = { _tag: string; id?: string; subject?: string }
const id = (ref: Ref): string => ref.id ?? ref.subject ?? ''
const rowKey = (ref: Ref): string => `${ref._tag === 'Folder' ? 'folder' : 'subject'}:${id(ref)}`
const folder = (suffix: string): string => `folder:00000000-0000-7000-8000-0000000000${suffix}`
const nested = 'nested-and-dormant-non-agent-placements'
const lifted = 'multi-tombstone-lift-and-unknown-root'
const ties = 'separate-domains-and-unicode-scalar-ties'

// Native v1 differences are pinned, not silently filtered out of the shared goldens.
const divergences: Record<string, { reason: string; native: unknown; golden: unknown }> = {
  [`${nested}/tree`]: { reason: 'Stored dormant and non-agent placements have no catalog seats.', native: [
    [folder('0a'), [[folder('0b'), [['subject:agent/test/one', []]]]]], ['subject:agent/test/unplaced', []],
  ], golden: [
    [folder('0a'), [[folder('0b'), [['subject:agent/test/one', []], ['subject:agent/test/dormant', []]]], ['subject:machine/retained', []]]], ['subject:agent/test/unplaced', []],
  ] },
  [`${lifted}/tree`]: { reason: 'An empty catalog renders folders but no stored member seats.', native: [
    [folder('0a'), [[folder('0b'), []]]],
  ], golden: [[folder('0a'), [[folder('0b'), []], ['subject:agent/test/lifted', []]]], ['subject:agent/test/unknown', []]] },
  [`${ties}/tree`]: { reason: 'Unfiled web seats use automatic identity order, not placement order.', native: [
    [folder('10'), []], [folder('11'), []], ['subject:agent/test/unplaced', []], ['subject:agent/test/\ue000', []], ['subject:agent/test/😀', []],
  ], golden: [
    [folder('10'), []], [folder('11'), []], ['subject:agent/test/\ue000', []], ['subject:agent/test/😀', []], ['subject:agent/test/unplaced', []],
  ] },
  [`${nested}/move/2`]: { reason: 'Subject parents have no native drop target; Into rejects the subject as a folder.', native: { _tag: 'Refused', sentence: 'This folder is no longer available.' }, golden: { _tag: 'Refused', sentence: 'Subject parents are unsupported.' } },
  [`${nested}/move/6`]: { reason: 'A dormant anchor has no native seat, rather than yielding golden NoChange.', native: { _tag: 'Refused', sentence: 'This item is no longer available.' }, golden: { _tag: 'Satisfied' } },
  [`${nested}/move/7`]: { reason: 'A dormant mover has no native seat, rather than yielding golden Planned.', native: { _tag: 'Refused', sentence: 'This item is no longer available.' }, golden: { _tag: 'Operations' } },
  [`${lifted}/move/0`]: { reason: 'The lifted mover has no catalog seat, rather than yielding golden Planned.', native: { _tag: 'Refused', sentence: 'This item is no longer available.' }, golden: { _tag: 'Operations' } },
  [`${lifted}/move/1`]: { reason: 'Missing native mover takes precedence over the deleted parent refusal.', native: { _tag: 'Refused', sentence: 'This item is no longer available.' }, golden: { _tag: 'Refused', sentence: 'This folder is no longer available.' } },
}
const seen = new Set<string>()
const matches = (key: string, actual: unknown, golden: unknown): void => {
  const difference = divergences[key]
  if (difference === undefined) expect(actual, key).toEqual(golden)
  else {
    seen.add(key)
    expect(actual, difference.reason).toEqual(difference.native)
    expect(golden, `${key}: golden contract changed`).toEqual(difference.golden)
    expect(actual, `${key}: remove resolved divergence`).not.toEqual(golden)
  }
}
const nativeRows = (rows: readonly SidebarNode[]): unknown[] => rows.flatMap((row): unknown[] =>
  row._tag === 'Group' ? nativeRows(row.children)
    : [[`${row._tag === 'Folder' ? 'folder' : 'subject'}:${row._tag === 'Folder' ? row.id : row.subject}`, nativeRows(row.children ?? [])]],
)
const sentences: Record<string, string> = {
  Self: 'A folder cannot go inside itself or its subfolders.',
  Cycle: 'A folder cannot go inside itself or its subfolders.',
  ParentMissing: 'This folder is no longer available.',
  NodeMissing: 'This item is no longer available.',
  AnchorMoved: 'Folders and agents have separate ordering.',
  ParentNotAllowed: 'Subject parents are unsupported.',
}
const nativeMove = (intent: { node: Ref; parent: Ref; before: Ref | null }): SidebarMove => ({
  items: [id(intent.node)],
  target: intent.before !== null ? { _tag: 'Before', sibling: id(intent.before) }
    : intent.parent._tag !== 'Root' ? { _tag: 'Into', folder: id(intent.parent) }
      : intent.node._tag === 'Folder' ? { _tag: 'Root', index: 0 } : { _tag: 'Unfiled' },
})

describe('shared sidebar v1 through the production web adapter/editor', () => {
  it('rejects schema-invalid fixtures before replay', () => {
    expect(() => loadSidebarBehavior({ ...vectors, schemaVersion: 2 })).toThrow()
    expect(() => loadSidebarBehavior({ ...vectors, unknown: true })).toThrow()
  })
  for (const fixture of vectors.cases) {
    it(fixture.name, () => {
      const arrangement = fixtureArrangement(fixture.initial.arrangement)
      // Only catalog identities become seats: dormant placements are deliberately not synthesized.
      const roster = fixture.catalog.map(subject => ({ id: subject, subject, host: '', label: subject }))
      const doc = { ...arrangementSidebarDoc(arrangement), roster }
      const checkpoint = fixture.checkpoints[0]!
      const nodes = checkpoint.tree.nodes
      const goldenRows = (refs: readonly Ref[]): unknown[] => refs.map(ref => {
        const node = Object.entries(nodes).find(([key]) => key === rowKey(ref))?.[1]
        expect(node, rowKey(ref)).toBeDefined()
        return [rowKey(ref), goldenRows(node?.children ?? [])]
      })
      matches(`${fixture.name}/tree`, nativeRows(sidebarTree(sidebarProjection(doc), roster, new Map())), goldenRows(checkpoint.tree.roots))
      for (const [index, intent] of fixture.events[0]!.moves.entries()) {
        const expected = checkpoint.moves[index]!
        const golden = expected._tag === 'Planned' ? { _tag: 'Operations' }
          : expected.refusal?._tag === 'NoChange' ? { _tag: 'Satisfied' }
            : { _tag: 'Refused', sentence: sentences[expected.refusal?._tag ?? ''] }
        const move = nativeMove(intent)
        const verdict = restageMove(doc, move)
        matches(`${fixture.name}/move/${index}`, verdict._tag === 'Operations' ? { _tag: verdict._tag } : verdict, golden)
        expect(canDrop(doc, move)).toEqual(verdict._tag === 'Refused' ? { refused: verdict.sentence } : { ok: true })
        const operations = operationsForMove(doc, move)
        expect(operations).toEqual(verdict._tag === 'Operations' ? verdict.operations : [])
        expect(optimisticArrangement(arrangement, operations).body).toEqual(arrangement.body)
      }
      expect(checkpoint.effects).toEqual([])
      expect(checkpoint.wireActions).toEqual([])
    })
  }
  it('has no unused native divergence entries', () => {
    expect([...seen].sort()).toEqual(Object.keys(divergences).sort())
  })
  it('applies a catalog-seat move through production optimistic editing', () => {
    const fixture = vectors.cases.find(value => value.name === nested)!
    const arrangement = fixtureArrangement(fixture.initial.arrangement)
    const roster = fixture.catalog.map(subject => ({ id: subject, subject, host: '', label: subject }))
    const doc = { ...arrangementSidebarDoc(arrangement), roster }
    const parent = id(fixture.events[0]!.moves[0]!.node)
    const operations = operationsForMove(doc, { items: ['agent/test/one'], target: { _tag: 'Into', folder: parent } })
    expect(operations).toEqual([{ op: 'subject.place', subject: 'agent/test/one', folder: parent, key: 'a2' }])
    const edited = optimisticArrangement(arrangement, operations)
    expect(edited.body.version === 1 && edited.body.placements['agent/test/one']?.value).toEqual({ folder: parent, key: 'a2' })
    expect(sidebarProjection({ ...arrangementSidebarDoc(edited), roster }).folders[0]?.members).toEqual(['agent/test/one'])
  })
})

// Pre-activation component harness, not browser/socket activation evidence.
const scaleArrangement = fixtureArrangement(v2Vectors.cases[0]!.initial.arrangement)
const scaleFolder = Object.keys(scaleArrangement.body.folders)[0]!
const scaleScope = { owner: scaleArrangement.owner, container: scaleArrangement.id, endpoint: 'endpoint/test', host: 'host/test', projection: 'client-projection.v0' }
const scaleSnapshot = (index: number): Snapshot & { readonly expires_at: string } => ({ id: `snapshot/${index}`, host_id: scaleScope.host, store_index: index, projection_version: 'client-projection.v0', created_at: 'created', expires_at: 'expires' })
const scaleRows = (count: number): OrderedMembership[] => Array.from({ length: count }, (_, i) => ({
  id: `membership/${i}`, container: scaleArrangement.id, member: `agent/test/${i.toString().padStart(4, '0')}`,
  revision: 'claim/test', position: { bucket: scaleFolder, key: 'a0' },
}))
const scaleIntent = (rows: readonly OrderedMembership[]): MembershipIntent => ({
  node: { kind: 'subject', subject: rows.at(-1)!.member }, parent: null, before: null,
})
const scaleEditor = (intent: MembershipIntent) => {
  const values = new Map<string, string>()
  const journal = new SidebarJournal(scaleScope, {
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value) }, removeItem: key => { values.delete(key) },
  })
  return new MembershipEditor(journal, intent)
}
const scaleIdentity = (name: string) => ({ actor: scaleScope.owner, action_id: `action/${name}`, idempotency_key: `key/${name}` })
const scaleBase: SidebarJournalRequest['fence'] = { scope: scaleScope, revision: scaleArrangement.revision, ready: true, snapshot: scaleSnapshot(1) }
const stageScale = (rows: readonly OrderedMembership[]) => {
  const intent = scaleIntent(rows)
  const plan = planMembershipMove(projectMemberships(scaleArrangement, rows), intent)
  expect(plan._tag).toBe('PlannedV2')
  if (plan._tag !== 'PlannedV2') throw new Error('scale plan refused')
  const editor = scaleEditor(intent)
  expect(editor.reduce({ _tag: 'Plan', base: scaleBase, ...plan.edit, folder: scaleIdentity('folder'), membership: scaleIdentity('member') })).toEqual(['PersistPending'])
  return editor
}
const pageScale = async (rows: readonly OrderedMembership[], unrelatedWrites = false) => {
  const paging = new MembershipPaging()
  const frontier = { epoch: 'epoch/test', changed_index: 1, live_count: rows.length }
  let effects = paging.reduce({ _tag: 'Refresh', scope: scaleScope, generation: 1 })
  let calls = 0
  const port: MembershipReadPort = {
    subscribe: async () => {},
    read: async (_token, cursor) => {
      const start = cursor === undefined ? 0 : Number(cursor)
      const next = Math.min(start + 128, rows.length)
      calls++
      return { snapshot: scaleSnapshot(calls), value: {
        collection: 'ordered-memberships', kind: 'page', filters: { person: scaleScope.owner, subject: scaleScope.container },
        items: rows.slice(start, next), page: { limit: 128, has_more: next < rows.length, next_cursor: next < rows.length ? String(next) : null },
        membership: { container: scaleScope.container, ...frontier },
      } }
    },
  }
  while (effects.length && effects[0]!._tag !== 'PersistBase') {
    expect(effects).toHaveLength(1)
    const effect = effects[0]!
    if (unrelatedWrites) for (let write = 0; write < 100; write++)
      expect(paging.reduce({ _tag: 'Metadata', token: effect.token, frontier })).toEqual([])
    effects = paging.reduce(await executeMembershipRead(effect, port))
    if (paging.status === 'Paging') expect(paging.complete).toBeUndefined()
  }
  expect(paging.status).toBe('Ready')
  expect(paging.stale).toBe(false)
  expect(paging.complete?.members).toEqual(rows)
  expect(calls).toBe(Math.ceil(rows.length / 128))
  return paging
}

describe('unwired web v2 scale/fault acceptance', () => {
  for (const count of [1001, 4096]) {
    it(`${count} live agents: full paging adapter, pending edit and exact unknown-delivery replay`, async () => {
      const rows = scaleRows(count)
      await pageScale(rows, true)
      const editor = stageScale(rows)
      // Pending intent/request is available synchronously, before persistence or network work.
      expect(editor.state).toMatchObject({ status: 'AwaitingPersistence', intent: scaleIntent(rows) })
      expect(editor.execute({ _tag: 'Send', delivery: 'Replay' })._tag).toBe('PersistenceRequired')
      editor.execute('PersistPending')
      const sent = editor.execute({ _tag: 'Send', delivery: 'Replay' })
      if (sent._tag !== 'Applied' || !sent.request) throw new Error('persisted request missing')
      const wire = membershipWriteRequest(sent.request)
      expect(wire.parameters.operations).toEqual([{ op: 'place', member: rows.at(-1)!.member, bucket: null, key: expect.any(String) }])
      editor.reduce(membershipWriteResult(sent.request, 'First', new Error('transport lost')))
      expect(editor.state.status).toBe('UnknownDelivery')
      editor.execute('PersistPending')
      const restored = editor.journal.restore()
      if (restored._tag !== 'Restored' || !restored.state) throw new Error('journal restore failed')
      const replay = new MembershipEditor(editor.journal, scaleIntent(rows), restored.state)
      replay.execute('PersistPending')
      const resent = replay.execute({ _tag: 'Send', delivery: 'Replay' })
      if (resent._tag !== 'Applied' || !resent.request) throw new Error('exact replay missing')
      expect(membershipWriteRequest(resent.request)).toEqual(wire)
      replay.reduce({ _tag: 'Refused', identity: scaleIdentity('member'), refusal: 'Unauthorized' })
      expect(replay.state).toMatchObject({ status: 'Refused', refusal: 'Unauthorized' })
    })
  }
  it('distinguishes complete-empty from unavailable and bounds invalidated scale attempts', () => {
    const paging = new MembershipPaging()
    expect(paging.complete).toBeUndefined()
    let effects = paging.reduce({ _tag: 'Refresh', scope: scaleScope, generation: 1 })
    for (let attempt = 0; attempt < 3; attempt++) {
      const effect = effects[0]!
      if (effect._tag === 'PersistBase') throw new Error('unexpected complete view')
      effects = paging.reduce({ _tag: 'Expired', token: effect.token })
    }
    expect(paging.status).toBe('Exhausted')
    expect(effects).toEqual([])
    const [fresh] = paging.reduce({ _tag: 'Refresh', scope: scaleScope, generation: 2 })
    if (!fresh || fresh._tag === 'PersistBase') throw new Error('missing refresh')
    paging.reduce({ _tag: 'Page', token: fresh.token, page: { frontier: { epoch: 'epoch/test', changed_index: 1, live_count: 0 }, snapshot: scaleSnapshot(1), members: [], has_more: false } })
    expect(paging.status).toBe('Ready')
    expect(paging.complete?.members).toEqual([])
  })
  it('plans a minimal tied-key rekey at the live-member bound without truncation', () => {
    const rows = scaleRows(4096)
    const intent: MembershipIntent = { node: { kind: 'subject', subject: rows.at(-1)!.member }, parent: scaleFolder, before: { kind: 'subject', subject: rows.at(-2)!.member } }
    const plan = planMembershipMove(projectMemberships(scaleArrangement, rows), intent)
    expect(plan._tag).toBe('PlannedV2')
    if (plan._tag !== 'PlannedV2') throw new Error('bounded rekey refused')
    expect(plan.edit.memberships.map(row => row.member)).toEqual([rows.at(-1)!.member, rows.at(-2)!.member])
    expect(plan.edit.memberships[0]!.key < plan.edit.memberships[1]!.key).toBe(true)
  })
  it('retains the last complete scale view across transport loss, refusal and obsolete attempts', async () => {
    const paging = await pageScale(scaleRows(4096))
    const complete = paging.complete
    const [effect] = paging.reduce({ _tag: 'Refresh', scope: scaleScope, generation: 2 })
    if (!effect || effect._tag === 'PersistBase') throw new Error('missing read')
    paging.reduce({ _tag: 'TransportLost', token: effect.token })
    expect(paging.status).toBe('Disconnected')
    expect(paging.stale).toBe(true)
    expect(paging.complete).toBe(complete)
    paging.reduce({ _tag: 'Refresh', scope: scaleScope, generation: 3 })
    expect(paging.reduce({ _tag: 'Refused', token: effect.token })).toEqual([])
    expect(paging.status).toBe('Paging')
    expect(paging.complete).toBe(complete)
  })
  it('admits a 1,024-folder ancestry without recursive planner overflow', () => {
    const template = Object.values(scaleArrangement.body.folders)[0]!
    const ids = Array.from({ length: 1024 }, (_, i) => `folder/${i}`)
    const folders = Object.fromEntries(ids.map((id, i) => [id, { ...template, position: { ...template.position, value: { parent: ids[i - 1] ?? null, key: 'a0' } } }]))
    const parents = Object.fromEntries(ids.map((id, i) => [id, ids[i - 1] ?? null]))
    const base = { ...scaleArrangement, body: { ...scaleArrangement.body, version: 2 as const, folders }, resolved: { ...scaleArrangement.resolved!, parents } }
    const rows = scaleRows(1001).map(row => ({ ...row, position: { bucket: ids.at(-1)!, key: 'a0' } }))
    expect(planMembershipMove(projectMemberships(base, rows), scaleIntent(rows))._tag).toBe('PlannedV2')
    expect(planMembershipMove(projectMemberships(base, rows), { node: { kind: 'folder', id: ids[0]! }, parent: ids.at(-1)!, before: null })).toEqual({ _tag: 'RefusedV2', refusal: 'Cycle' })
  })
  it('rejects a 1,025-operation atomic tied-key rekey and >1 MiB before dispatch', () => {
    const tied = scaleRows(2048)
    const mover = { ...scaleRows(1)[0]!, member: 'machine/mover', position: { bucket: null, key: 'a1' } }
    const oversized = { ...mover, member: `machine/${'界'.repeat(350000)}` }
    const cases = [
      { rows: [...tied, mover], intent: { node: { kind: 'subject' as const, subject: mover.member }, parent: scaleFolder, before: { kind: 'subject' as const, subject: tied[1024]!.member } }, refusal: 'TooManyOperations' },
      { rows: [oversized], intent: { node: { kind: 'subject' as const, subject: oversized.member }, parent: scaleFolder, before: null }, refusal: 'TooManyBytes' },
    ]
    for (const test of cases) {
      const editor = scaleEditor(test.intent)
      expect(planMembershipMove(projectMemberships(scaleArrangement, test.rows), test.intent)).toEqual({ _tag: 'RefusedV2', refusal: test.refusal })
      expect(editor.state.request).toBeNull()
      expect(editor.execute({ _tag: 'Send', delivery: 'Replay' })._tag).toBe('PersistenceRequired')
    }
  })
  it('reuses immutable indexes across snapshot writes and invalidates each relevant revision and scope', () => {
    const cache = new MembershipProjectionCache()
    const members = scaleRows(1001)
    const complete: MembershipComplete = { scope: scaleScope, frontier: { epoch: 'epoch/1', changed_index: 1, live_count: members.length }, members, snapshots: [scaleSnapshot(1)] }
    const first = cache.get(scaleArrangement, complete, 'catalog/1', 'presentation/1')
    const intent = scaleIntent(members)
    const before = planMembershipMove(first, intent)
    expect(cache.get({ ...scaleArrangement }, { ...complete, snapshots: [scaleSnapshot(9999)] }, 'catalog/1', 'presentation/1')).toBe(first)
    expect(planMembershipMove(first, intent)).toEqual(before)
    if (first._tag !== 'ProjectedV2') throw new Error('projection refused')
    expect(first.nodes.size).toBe(members.length + Object.values(scaleArrangement.body.folders).filter(folder => folder.tombstone === null).length)
    for (const changed of [
      { arrangement: { ...scaleArrangement, revision: 'claim/changed' }, complete, catalog: 'catalog/1', presentation: 'presentation/1' },
      { arrangement: scaleArrangement, complete, catalog: 'catalog/2', presentation: 'presentation/1' },
      { arrangement: scaleArrangement, complete, catalog: 'catalog/1', presentation: 'presentation/2' },
      ...[{ epoch: 'epoch/2' }, { changed_index: 2 }, { live_count: members.length - 1 }].map(frontier => ({ arrangement: scaleArrangement, complete: { ...complete, frontier: { ...complete.frontier, ...frontier } }, catalog: 'catalog/1', presentation: 'presentation/1' })),
      ...[{ host: 'host/other' }, { endpoint: 'endpoint/other' }, { projection: 'projection/other' }].map(scope => ({ arrangement: scaleArrangement, complete: { ...complete, scope: { ...scaleScope, ...scope } }, catalog: 'catalog/1', presentation: 'presentation/1' })),
    ]) {
      const current = cache.get(scaleArrangement, complete, 'catalog/1', 'presentation/1')
      expect(cache.get(changed.arrangement, changed.complete, changed.catalog, changed.presentation)).not.toBe(current)
    }
    for (const scope of [{ owner: 'person/other' }, { container: 'arrangement/other' }]) {
      expect(cache.get(scaleArrangement, { ...complete, scope: { ...scaleScope, ...scope } }, 'catalog/1', 'presentation/1')).toEqual({ _tag: 'RefusedV2', refusal: 'InvalidResolution' })
    }
    expect(cache.get(scaleArrangement, complete, 'catalog/1', 'presentation/1')).not.toBe(first)
    expect(planMembershipMove(first, intent)).toEqual(before)
    expect(cache.get({ ...scaleArrangement, resolved: undefined }, complete, 'catalog/1', 'presentation/1')).toEqual({ _tag: 'RefusedV2', refusal: 'NotReady' })
    expect(cache.get(scaleArrangement, complete, 'catalog/1', 'presentation/1')._tag).toBe('ProjectedV2')
    const edited = { ...complete, frontier: { ...complete.frontier, changed_index: 2 }, members: members.map(row => row.member === members.at(-1)!.member ? { ...row, position: { bucket: null, key: 'a0' } } : row) }
    expect(planMembershipMove(cache.get(scaleArrangement, edited, 'catalog/1', 'presentation/1'), intent)).toEqual({ _tag: 'RefusedV2', refusal: 'NoChange' })
  })
  it('copies historical cache keys even when host inputs mutate in place', () => {
    const cache = new MembershipProjectionCache()
    const arrangement = { ...scaleArrangement }
    const complete = { scope: { ...scaleScope }, frontier: { epoch: 'epoch/1', changed_index: 1, live_count: 1001 }, members: scaleRows(1001), snapshots: [scaleSnapshot(1)] }
    let previous = cache.get(arrangement, complete, 'catalog/1', 'presentation/1')
    for (const change of [
      () => { arrangement.revision = 'claim/changed' },
      () => { complete.frontier.epoch = 'epoch/2' },
      () => { complete.frontier.changed_index++ },
      () => { complete.frontier.live_count--; complete.members.pop() },
      () => { complete.scope.host = 'host/other' },
      () => { complete.scope.endpoint = 'endpoint/other' },
      () => { complete.scope.projection = 'projection/other' },
    ]) {
      change()
      const current = cache.get(arrangement, complete, 'catalog/1', 'presentation/1')
      expect(current._tag).toBe('ProjectedV2')
      expect(current).not.toBe(previous)
      previous = current
    }
    complete.members[0] = { ...complete.members[0]!, position: { bucket: null, key: 'a0' } }
    complete.frontier.changed_index++
    const intent: MembershipIntent = { node: { kind: 'subject', subject: complete.members[0]!.member }, parent: null, before: null }
    expect(planMembershipMove(cache.get(arrangement, complete, 'catalog/1', 'presentation/1'), intent)).toEqual({ _tag: 'RefusedV2', refusal: 'NoChange' })
  })
  it('invalidates complete -> incomplete -> complete resolution at the same arrangement revision', () => {
    const cache = new MembershipProjectionCache()
    const parents = { ...scaleArrangement.resolved!.parents }
    const parent = parents[scaleFolder]!
    delete parents[scaleFolder]
    const arrangement = { ...scaleArrangement, resolved: { ...scaleArrangement.resolved!, parents } }
    const complete: MembershipComplete = { scope: scaleScope, frontier: { epoch: 'epoch/1', changed_index: 1, live_count: 1001 }, members: scaleRows(1001), snapshots: [scaleSnapshot(1)] }
    expect(cache.get(arrangement, complete, 'catalog/1', 'presentation/1')).toEqual({ _tag: 'RefusedV2', refusal: 'NotReady' })
    parents[scaleFolder] = parent
    const model = cache.get(arrangement, complete, 'catalog/1', 'presentation/1')
    expect(model._tag).toBe('ProjectedV2')
    expect(planMembershipMove(model, scaleIntent(complete.members))._tag).toBe('PlannedV2')
    expect(cache.get(arrangement, complete, 'catalog/1', 'presentation/1')).toBe(model)
    delete parents[scaleFolder]
    const incomplete = cache.get(arrangement, complete, 'catalog/1', 'presentation/1')
    expect(incomplete).toEqual({ _tag: 'RefusedV2', refusal: 'NotReady' })
    expect(planMembershipMove(incomplete, scaleIntent(complete.members))).toEqual(incomplete)
    parents[scaleFolder] = parent
    const recovered = cache.get(arrangement, complete, 'catalog/1', 'presentation/1')
    expect(recovered._tag).toBe('ProjectedV2')
    expect(recovered).not.toBe(model)
    parents[scaleFolder] = scaleFolder
    expect(cache.get(arrangement, complete, 'catalog/1', 'presentation/1')).toEqual({ _tag: 'RefusedV2', refusal: 'InvalidResolution' })
  })
  for (const fault of ['duplicate', 'wrong-container'] as const) it(`validates ${fault} members before missing resolved parents, cold and after invalidation`, () => {
    const cache = new MembershipProjectionCache()
    const members = scaleRows(1001)
    const complete: MembershipComplete = { scope: scaleScope, frontier: { epoch: 'epoch/1', changed_index: 1, live_count: members.length }, members, snapshots: [scaleSnapshot(1)] }
    expect(cache.get(scaleArrangement, complete, 'catalog/1', 'presentation/1')._tag).toBe('ProjectedV2')
    const parents = { ...scaleArrangement.resolved!.parents }
    delete parents[scaleFolder]
    const incomplete = { ...scaleArrangement, resolved: { ...scaleArrangement.resolved!, parents } }
    const invalid = { ...complete, members: fault === 'duplicate' ? [...members, members[0]!] : [{ ...members[0]!, container: 'arrangement/other' }, ...members.slice(1)] }
    const expected = { _tag: 'RefusedV2', refusal: 'InvalidResolution' }
    expect(projectMemberships(incomplete, invalid.members)).toEqual(expected)
    expect(cache.get(incomplete, invalid, 'catalog/1', 'presentation/1')).toEqual(expected)
    expect(new MembershipProjectionCache().get(incomplete, invalid, 'catalog/1', 'presentation/1')).toEqual(expected)
    expect(planMembershipMove(projectMemberships(incomplete, invalid.members), scaleIntent(members))).toEqual(expected)
    expect(cache.get(incomplete, complete, 'catalog/1', 'presentation/1')).toEqual({ _tag: 'RefusedV2', refusal: 'NotReady' })
  })
  it.skip('Daemon integration fixture required: reject the 4,097th live-member edit through the real server limit (not a paging-D dependency)', () => {})
  it.skip('D #4739: local-socket initial-load p95 <=1s with sustained unrelated daemon writes', () => {})
  it.skip('PR21 host activation: browser overlay visible next animation frame; render p95 <=8ms; v2 <=2x v1', () => {})
  it.skipIf(process.env.FRACTAL_SIDEBAR_PERF !== '1')('opt-in cached per-keystroke p95 and equivalent v1 component baseline', () => {
    const percentile = (values: number[], fraction: number) => values.toSorted((a, b) => a - b)[Math.ceil(values.length * fraction) - 1]!
    for (const count of [1001, 4096]) {
      const rows = scaleRows(count)
      const complete: MembershipComplete = { scope: scaleScope, frontier: { epoch: 'epoch/1', changed_index: 1, live_count: count }, members: rows, snapshots: [scaleSnapshot(1)] }
      const cache = new MembershipProjectionCache()
      const intent = scaleIntent(rows)
      const v1: ClientArrangement = { ...scaleArrangement, body: { ...scaleArrangement.body, version: 1, placements: Object.fromEntries(rows.map(row => [row.member, { revision: row.revision, value: { folder: row.position.bucket, key: row.position.key } }])) } }
      const doc = { ...arrangementSidebarDoc(v1), roster: rows.map(row => ({ id: row.member, subject: row.member, host: '', label: row.member })) }
      const move: SidebarMove = { items: [rows.at(-1)!.member], target: { _tag: 'Unfiled' } }
      const cold: number[] = [], warm: number[] = [], baseline: number[] = []
      for (let sample = 0; sample < 110; sample++) {
        const coldStart = performance.now()
        const projected = projectMemberships(scaleArrangement, rows)
        const projectedAt = performance.now()
        const editor = scaleEditor(intent)
        const start = performance.now()
        const model = cache.get(scaleArrangement, complete, 'catalog/1', 'presentation/1')
        const plan = planMembershipMove(model, intent)
        if (plan._tag !== 'PlannedV2') throw new Error('cached input refused')
        editor.reduce({ _tag: 'Plan', base: scaleBase, ...plan.edit, folder: scaleIdentity('folder'), membership: scaleIdentity('member') })
        const pendingAt = performance.now()
        const v1Start = performance.now()
        const projection = sidebarProjection(doc)
        const verdict = restageMove(doc, move)
        if (verdict._tag !== 'Operations') throw new Error('v1 input refused')
        const pending = optimisticArrangement(v1, verdict.operations)
        const v1End = performance.now()
        expect(projected._tag).toBe('ProjectedV2')
        expect(projection.folders.length).toBeGreaterThan(0)
        expect(pending.body.version).toBe(1)
        expect(editor.state.status).toBe('AwaitingPersistence')
        if (sample >= 10) { cold.push(projectedAt - coldStart); warm.push(pendingAt - start); baseline.push(v1End - v1Start) }
      }
      const warmP95Ms = percentile(warm, 0.95), v1P95Ms = percentile(baseline, 0.95)
      process.stdout.write(`sidebar-scale-measurement ${JSON.stringify({ runner: `${process.platform}/${process.arch} ${process.version}`, count, coldProjectionP50Ms: percentile(cold, 0.5), coldProjectionP95Ms: percentile(cold, 0.95), warmP95Ms, v1P95Ms, ratio: warmP95Ms / v1P95Ms, scope: 'projection+planning+pending state; not browser render or local socket' })}\n`)
      expect(warmP95Ms).toBeLessThanOrEqual(4)
      expect(warmP95Ms).toBeLessThanOrEqual(v1P95Ms * 2)
    }
  }, 60000)
})
