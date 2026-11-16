import { showsTerminal, tabsOf, visibleTabIds, type Tab, type TabView, type WorkspaceState } from './model.ts'
import { corePolicyOf, type DataResource, type KindPolicy } from './registry.ts'
import { parsePane, type Subject } from './subject.ts'

export const resourceKey = (resource: DataResource): string => {
  switch (resource._tag) {
    case 'Shared': return `shared:${resource.feed}`
    case 'Contributed': return `contributed:${resource.kind}:${resource.ref}`
    default: return `${resource._tag}:${resource.ref}`
  }
}

/**
 * - `active`: visible, its data held.
 * - `mounted`: hidden, DOM retained and its data held; a switch only reveals it.
 * - `warm`: hidden, no DOM, its data held; a switch mounts over current data without a loading state.
 * - `cold`: only the tab and its view state remain; a switch mounts and fetches.
 */
export type Tier = 'active' | 'mounted' | 'warm' | 'cold'

export type PolicyOf = (subject: Subject) => KindPolicy

export interface Budget {
  /** Hidden DOM subtrees. Three matches the transcript switch budget proven before tabs. */
  readonly dom: number
  /** Hidden per-subject follows, under the source's 24 retained conversation identities. */
  readonly data: number
}
export const defaultBudget: Budget = { dom: 3, data: 8 }

/** Walks hidden tabs from most recently used: each takes what still fits. Eviction is simply no longer fitting. */
export const planTiers = ({ state, budget, policyOf = corePolicyOf }: {
  readonly state: Pick<WorkspaceState, 'layout' | 'mru'> & { readonly views?: WorkspaceState['views'] }
  readonly budget: Budget
  readonly policyOf?: PolicyOf
}): ReadonlyMap<string, Tier> => {
  const tiers = new Map<string, Tier>()
  const tabs = tabsOf(state.layout)
  const byId = new Map(tabs.map(tab => [tab.id, tab]))
  const visible = visibleTabIds(state.layout)
  const order = [...new Set([...state.mru, ...tabs.map(tab => tab.id)])].filter(id => byId.has(id))
  let dom = budget.dom, data = budget.data
  for (const id of order) {
    if (visible.has(id)) {
      tiers.set(id, 'active')
      continue
    }
    const policy = policyOf(parsePane(byId.get(id)!.pane))
    const dataFits = policy.dataCost <= data
    // Evicting this surface would lose an unsent draft, so it stays whatever the budget says, and
    // still spends the budget: other tabs give way first.
    if (state.views?.[id]?.dirty === true) {
      dom -= policy.domCost
      data -= policy.dataCost
      tiers.set(id, 'mounted')
    } else if (policy.domCost > 0 && policy.domCost <= dom && dataFits) {
      dom -= policy.domCost
      data -= policy.dataCost
      tiers.set(id, 'mounted')
    } else if (policy.keepWarm && policy.dataCost > 0 && dataFits) {
      data -= policy.dataCost
      tiers.set(id, 'warm')
    } else tiers.set(id, 'cold')
  }
  return tiers
}

const terminalScreen = (agent: string): DataResource => ({ _tag: 'TerminalScreen', ref: `terminal/${agent.slice('agent/'.length)}` })

/**
 * The resources each tab holds at its tier. An agent's tab keeps its conversation whichever face it
 * shows, so the toggle is instant; the terminal's raw stream is held only while that face is on screen.
 */
export const desiredHolds = ({ tabs, tiers, views = {}, policyOf = corePolicyOf }: {
  readonly tabs: readonly Tab[]
  readonly tiers: ReadonlyMap<string, Tier>
  readonly views?: Readonly<Record<string, TabView>>
  readonly policyOf?: PolicyOf
}): ReadonlyMap<string, readonly DataResource[]> => {
  const holds = new Map<string, readonly DataResource[]>()
  for (const tab of tabs) {
    const tier = tiers.get(tab.id) ?? 'cold'
    const subject = parsePane(tab.pane)
    const policy = policyOf(subject)
    const data = tier === 'active' || tier === 'mounted' || (tier === 'warm' && policy.keepWarm) ? policy.resources(subject) : []
    const agent = subject._tag === 'Conversation' || subject._tag === 'Terminal' ? subject.agent : undefined
    holds.set(tab.id, tier === 'active' && agent !== undefined && showsTerminal(tab, views[tab.id]) ? [...data, terminalScreen(agent)] : data)
  }
  return holds
}

export interface ResourceDriver {
  /** Acquires demand and returns its release, or undefined when this source has no such demand. */
  readonly acquire: (resource: DataResource) => (() => void) | undefined
}

export interface ResourceStats {
  /** Distinct resources currently held. */
  readonly held: number
  /** Distinct per-subject sockets currently held (excludes Shared). */
  readonly follows: number
  /** Holders across all resources; exceeds `held` only when tabs share a feed. */
  readonly refs: number
  readonly acquired: number
  readonly released: number
}

/** The single owner of feed demand: panes never mount their own follows while a workspace hosts them. */
export const makeResourceManager = (driver: ResourceDriver) => {
  const entries = new Map<string, { readonly resource: DataResource; holders: ReadonlySet<string>; readonly release: (() => void) | undefined }>()
  let acquired = 0, released = 0
  const reconcile = (holds: ReadonlyMap<string, readonly DataResource[]>) => {
    const wanted = new Map<string, { readonly resource: DataResource; readonly holders: Set<string> }>()
    for (const [holder, resources] of holds) {
      for (const resource of resources) {
        const key = resourceKey(resource)
        const entry = wanted.get(key) ?? { resource, holders: new Set<string>() }
        entry.holders.add(holder)
        wanted.set(key, entry)
      }
    }
    // Acquire before release, so a resource moving between holders never drops its socket.
    for (const [key, want] of wanted) {
      const entry = entries.get(key)
      if (entry !== undefined) entry.holders = want.holders
      else {
        acquired += 1
        entries.set(key, { resource: want.resource, holders: want.holders, release: driver.acquire(want.resource) })
      }
    }
    for (const [key, entry] of entries) {
      if (wanted.has(key)) continue
      released += 1
      entry.release?.()
      entries.delete(key)
    }
  }
  return {
    reconcile,
    dispose: () => reconcile(new Map()),
    stats: (): ResourceStats => {
      const rows = [...entries.values()]
      return {
        held: rows.length,
        follows: rows.filter(row => row.resource._tag !== 'Shared').length,
        refs: rows.reduce((sum, row) => sum + row.holders.size, 0),
        acquired,
        released,
      }
    },
    keys: () => [...entries.keys()],
  }
}
export type ResourceManager = ReturnType<typeof makeResourceManager>
