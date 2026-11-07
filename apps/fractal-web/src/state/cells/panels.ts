import { Schema } from 'effect'

import * as StateCell from '../cell/StateCell.ts'

export const RightDockTab = Schema.Literals(['details', 'changes', 'resources'])
export type RightDockTab = typeof RightDockTab.Type
export const RightDock = Schema.Struct({ open: Schema.Boolean, tab: RightDockTab, width: Schema.Finite })
export type RightDock = typeof RightDock.Type
export const defaultRightDock: RightDock = { open: false, tab: 'details', width: 440 }

/**
 * The right dock: one setting for every conversation, gateway and window, painted on the first
 * frame. Width is the preferred size; the dock still clamps it to the room it has.
 */
export const rightDock = StateCell.make({
  scope: 'global',
  key: 'panels.rightDock',
  version: 1,
  schema: RightDock,
  default: defaultRightDock,
  tier: 'boot',
  boot: ({ open, width }) => ({ 'dock.open': open, 'dock.width': Math.round(width) }),
  seed: (retired) => {
    const tab = StateCell.retiredEntry(retired, RightDockTab, 'inspector.view')
    const width = StateCell.retiredNumber(retired, 'composition.live.rightPanel')
    return tab === undefined && width === undefined ? undefined : { ...defaultRightDock, ...(tab === undefined ? {} : { tab }), ...(width === undefined ? {} : { width }) }
  },
})

export const LeftSidebar = Schema.Struct({ open: Schema.Boolean, ratio: Schema.Finite })
export type LeftSidebar = typeof LeftSidebar.Type
export const defaultLeftSidebar: LeftSidebar = { open: true, ratio: 256 / 1440 }

/** The left sidebar: open state and width as a share of the window, painted on the first frame. */
export const leftSidebar = StateCell.make({
  scope: 'global',
  key: 'panels.leftSidebar',
  version: 1,
  schema: LeftSidebar,
  default: defaultLeftSidebar,
  tier: 'boot',
  boot: ({ open, ratio }) => ({ 'sidebar.open': open, 'sidebar.ratio': ratio }),
  seed: (retired) => {
    const closed = StateCell.retiredEntry(retired, Schema.Boolean, 'round2.sidebarClosed')
    const ratio = StateCell.retiredEntry(retired, Schema.Finite, 'round2.sidebarRatio')
    return closed === undefined && ratio === undefined ? undefined : { open: closed !== true, ratio: ratio ?? defaultLeftSidebar.ratio }
  },
})
