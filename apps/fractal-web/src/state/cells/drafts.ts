import { Schema } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'

import { maxComposerDraftBytes, maxComposerDrafts, type ComposerDraft, type ComposerDraftStore } from '../../web/composerDrafts.ts'
import * as StateCell from '../cell/StateCell.ts'

const Draft = Schema.Struct({ text: Schema.String, updatedAt: Schema.Int })
type Draft = typeof Draft.Type
const empty: Draft = { text: '', updatedAt: 0 }

/**
 * One composer's unsent text, owned by the signed-in person and keyed `<gateway>/<conversation>`,
 * so a revoke or sign-out removes every draft that person typed. The oldest go first: past
 * `maxComposerDrafts`, and when storage is full.
 */
export const composerDraft = StateCell.make({
  scope: 'person', key: 'composer.draft', version: 1, schema: Draft, default: empty, tier: 'sensitive', retain: maxComposerDrafts,
})

/** A decision card's typed answer, kept like a composer draft until it is sent. Keyed `<gateway>/<alert>`. */
export const decisionWords = StateCell.make({
  scope: 'person', key: 'decision.words', version: 1, schema: Schema.String, default: '', tier: 'sensitive', retain: maxComposerDrafts,
})

/** The person the last session in this browser belonged to, so a revoke noticed after a reload still clears them. */
export const lastPerson = StateCell.make({
  scope: 'global', key: 'session.person', version: 1, schema: Schema.NullOr(Schema.String), default: null,
})

/** Drafts and every other person-scoped cell, gone in every window. Call on revoke and sign-out. */
export const forgetPerson = (person: string) => StateCell.clearScope('person', person)

// Cells hold their value in the store, so a private registry reads and writes the same state.
const sessionRegistry = AtomRegistry.make()
/** Discovery named the signed-in person. */
export const rememberSessionPerson = (person: string) => {
  if (sessionRegistry.get(lastPerson) !== person) sessionRegistry.set(lastPerson, person)
}
/**
 * Revoke or sign-out: forget the person discovery named, and the one the last session recorded,
 * which is all a reloaded page knows once its credential is refused.
 */
export const forgetSessionPerson = (current: string | undefined) => {
  const previous = sessionRegistry.get(lastPerson)
  for (const person of new Set([current, previous])) if (person !== undefined && person !== null) forgetPerson(person)
  if (previous !== null) sessionRegistry.set(lastPerson, null)
}

const utf8 = new TextEncoder()
const toRecord = (draft: Draft): ComposerDraft | undefined =>
  draft.updatedAt === 0 && draft.text === '' ? undefined : { version: 1, text: draft.text, updatedAt: draft.updatedAt }
const same = (left: ComposerDraft, right: ComposerDraft) => left.updatedAt === right.updatedAt && left.text === right.text

/**
 * The composer draft port over person-scoped cells. Keeps the local store's rules: a write older
 * than the shared value is refused as `Stale`, and oversized text is refused as `TooLarge`.
 */
export const cellComposerDraftStore = ({ registry, person, gateway }: {
  readonly registry: AtomRegistry.AtomRegistry
  readonly person: string
  readonly gateway: string
}): ComposerDraftStore => {
  const cell = (conversationId: string) => composerDraft(person, `${gateway}/${conversationId}`)
  const read: ComposerDraftStore['read'] = (id) => toRecord(registry.get(cell(id)))
  return {
    read,
    write: (id, draft) => {
      if (utf8.encode(draft.text).length > maxComposerDraftBytes) return { _tag: 'TooLarge' }
      const current = read(id)
      if (current !== undefined && current.updatedAt >= draft.updatedAt && !same(current, draft)) return { _tag: 'Stale', draft: current }
      registry.set(cell(id), { text: draft.text, updatedAt: draft.updatedAt })
      return { _tag: 'Saved', draft }
    },
    subscribe: (id, listener) => registry.subscribe(cell(id), (value) => {
      const record = toRecord(value)
      if (record !== undefined) listener(record)
    }),
  }
}
