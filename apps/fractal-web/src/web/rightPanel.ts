import { Schema } from 'effect'
import { persistedAtom } from '../state/persistence.ts'

/**
 * The right dock: one setting for the whole app, whatever tab or gateway is showing, and shared by every
 * window. `state-cells` replaces this with its global cell; the shape stays.
 */
export const RightPanel = Schema.Struct({
  open: Schema.Boolean,
  tab: Schema.Literals(['details', 'changes', 'resources']),
})
export type RightPanel = typeof RightPanel.Type

export const rightPanelAtom = persistedAtom({ key: 'live.rightPanel.state', schema: RightPanel, defaultValue: { open: false, tab: 'details' } })
