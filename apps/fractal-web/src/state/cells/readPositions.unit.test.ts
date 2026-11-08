// @vitest-environment node
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, describe, expect, it } from 'vitest'

import * as StateCell from '../cell/StateCell.ts'
import { memoryOrigin } from '../cell/testing.ts'
import { forgetPerson } from './drafts.ts'
import { cellReadPositionStore } from './readPositions.ts'

let uninstall: (() => Promise<void>) | undefined
const install = (origin: ReturnType<typeof memoryOrigin>) => {
  void uninstall?.()
  uninstall = StateCell.install(StateCell.layer(origin.window().environment, { debounceMs: 5 }))
}
afterEach(async () => { await uninstall?.(); uninstall = undefined })
const store = () => cellReadPositionStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'gw' })

describe('read positions on cells', () => {
  it('is unknown until the person first looks, then survives a reload under the person and gateway', () => {
    const origin = memoryOrigin()
    install(origin)
    expect(store().read('agent/one')).toBeUndefined()
    store().advance('agent/one', 40)
    StateCell.flush()
    expect([...origin.local.keys()].filter((key) => key.includes('conversation.read'))).toEqual(['wf.c/p:person%2Fada/conversation.read/gw%2Fagent%2Fone'])
    install(origin)
    expect(store().read('agent/one')).toBe(40)
  })

  it('only moves forward', () => {
    install(memoryOrigin())
    const positions = store()
    positions.advance('c', 40)
    positions.advance('c', 20)
    positions.advance('c', Number.NaN)
    expect(positions.read('c')).toBe(40)
    positions.advance('c', 60)
    expect(positions.read('c')).toBe(60)
  })

  it('is forgotten with the person on revoke', () => {
    install(memoryOrigin())
    store().advance('c', 40)
    forgetPerson('person/ada')
    expect(store().read('c')).toBeUndefined()
  })
})
