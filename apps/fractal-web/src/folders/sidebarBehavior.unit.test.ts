import { describe, expect, it } from '@effect/vitest'
import type { Arrangement as ClientArrangement, OrderedMembership, Snapshot } from '@fractal/client'
import { Schema } from 'effect'
import vectors from '../../../../fixtures/clients/sidebar/behavior-v1.json'
import { loadSidebarBehavior } from '../../../../scripts/sidebar-behavior-fixtures.mjs'
import { arrangementSidebarDoc } from './client.ts'
import { MembershipEditor, optimisticArrangement } from './edit.ts'
import { canDrop, membershipWriteRequest, membershipWriteResult, operationsForMove, planMembershipMove, restageMove, sidebarProjection, sidebarTree, type MembershipIntent } from './sidebarAdapter.ts'
import type { SidebarMove, SidebarNode } from './sidebarContract.ts'
import v2Vectors from '../../../../fixtures/clients/sidebar/behavior-v2.json'
import { executeMembershipRead, MembershipPaging, type MembershipReadPort } from './memberships.ts'
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
  const plan = planMembershipMove(scaleArrangement, rows, intent)
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
      expect(editor.execute('Send')._tag).toBe('PersistenceRequired')
      editor.execute('PersistPending')
      const sent = editor.execute('Send')
      if (sent._tag !== 'Applied' || !sent.request) throw new Error('persisted request missing')
      const wire = membershipWriteRequest(sent.request)
      expect(wire.parameters.operations).toEqual([{ op: 'place', member: rows.at(-1)!.member, bucket: null, key: expect.any(String) }])
      editor.reduce(membershipWriteResult(sent.request, new Error('transport lost')))
      expect(editor.state.status).toBe('UnknownDelivery')
      editor.execute('PersistPending')
      const restored = editor.journal.restore()
      if (restored._tag !== 'Restored' || !restored.state) throw new Error('journal restore failed')
      const replay = new MembershipEditor(editor.journal, scaleIntent(rows), restored.state)
      replay.execute('PersistPending')
      const resent = replay.execute('Send')
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
    const plan = planMembershipMove(scaleArrangement, rows, intent)
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
    expect(planMembershipMove(base, rows, scaleIntent(rows))._tag).toBe('PlannedV2')
    expect(planMembershipMove(base, rows, { node: { kind: 'folder', id: ids[0]! }, parent: ids.at(-1)!, before: null })).toEqual({ _tag: 'RefusedV2', refusal: 'Cycle' })
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
      expect(planMembershipMove(scaleArrangement, test.rows, test.intent)).toEqual({ _tag: 'RefusedV2', refusal: test.refusal })
      expect(editor.state.request).toBeNull()
      expect(editor.execute('Send')._tag).toBe('PersistenceRequired')
    }
  })
  it.skip('Daemon integration fixture required: reject the 4,097th live-member edit through the real server limit (not a paging-D dependency)', () => {})
  it.skip('D #4739: local-socket initial-load p95 <=1s with sustained unrelated daemon writes', () => {})
  it.skip('PR21 host activation: browser overlay visible next animation frame; render p95 <=8ms; v2 <=2x v1', () => {})
  it.skipIf(process.env.FRACTAL_SIDEBAR_PERF !== '1')('opt-in component p95 (not browser or local-socket evidence)', async () => {
    const p95 = (values: number[]) => values.toSorted((a, b) => a - b)[Math.ceil(values.length * 0.95) - 1]!
    for (const count of [1001, 4096]) {
      const rows = scaleRows(count)
      const planning: number[] = []
      const loading: number[] = []
      for (let sample = 0; sample < 110; sample++) {
        const start = performance.now()
        const plan = planMembershipMove(scaleArrangement, rows, scaleIntent(rows))
        const planned = performance.now()
        expect(plan._tag).toBe('PlannedV2')
        await pageScale(rows)
        if (sample >= 10) { planning.push(planned - start); loading.push(performance.now() - planned) }
      }
      process.stdout.write(`sidebar-scale-measurement ${JSON.stringify({ runner: `${process.platform}/${process.arch} ${process.version}`, count, plannerP95Ms: p95(planning), scriptedPagingP95Ms: p95(loading), projectionRenderBaseline: 'blocked: v2 host unwired; not an activation pass' })}\n`)
      expect(p95(planning)).toBeLessThanOrEqual(4)
    }
  }, 60000)
})
