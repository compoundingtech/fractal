import { Schema } from 'effect'

import * as StateCell from '../cell/StateCell.ts'
import { legacyEntry } from '../legacy.ts'

const legacy = <A, I>(schema: Schema.Codec<A, I>, key: string): A | undefined => {
  const value = legacyEntry(key)
  if (value === undefined) return undefined
  const decoded = Schema.decodeUnknownOption(schema)(value)
  return decoded._tag === 'Some' ? decoded.value : undefined
}
const storedNumber = (key: string): number | undefined => {
  if (typeof window === 'undefined') return undefined
  try {
    const value = Number(window.localStorage.getItem(key) ?? Number.NaN)
    return Number.isFinite(value) ? value : undefined
  } catch { return undefined }
}

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
  seed: () => {
    const tab = legacy(RightDockTab, 'inspector.view')
    const width = storedNumber('composition.live.rightPanel')
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
  seed: () => {
    const closed = legacy(Schema.Boolean, 'round2.sidebarClosed')
    const ratio = legacy(Schema.Finite, 'round2.sidebarRatio')
    return closed === undefined && ratio === undefined ? undefined : { open: closed !== true, ratio: ratio ?? defaultLeftSidebar.ratio }
  },
})
