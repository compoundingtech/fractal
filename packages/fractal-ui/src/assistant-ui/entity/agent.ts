/** An agent row as an entity descriptor: the same kind + ref + title + status a hover card and a drag both carry. */
import { rowAttention, rowStatusLabel, type SidebarAgentRow } from '../sidebar/model'
import type { EntityDescriptor, EntityStatusTone } from './entity.ts'

const toneOf = (row: SidebarAgentRow): EntityStatusTone => {
  const attention = rowAttention(row)
  if (attention !== undefined) return attention
  return row.status === 'working' ? 'working' : row.status === 'ended' || row.status === 'retired' ? 'done' : 'neutral'
}

export const agentDescriptor = (row: SidebarAgentRow): EntityDescriptor => ({
  kind: 'agent', id: row.ref, title: row.title,
  ...(row.status === 'unobserved' ? {} : { status: { tone: toneOf(row), label: rowStatusLabel(row) } }),
})
