// @vitest-environment node
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, describe, expect, it } from 'vitest'

import { createComposerDraftSession } from '../../web/composerDrafts.ts'
import * as StateCell from '../cell/StateCell.ts'
import { memoryOrigin } from '../cell/testing.ts'
import { cellComposerDraftStore, forgetSessionPerson, rememberSessionPerson } from './drafts.ts'

let uninstall: (() => Promise<void>) | undefined
const install = (origin: ReturnType<typeof memoryOrigin>) => {
  void uninstall?.()
  uninstall = StateCell.install(StateCell.layer(origin.window().environment, { debounceMs: 5 }))
}
afterEach(async () => { await uninstall?.(); uninstall = undefined })
const draftKeys = (origin: ReturnType<typeof memoryOrigin>) => [...origin.local.keys()].filter((key) => key.includes('composer.draft'))

describe('composer drafts on cells', () => {
  it('restores a draft after a reload, under the person and gateway', () => {
    const origin = memoryOrigin()
    install(origin)
    const store = cellComposerDraftStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'gw' })
    const session = createComposerDraftSession({ store, conversationId: 'agent/one', now: () => 10 })
    session.edit('Keep this unsent note')
    session.close()
    StateCell.flush()
    expect(draftKeys(origin)).toEqual(['wf.c/p:person%2Fada/composer.draft/gw%2Fagent%2Fone'])
    install(origin)
    const reopened = createComposerDraftSession({ store: cellComposerDraftStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'gw' }), conversationId: 'agent/one' })
    expect(reopened.getSnapshot()).toEqual({ version: 1, text: 'Keep this unsent note', updatedAt: 10 })
  })

  it('refuses an older write and oversized text', () => {
    install(memoryOrigin())
    const store = cellComposerDraftStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'gw' })
    expect(store.write('c', { version: 1, text: 'new', updatedAt: 20 })._tag).toBe('Saved')
    expect(store.write('c', { version: 1, text: 'old', updatedAt: 10 })).toEqual({ _tag: 'Stale', draft: { version: 1, text: 'new', updatedAt: 20 } })
    expect(store.write('c', { version: 1, text: 'x'.repeat(70 * 1024), updatedAt: 30 })._tag).toBe('TooLarge')
  })

  it('a revoke removes the person\'s drafts, including the one a previous session recorded', () => {
    const origin = memoryOrigin()
    install(origin)
    const registry = AtomRegistry.make()
    rememberSessionPerson('person/ada')
    cellComposerDraftStore({ registry, person: 'person/ada', gateway: 'gw' }).write('c', { version: 1, text: 'secret', updatedAt: 1 })
    cellComposerDraftStore({ registry, person: 'person/bob', gateway: 'gw' }).write('c', { version: 1, text: 'bob', updatedAt: 1 })
    StateCell.flush()
    // After a reload discovery is refused, so only the recorded person is known.
    install(origin)
    forgetSessionPerson(undefined)
    StateCell.flush()
    expect([...origin.local.values()].some((value) => value.includes('secret'))).toBe(false)
    expect([...origin.local.values()].some((value) => value.includes('bob'))).toBe(true)
  })
})
