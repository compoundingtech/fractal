import { Schema } from 'effect'

import * as StateCell from '../cell/StateCell.ts'
import { legacyEntry } from '../legacy.ts'

const fromLegacy = <A, I>(schema: Schema.Codec<A, I>, key: string, version = 1): A | undefined => {
  const value = legacyEntry(key, version)
  if (value === undefined) return undefined
  const decoded = Schema.decodeUnknownOption(schema)(value)
  return decoded._tag === 'Some' ? decoded.value : undefined
}
const storedString = (key: string): string | undefined => {
  if (typeof window === 'undefined') return undefined
  try { return window.localStorage.getItem(key) ?? undefined } catch { return undefined }
}

/** Sidebar search, visibility, status and ordering; Show filters live in the query. */
export const SidebarFilters = Schema.Struct({
  query: Schema.String,
  hideEnded: Schema.Boolean,
  hideRetired: Schema.Boolean,
  statuses: Schema.Array(Schema.String),
  sort: Schema.Literals(['manual', 'status', 'activity', 'name', 'host']),
})
export type SidebarFilters = typeof SidebarFilters.Type
export const defaultSidebarFilters: SidebarFilters = { query: '', hideEnded: false, hideRetired: false, statuses: [], sort: 'manual' }
export const SidebarTab = Schema.Literals(['agents', 'alerts', 'missions', 'fleet'])
export type SidebarTab = typeof SidebarTab.Type
const FolderIds = Schema.Array(Schema.String)

/** Per gateway: the sidebar's filters. Folder trees differ per gateway, so none of this is global. */
export const sidebarFilters = StateCell.make({
  scope: 'gateway', key: 'sidebar.filters', version: 1, schema: SidebarFilters, default: defaultSidebarFilters,
  seed: (gateway) => fromLegacy(SidebarFilters, `${gateway}:sidebar.filters`),
})
/** Per gateway: collapsed folders, plus `wf/expanded/<agent>` markers for parent rows the reader opened. */
export const sidebarCollapsed = StateCell.make({
  scope: 'gateway', key: 'sidebar.collapsedFolders', version: 1, schema: FolderIds, default: [] as readonly string[],
  seed: (gateway) => fromLegacy(FolderIds, `${gateway}:sidebar.collapsedFolders`),
})
export const sidebarTab = StateCell.make({
  scope: 'gateway', key: 'sidebar.tab', version: 1, schema: SidebarTab, default: 'agents' as SidebarTab,
  seed: (gateway) => fromLegacy(SidebarTab, `${gateway}:sidebar.tab`),
})

/** Browser-local machine groups: `null` = no readable choice; `[]` = explicitly cleared. AND between tokens, OR within. */
export const MachineGroups = Schema.NullOr(Schema.Array(Schema.Array(Schema.String.check(Schema.isPattern(/^[^\s,A-Z]+$/u))).check(Schema.isMinLength(1))))
export const sidebarMachines = StateCell.make({
  scope: 'global', key: 'sidebar.show.machines', version: 2, schema: MachineGroups, default: null,
  seed: () => fromLegacy(MachineGroups, 'sidebar.show.machines@2', 2) ?? undefined,
})

export const UsageSection = Schema.Struct({ open: Schema.Boolean, height: Schema.Finite })
export type UsageSection = typeof UsageSection.Type
/** The sidebar's Usage section, one setting shared by every window. */
export const usageSection = StateCell.make({
  scope: 'global', key: 'sidebar.usageSection', version: 1, schema: UsageSection, default: { open: false, height: 0 },
  seed: () => {
    const open = storedString('wf.sidebar.usageSection.open')
    const height = Number(storedString('wf.sidebar.usageSection.height') ?? Number.NaN)
    return open === undefined && !Number.isFinite(height) ? undefined : { open: open === 'true', height: Number.isFinite(height) ? height : 0 }
  },
})

/** Per gateway: whether a missions list shows every mission or only the active ones. */
export const missionsShowAll = StateCell.make({
  scope: 'gateway', key: 'missions.showAll', version: 1, schema: Schema.Boolean, default: false,
})
