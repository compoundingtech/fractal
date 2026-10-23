import type { Agent } from '../data/source.ts'
import type { SidebarNode } from '../folders/sidebarContract.ts'
import { isSidebarFiltering, matchesRow, parseSidebarQuery, sortMembers, type SidebarMatchRow } from '../shell/sidebar/filter.ts'
import type { ShowRow } from '../shell/sidebar/show.ts'
import type { SidebarFilters } from '../shell/sidebar/state.ts'
import { sidebarRow, subagentRowId } from './sidebarRow.ts'

const matchRows = ({ agents, needsYou, asking, stale }: {
  readonly agents: readonly Agent[]
  readonly needsYou: ReadonlySet<string>
  readonly asking: ReadonlySet<string>
  readonly stale: boolean
}): ReadonlyMap<string, SidebarMatchRow> =>
{
  const rows = new Map<string, SidebarMatchRow>(agents.map(agent => [agent.ref, { workspace: { title: agent.name, host: agent.host }, agent, status: sidebarRow({ agent, stale }).status, needsMe: needsYou.has(agent.ref), asking: asking.has(agent.ref) }]))
  for (const agent of agents) for (const child of agent.subagents ?? []) {
    rows.set(subagentRowId({ parent: agent.ref, id: child.id }), {
      workspace: { title: `${child.description ?? ''} ${child.id}`, host: agent.host },
      agent: undefined, status: 'unobserved', needsMe: false, asking: false,
    })
  }
  return rows
}

const childDriverMap = (agents: readonly Agent[]): ReadonlyMap<string, string> =>
  new Map(agents.flatMap(agent => (agent.subagents ?? []).map((child): [string, string] => [subagentRowId({ parent: agent.ref, id: child.id }), child.driver])))

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
  const childDrivers = childDriverMap(agents)
  const filtering = isSidebarFiltering(filters, query)
  const visit = (nodes: readonly SidebarNode[]): readonly SidebarNode[] => {
    const visible = nodes.flatMap((node): SidebarNode[] => {
      if (node._tag === 'Agent') {
        const row = rows.get(node.id)
        const children = node.children === undefined ? undefined : visit(node.children) as typeof node.children
        // Child tool is reported; parent lifecycle, activity, repository and attention are not child facts.
        const driver = childDrivers.get(node.id)
        const childToolsMatch = driver === undefined || query.facets.every(facet =>
          facet.field !== 'tool' || (facet.value !== '' && driver.toLocaleLowerCase().includes(facet.value)))
        const rowQuery = driver === undefined ? query : { ...query, facets: query.facets.filter(facet => facet.field !== 'tool') }
        if (!(row !== undefined && childToolsMatch && matchesRow({ row, filters, query: rowQuery })) && (children?.length ?? 0) === 0) return []
        const collapsed = filtering && node.children !== undefined ? false : node.collapsed
        return children === node.children && collapsed === node.collapsed ? [node] : [{ ...node, children, collapsed }]
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
