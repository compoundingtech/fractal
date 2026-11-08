// @vitest-environment node
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, describe, expect, it } from 'vitest'

import * as StateCell from '../state/cell/StateCell.ts'
import { memoryOrigin } from '../state/cell/testing.ts'
import { forgetPerson } from '../state/cells/drafts.ts'
import { cellRecentStore } from './recent.ts'

let uninstall: (() => Promise<void>) | undefined
const install = (origin: ReturnType<typeof memoryOrigin>) => {
  void uninstall?.()
  uninstall = StateCell.install(StateCell.layer(origin.window().environment, { debounceMs: 5 }))
}
afterEach(async () => { await uninstall?.(); uninstall = undefined })

describe('palette recent on cells', () => {
  it('keeps the recent list per person and gateway across a reload', () => {
    const origin = memoryOrigin()
    install(origin)
    const store = cellRecentStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'gw' })
    store.set(['agent:agent/a', 'zoom'])
    expect(cellRecentStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'other' }).get()).toEqual([])
    StateCell.flush()
    expect([...origin.local.keys()].filter(key => key.includes('palette.recent'))).toEqual(['wf.c/p:person%2Fada/palette.recent/gw'])
    install(origin)
    expect(cellRecentStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'gw' }).get()).toEqual(['agent:agent/a', 'zoom'])
  })

  it('tells subscribers about changes, and goes when the person is forgotten', () => {
    install(memoryOrigin())
    const registry = AtomRegistry.make()
    const store = cellRecentStore({ registry, person: 'person/ada', gateway: 'gw' })
    const seen: (readonly string[])[] = []
    const stop = store.subscribe(() => seen.push(store.get()))
    store.set(['a'])
    expect(seen.at(-1)).toEqual(['a'])
    stop()
    forgetPerson('person/ada')
    expect(cellRecentStore({ registry: AtomRegistry.make(), person: 'person/ada', gateway: 'gw' }).get()).toEqual([])
  })
})
