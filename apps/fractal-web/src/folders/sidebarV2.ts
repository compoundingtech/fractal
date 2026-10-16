import { Schema } from 'effect'
import { Arrangement as ArrangementSchema } from '@smalltalk/st3-client/schema'
import { build, nodeKey, type ProjectionError } from '@smalltalk/st3-views/sidebarView'
import type { SidebarNode } from './sidebarContract.ts'
import type { SidebarSeat } from './sidebarAdapter.ts'

const isArrangement = Schema.is(Schema.toEncoded(ArrangementSchema))
export type SidebarV2Result =
  | { readonly _tag: 'Legacy' }
  | { readonly _tag: 'Refused'; readonly error: ProjectionError | { readonly _tag: 'InvalidArrangement' } }
  | { readonly _tag: 'Built'; readonly tree: readonly SidebarNode[] }

/** sidebar-v2 is a folder-only projection contract, not a new arrangement body version. */
export const sidebarV2 = (input: unknown, roster: readonly SidebarSeat[]): SidebarV2Result => {
  if (input === undefined) return { _tag: 'Legacy' }
  if (!isArrangement(input)) return { _tag: 'Refused', error: { _tag: 'InvalidArrangement' } }
  if (input.resolved == null) return { _tag: 'Legacy' }
  const result = build(input, roster.map(seat => seat.subject))
  if (result._tag === 'Refused') return result
  const seats = new Map<string, SidebarNode[]>()
  for (const seat of roster) {
    const rows = seats.get(seat.subject) ?? []
    rows.push({ _tag: 'Agent', id: seat.id, subject: seat.subject, label: seat.label })
    seats.set(seat.subject, rows)
  }
  const visit = (refs: typeof result.tree.roots): SidebarNode[] => refs.flatMap((ref): SidebarNode[] => {
    if (ref._tag === 'Subject') return seats.get(ref.subject) ?? []
    const folder = input.body.folders[ref.id]!
    return [{ _tag: 'Folder', id: `folder:${ref.id}`, label: folder.name.value, collapsed: false,
      children: visit(result.tree.nodes.get(nodeKey(ref))!.children) }]
  })
  return { _tag: 'Built', tree: visit(result.tree.roots) }
}
