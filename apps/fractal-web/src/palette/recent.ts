import { Schema } from 'effect'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { RecentStore } from '@fractal/ui/workspace/palette'

import * as StateCell from '../state/cell/StateCell.ts'

/** How many entities and commands the palette remembers. */
export const recentLimit = 20

/**
 * The palette's last choices, newest first: pane keys for entities, fixed ids for commands. Owned by the
 * signed-in person and kept per gateway (the member), so it follows them to every window and goes on
 * revoke or sign-out with their drafts.
 */
export const paletteRecent = StateCell.make({
  scope: 'person', key: 'palette.recent', version: 1, schema: Schema.Array(Schema.String), default: [], retain: 8,
})

/** The kit's recent port over the cell. */
export const cellRecentStore = ({ registry, person, gateway }: {
  readonly registry: AtomRegistry.AtomRegistry
  readonly person: string
  readonly gateway: string
}): RecentStore => {
  const cell = paletteRecent(person, gateway)
  return {
    get: () => registry.get(cell),
    set: ids => registry.set(cell, ids.slice(0, recentLimit)),
    subscribe: listener => registry.subscribe(cell, () => listener()),
  }
}
