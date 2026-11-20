import { describe, expect, it } from 'vitest'

import { agentDisplayName, isFractalMachinery, nameInSentence, personName } from './agentName.ts'

const id = 'agent/example/atlas'
const opaque = 'agent/example/d415892ea7a1dab11e6ba7ed22a8eb13'

describe('agent display names', () => {
  it('shows a served name as declared, slugs included', () => {
    expect(agentDisplayName({ id, name: 'atlas' })).toBe('atlas')
    expect(agentDisplayName({ id, name: 'Atlas' })).toBe('Atlas')
    expect(agentDisplayName({ id, name: 'Manager' })).toBe('Manager')
    expect(agentDisplayName({ id, name: 'Atlas/Manager' })).toBe('Atlas/Manager')
    expect(agentDisplayName({ id: opaque, name: 'd415892e' })).toBe('d415892e')
  })

  it('preserves a supplied single name even when it matches the id slug', () => {
    expect(agentDisplayName({ id: 'agent/builder', name: 'builder' })).toBe('builder')
    expect(agentDisplayName({ id: 'agent/builder' })).toBe('An agent')
  })

  it('reads an agent with no name as An agent, never as any part of its id', () => {
    for (const name of [undefined, '', '  ']) {
      expect(agentDisplayName({ id, name })).toBe('An agent')
      expect(agentDisplayName({ id: opaque, name })).toBe('An agent')
    }
  })

  it('treats the fallback forms Fractal serves for a seat without a name as no name', () => {
    for (const name of [id, 'example/atlas', ' example/atlas ']) expect(agentDisplayName({ id, name })).toBe('An agent')
    for (const name of [opaque, 'example/d415892ea7a1dab11e6ba7ed22a8eb13']) expect(agentDisplayName({ id: opaque, name })).toBe('An agent')
    expect(agentDisplayName({ id: 'agent/build-host-a/worker-tests', name: 'build-host-a/worker-tests' })).toBe('An agent')
  })

  it('lowercases only the unnamed form inside a sentence', () => {
    expect(nameInSentence(agentDisplayName({ id }))).toBe('an agent')
    expect(nameInSentence(agentDisplayName({ id, name: 'Atlas' }))).toBe('Atlas')
    expect(nameInSentence(agentDisplayName({ id: 'agent/st3/reconciler' }))).toBe('Fractal')
  })

  it('reads a person by their handle, and nothing else as a person', () => {
    expect(personName('person/robin')).toBe('robin')
    expect(personName('person/robin/session/1')).toBe('robin')
    for (const actor of ['person/', id, 'daemon/runtime', 'step-run/example/build']) expect(personName(actor)).toBeUndefined()
  })

  // Mirrors is_fractal in crates/fractal-ui-model/src/attention.rs.
  it('treats the daemon and the st3 reconciler as Fractal machinery, and nothing else', () => {
    for (const machinery of ['daemon/runtime', 'daemon/site-a', 'agent/st3/reconciler']) expect(isFractalMachinery(machinery)).toBe(true)
    for (const actor of [id, 'agent/st3/reconciler/session/1', 'agent/st3', 'agent/example/reconciler', 'person/avery', 'daemon']) expect(isFractalMachinery(actor)).toBe(false)
  })

  it('names Fractal machinery Fractal whatever the roster calls it, as the terminal app does', () => {
    for (const name of [undefined, 'Reconciler', 'st3/reconciler', 'agent/st3/reconciler']) expect(agentDisplayName({ id: 'agent/st3/reconciler', name })).toBe('Fractal')
    expect(agentDisplayName({ id: 'daemon/runtime', name: 'runtime' })).toBe('Fractal')
    expect(agentDisplayName({ id: 'agent/example/reconciler', name: 'Reconciler' })).toBe('Reconciler')
  })
})
