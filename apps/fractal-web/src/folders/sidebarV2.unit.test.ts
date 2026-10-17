import { describe, expect, it } from 'vitest'
import fixture from '../../../../fixtures/clients/sidebar-v2.json'
import { sidebarV2 } from './sidebarV2.ts'
import type { SidebarNode } from './sidebarContract.ts'

// Folder-only golden contract at #3072, beb42cb8cba02993a6d3a22c683a06337ae71d95.
describe('sidebar-v2 folder-only adapter', () => {
  for (const vector of fixture.cases) it(vector.name, () => {
    const inventory: readonly string[] = vector.inventory
    const roster = vector.inventory.map(subject => ({ id: subject, subject, label: subject, host: 'fixture' }))
    const result = sidebarV2(vector.arrangement, roster)
    expect(result._tag).toBe('Built')
    if (result._tag !== 'Built') return
    const parents: Record<string, string | null> = {}
    const visit = (nodes: readonly SidebarNode[], parent: string | null) => {
      for (const node of nodes) {
        parents[node.id] = parent
        if (node._tag !== 'Agent') visit(node.children, node.id)
      }
    }
    visit(result.tree, null)
    const expected: Record<string, string | null> = {}
    for (const entry of Object.values(vector.expected.nodes)) {
      const node = entry.node
      if (node._tag === 'Subject' && !inventory.includes(node.subject!)) continue
      expected[node._tag === 'Folder' ? `folder:${node.id!}` : node.subject!] = entry.shownUnder._tag === 'Root' ? null : `folder:${entry.shownUnder.id!}`
    }
    expect(parents).toEqual(expected)
    expect(result.tree.map(node => node.id)).toEqual(vector.expected.roots.flatMap(node => 'id' in node && node.id !== undefined ? [`folder:${node.id}`] : 'subject' in node && node.subject !== undefined && inventory.includes(node.subject) ? [node.subject] : []))
  })

  it('keeps legacy behavior when no arrangement or no authoritative resolution is sent', () => {
    expect(sidebarV2(undefined, [])).toEqual({ _tag: 'Legacy' })
    const { resolved: _, ...legacy } = fixture.cases[1]!.arrangement
    expect(sidebarV2(legacy, [])).toEqual({ _tag: 'Legacy' })
  })
  it('rejects malformed data instead of drawing an invented flat projection', () => {
    expect(sidebarV2({ body: { version: 1 } }, [])._tag).toBe('Refused')
  })
  it('expands a subject to every actual seat without drawing dormant or non-agent placements', () => {
    const vector = fixture.cases[1]!
    const subject = 'agent/test/one'
    const result = sidebarV2(vector.arrangement, ['one', 'two'].map(id => ({ id, subject, label: id, host: 'fixture' })))
    expect(result._tag).toBe('Built')
    if (result._tag !== 'Built') return
    const root = result.tree[0]!
    expect(root._tag).toBe('Folder')
    if (root._tag === 'Agent') return
    const nested = root.children[0]!
    if (nested._tag === 'Agent') return
    expect(nested.children.map(node => node.id)).toEqual(['one', 'two'])
  })
})
