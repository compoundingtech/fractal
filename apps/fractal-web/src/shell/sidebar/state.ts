import * as Atom from 'effect/reactivity/Atom'

import { defaultSidebarFilters, sidebarCollapsed, sidebarFilters, sidebarTab } from '../../state/cells/lists.ts'

export { SidebarFilters, SidebarTab } from '../../state/cells/lists.ts'
import type { SidebarTab } from '../../state/cells/lists.ts'
/** Unfiltered manual-order sidebar settings for a new gateway. */
export const defaultFilters = defaultSidebarFilters
/** Gateway-scoped persisted sidebar filters, collapsed folders (and expanded agents) and tab. */
export const sidebarState = (gateway: string) => ({
  filters: sidebarFilters(gateway),
  collapsed: sidebarCollapsed(gateway),
  tab: sidebarTab(gateway),
})
/** Cache the subscribed atoms themselves: a weak-cached wrapper could be collected between renders. */
const fixtureFilters = Atom.family((_gateway: string) =>
  Atom.make(defaultFilters).pipe(Atom.keepAlive),
)
const fixtureCollapsed = Atom.family((_gateway: string) =>
  Atom.make<readonly string[]>([]).pipe(Atom.keepAlive),
)
const fixtureTab = Atom.family((_gateway: string) =>
  Atom.make<SidebarTab>('agents').pipe(Atom.keepAlive),
)
/** Story registries are isolated, including filters and collapse; no writes to live browser state. */
export const fixtureSidebarState = (gateway: string) => ({
  filters: fixtureFilters(gateway),
  collapsed: fixtureCollapsed(gateway),
  tab: fixtureTab(gateway),
})
