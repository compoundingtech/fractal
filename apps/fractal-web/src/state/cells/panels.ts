import { Schema } from 'effect'

import * as StateCell from '../cell/StateCell.ts'

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
