import { Schema } from 'effect'
import type * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'

import { maxRememberedFolds, type FoldMemory } from '../../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import * as StateCell from '../cell/StateCell.ts'

/** Turn ids are unique across conversations, so one bounded list covers every transcript. Oldest first. */
const Folds = Schema.Array(Schema.Tuple([Schema.String, Schema.Boolean]))
type Folds = typeof Folds.Type

/**
 * The reader's work-log fold choices, shared by every window: a turn folded in one stays folded in all.
 * Only the last `maxRememberedFolds` choices are kept.
 */
export const transcriptFolds: Atom.Writable<Folds> = StateCell.make({
  scope: 'global', key: 'transcript.folds', version: 1, schema: Folds, default: [],
})

/** The kit's fold port over the cell. Reads see other windows' choices the next time a turn mounts. */
export const persistedFolds = (registry: AtomRegistry.AtomRegistry, cell: Atom.Writable<Folds> = transcriptFolds): FoldMemory => ({
  get: (id) => registry.get(cell).find(([key]) => key === id)?.[1],
  set: (id, expanded) => {
    const next = [...registry.get(cell).filter(([key]) => key !== id), [id, expanded] as const]
    registry.set(cell, next.slice(-maxRememberedFolds))
  },
})
