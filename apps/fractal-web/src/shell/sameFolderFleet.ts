import type { Feed, Fleet } from '../data/source.ts'
import { parseSidebarQuery } from './sidebar/filter.ts'
import type { SidebarFilters } from './sidebar/state.ts'
import { agentStatus } from './sidebar/StatusIcon.tsx'

/** Ignore presentation-only fields when deciding whether React Aria must rebuild its collection. */
// oxlint-disable-next-line overeng/named-args -- Fixed positional (feed, feed, filters) comparator ABI.
export const sameFolderFleet = (
  left: Feed<Fleet>,
  right: Feed<Fleet>,
  filters: SidebarFilters,
): boolean => {
  if (left._tag !== 'Observed' || right._tag !== 'Observed') return left._tag === right._tag
  const status =
    filters.hideEnded ||
    filters.hideRetired ||
    filters.statuses.length > 0 ||
    filters.sort === 'status'
  // Facet predicates read raw reported facts; hideEnded also reads the raw lifecycle state.
  const facets = parseSidebarQuery(filters.query).facets
  const repo = facets.some((facet) => facet.field === 'repo')
  const tool = facets.some((facet) => facet.field === 'tool')
  const state = filters.hideEnded || facets.some((facet) => facet.field === 'state')
  return (
    left.value.agents.length === right.value.agents.length &&
    left.value.agents.every((agent, index) => {
      const next = right.value.agents[index]
      return (
        next !== undefined &&
        agent.ref === next.ref &&
        (!(status || state) ||
          agentStatus({ agent, fleet: left }) === agentStatus({ agent: next, fleet: right })) &&
        (!state || (agent.state === next.state && agent.status === next.status && agent.activity === next.activity)) &&
        (!tool || agent.harness === next.harness) &&
        (!repo ||
          (agent.checkout._tag === 'Known' ? agent.checkout.value.repository : undefined) ===
            (next.checkout._tag === 'Known' ? next.checkout.value.repository : undefined)) &&
        (filters.query.trim() === '' || agent.description === next.description) &&
        (filters.sort !== 'activity' ||
          (agent.lastActivityAt._tag === next.lastActivityAt._tag &&
            (agent.lastActivityAt._tag === 'Unknown' ||
              (next.lastActivityAt._tag === 'Known' && agent.lastActivityAt.value === next.lastActivityAt.value))))
      )
    })
  )
}
