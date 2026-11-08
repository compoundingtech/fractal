import { Schema } from 'effect'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'

import * as StateCell from '../cell/StateCell.ts'

/** How many conversations' positions a person keeps; the least recently read go first. */
export const maxReadPositions = 500

const Position = Schema.NullOr(Schema.Number)

/**
 * The time of the newest item a person has seen in a conversation, keyed `<gateway>/<conversation>`.
 * Person-scoped like drafts, so a revoke or sign-out forgets it, and shared by this browser's windows.
 * `null` until the person first looks: nothing is "new" in a conversation never opened.
 */
export const readPosition = StateCell.make({
  scope: 'person', key: 'conversation.read', version: 1, schema: Position, default: null, retain: maxReadPositions,
})

export interface ReadPositionStore {
  readonly read: (conversation: string) => number | undefined
  /** Moves the position forward only: an older window or a late observation never moves it back. */
  readonly advance: (conversation: string, atMs: number) => void
  readonly subscribe: (conversation: string, listener: (atMs: number | undefined) => void) => () => void
}

export const cellReadPositionStore = ({ registry, person, gateway }: {
  readonly registry: AtomRegistry.AtomRegistry
  readonly person: string
  readonly gateway: string
}): ReadPositionStore => {
  const cell = (conversation: string) => readPosition(person, `${gateway}/${conversation}`)
  const read = (conversation: string) => registry.get(cell(conversation)) ?? undefined
  return {
    read,
    advance: (conversation, atMs) => {
      if (!Number.isFinite(atMs)) return
      const current = read(conversation)
      if (current === undefined || atMs > current) registry.set(cell(conversation), atMs)
    },
    subscribe: (conversation, listener) => registry.subscribe(cell(conversation), (value) => listener(value ?? undefined)),
  }
}
