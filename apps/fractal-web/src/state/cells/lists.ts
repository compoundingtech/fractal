import { Schema } from 'effect'

import * as StateCell from '../cell/StateCell.ts'

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
  seed: (retired, gateway) => StateCell.retiredEntry(retired, SidebarFilters, `${gateway}:sidebar.filters`),
})
/** Per gateway: collapsed folders, plus `wf/expanded/<agent>` markers for parent rows the reader opened. */
export const sidebarCollapsed = StateCell.make({
  scope: 'gateway', key: 'sidebar.collapsedFolders', version: 1, schema: FolderIds, default: [] as readonly string[],
  seed: (retired, gateway) => StateCell.retiredEntry(retired, FolderIds, `${gateway}:sidebar.collapsedFolders`),
})
export const sidebarTab = StateCell.make({
  scope: 'gateway', key: 'sidebar.tab', version: 1, schema: SidebarTab, default: 'agents' as SidebarTab,
  seed: (retired, gateway) => StateCell.retiredEntry(retired, SidebarTab, `${gateway}:sidebar.tab`),
})

export const UsageSection = Schema.Struct({ open: Schema.Boolean, height: Schema.Finite })
export type UsageSection = typeof UsageSection.Type
/** The sidebar's Usage section, one setting shared by every window. */
export const usageSection = StateCell.make({
  scope: 'global', key: 'sidebar.usageSection', version: 1, schema: UsageSection, default: { open: false, height: 0 },
  seed: (retired) => {
    const open = retired.raw('wf.sidebar.usageSection.open')
    const height = StateCell.retiredNumber(retired, 'wf.sidebar.usageSection.height') ?? Number.NaN
    return open === undefined && !Number.isFinite(height) ? undefined : { open: open === 'true', height: Number.isFinite(height) ? height : 0 }
  },
})

/** Per gateway: whether a missions list shows every mission or only the active ones. */
export const missionsShowAll = StateCell.make({
  scope: 'gateway', key: 'missions.showAll', version: 1, schema: Schema.Boolean, default: false,
})

export const MissionsListView = Schema.Struct({
  group: Schema.Literals(['state', 'project', 'agent']),
  sort: Schema.Literals(['eta', 'activity', 'progress']),
  /** Group keys the reader collapsed; "Recently done" starts collapsed. */
  collapsed: Schema.Array(Schema.String),
})
export type MissionsListView = typeof MissionsListView.Type
export const defaultMissionsListView: MissionsListView = { group: 'state', sort: 'activity', collapsed: ['state:done'] }
/** Per gateway: how the missions list groups and sorts, and which groups are collapsed. */
export const missionsListView = StateCell.make({
  scope: 'gateway', key: 'missions.listView', version: 1, schema: MissionsListView, default: defaultMissionsListView,
})
