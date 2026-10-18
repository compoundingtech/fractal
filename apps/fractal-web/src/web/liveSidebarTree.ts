import type { Agent } from '../data/source.ts'
import type { SidebarNode } from '../folders/sidebarContract.ts'
import { isSidebarFiltering, matchesRow, parseSidebarQuery, sortMembers, type SidebarMatchRow } from '../shell/sidebar/filter.ts'
import type { ShowRow } from '../shell/sidebar/show.ts'
import type { SidebarFilters } from '../shell/sidebar/state.ts'
import { sidebarRow } from './sidebarRow.ts'

const matchRows = ({ agents, needsYou, asking, stale }: {
  readonly agents: readonly Agent[]
  readonly needsYou: ReadonlySet<string>
  readonly asking: ReadonlySet<string>
  readonly stale: boolean
}): ReadonlyMap<string, SidebarMatchRow> =>
  new Map(agents.map(agent => [agent.ref, { workspace: { title: agent.name, host: agent.host }, agent, status: sidebarRow({ agent, stale }).status, needsMe: needsYou.has(agent.ref), asking: asking.has(agent.ref) }]))

/** Show menu counts: each row passes every filter outside the Show menu (`is:asking` and machines). */
export const liveShowRows = ({ agents, needsYou, asking, filters, stale }: {
  readonly agents: readonly Agent[]
  readonly needsYou: ReadonlySet<string>
  readonly asking: ReadonlySet<string>
  readonly filters: SidebarFilters
  readonly stale: boolean
}): readonly ShowRow[] => {
  const others = { ...parseSidebarQuery(filters.query), asking: false, machines: [] }
  const rows = matchRows({ agents, needsYou, asking, stale })
  return agents.map(agent => ({ host: agent.host, asking: asking.has(agent.ref), others: matchesRow({ row: rows.get(agent.ref)!, filters, query: others }) }))
}

/** View-only filtering keeps the canonical arrangement and its move fences untouched. */
export const liveSidebarTree = ({ tree, agents, needsYou, asking, filters, stale }: {
  readonly tree: readonly SidebarNode[]
  readonly agents: readonly Agent[]
  readonly needsYou: ReadonlySet<string>
  readonly asking: ReadonlySet<string>
  readonly filters: SidebarFilters
  readonly stale: boolean
}): readonly SidebarNode[] => {
  const query = parseSidebarQuery(filters.query)
  const rows = matchRows({ agents, needsYou, asking, stale })
  const filtering = isSidebarFiltering(filters, query)
  const visit = (nodes: readonly SidebarNode[]): readonly SidebarNode[] => {
    const visible = nodes.flatMap((node): SidebarNode[] => {
      if (node._tag === 'Agent') {
        const row = rows.get(node.id)
        return row !== undefined && matchesRow({ row, filters, query }) ? [node] : []
      }
      const children = visit(node.children)
      const collapsed = filtering ? false : node.collapsed
      return filtering && children.length === 0 ? [] : [children === node.children && collapsed === node.collapsed ? node : { ...node, children, collapsed }]
    })
    if (filters.sort === 'manual') return visible.length === nodes.length && visible.every((node, index) => node === nodes[index]) ? nodes : visible
    const byId = new Map(visible.map(node => [node.id, node]))
    const sorted = sortMembers({ members: visible.filter(node => node._tag === 'Agent').map(node => node.id), rows, sort: filters.sort })
    return [...visible.filter(node => node._tag !== 'Agent'), ...sorted.map(id => byId.get(id)!)]
  }
  return visit(tree)
}
