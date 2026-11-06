import { describe, expect, it } from '@effect/vitest'
import type { Arrangement as ClientArrangement } from '@fractal/client'
import { Schema } from 'effect'
import vectors from '../../../../fixtures/clients/sidebar/behavior-v1.json'
import { loadSidebarBehavior } from '../../../../scripts/sidebar-behavior-fixtures.mjs'
import { arrangementSidebarDoc } from './client.ts'
import { optimisticArrangement } from './edit.ts'
import { canDrop, operationsForMove, restageMove, sidebarProjection, sidebarTree } from './sidebarAdapter.ts'
import type { SidebarMove, SidebarNode } from './sidebarContract.ts'

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
