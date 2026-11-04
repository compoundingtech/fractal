import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { describe, expect, it } from 'vitest'
import { pinsEditable, planPinToggle, sidebarPins } from './sidebarPins.ts'
import type { PinsState } from './pins.ts'

const state: PinsState = { phase: 'synced', editable: true, members: ['a1', 'a0'].map((key) => ({
  id: `membership/${key}`, container: 'arrangement/person/example/pins', member: `agent/${key}`,
  revision: 'claim/place', position: { bucket: null, key },
})) }
describe('sidebar Pins binding', () => {
  it('appends after the last root key and removes existing membership', () => {
    expect(planPinToggle(state, 'mission/example')).toEqual([{ op: 'place', member: 'mission/example', bucket: null, key: 'a2' }])
    expect(planPinToggle(state, 'agent/a0')).toEqual([{ op: 'remove', member: 'agent/a0' }])
    expect(planPinToggle({ ...state, members: [] }, 'agent/first')).toEqual([{ op: 'place', member: 'agent/first', bucket: null, key: 'a0' }])
  })
  it('requires the live follow grant, and never opts absent arrangements into creation', () => {
    expect(pinsEditable(state)).toBe(true)
    for (const next of [{ phase: 'synced' as const, members: state.members }, { ...state, editable: false },
      ...(['connecting', 'pending', 'unavailable'] as const).map((phase) => ({ ...state, phase }))]) {
      expect(pinsEditable(next)).toBe(false)
      expect(planPinToggle(next, 'agent/new')).toEqual([])
    }
  })
  it('keeps disabled fixtures inert without a browser origin or client', () => {
    const registry = AtomRegistry.make()
    try {
      expect(sidebarPins(false)).toBe(sidebarPins(false))
      const pins = registry.get(sidebarPins(false))
      pins.toggle('agent/example'); pins.retry()
      expect(pins).toMatchObject({ editable: false, state: { phase: 'synced', members: [] } })
    } finally { registry.dispose() }
  })
})
