import { makeAdmission } from '@fractal/sdk/admission'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it } from 'vitest'

import { makeVisibilityPolicy } from './admission.ts'

it('defaults unseen panes to visible and preserves recency across repeated invisible hints', () => {
  const policy = makeVisibilityPolicy<string>()
  expect(policy.isVisible('unseen')).toBe(true)
  expect(policy.priority('unseen')).toBeUndefined()
  policy.setVisible({ key: 'older', visible: false })
  policy.setVisible({ key: 'newer', visible: false })
  const older = policy.priority('older')!
  policy.setVisible({ key: 'older', visible: false })
  expect(policy.priority('older')).toBe(older)
  expect(older).toBeLessThan(policy.priority('newer')!)
  policy.setVisible({ key: 'older', visible: true })
  expect(policy.priority('older')).toBeUndefined()
  policy.setVisible({ key: 'older', visible: false })
  expect(policy.priority('older')!).toBeGreaterThan(policy.priority('newer')!)
})

it('evicts the least-recently-visible hidden pane while protecting visible follows', () => {
  const admission = makeAdmission({ cap: 3, policy: makeVisibilityPolicy<string>() })
  admission.open('visible')
  admission.open('older')
  admission.open('newer')
  admission.setVisible({ key: 'older', visible: false })
  admission.setVisible({ key: 'newer', visible: false })
  expect(admission.open('incoming')).toEqual({ _tag: 'Admitted', evict: 'older' })
  expect(admission.resizeLane('shared', 1)).toEqual(['newer'])
  expect([...admission.admitted()]).toEqual(['visible', 'incoming'])
  expect(admission.open('refused')).toEqual({ _tag: 'Full' })
})

it('retains hidden-pane policy through a close and keeps lane eviction isolated', () => {
  const admission = makeAdmission({ cap: 3, conversationSlots: 1, policy: makeVisibilityPolicy<string>() })
  admission.setVisible({ key: 'conversation', visible: false })
  admission.open('conversation', 'conversation')
  admission.close('conversation')
  expect(admission.isVisible('conversation')).toBe(false)
  admission.open('conversation', 'conversation')
  admission.open('shared-a')
  admission.open('shared-b')
  expect(admission.open('shared-c')).toEqual({ _tag: 'Full' })
  expect(admission.open('next-conversation', 'conversation')).toEqual({ _tag: 'Admitted', evict: 'conversation' })
})

it('admits against current interest ownership before a released pane finalizes', () => {
  const registry = AtomRegistry.make()
  let finalized = false
  const interest = Atom.make((get) => {
    get.addFinalizer(() => { finalized = true })
  })
  const policy = makeVisibilityPolicy<string>((key) =>
    key === 'outgoing' ? !registry.ensureNode(interest).canBeRemoved : undefined,
  )
  const admission = makeAdmission({ cap: 1, policy })
  try {
    const release = registry.mount(interest)
    policy.setVisible({ key: 'outgoing', visible: true })
    admission.open('outgoing')
    expect(admission.open('blocked')).toEqual({ _tag: 'Full' })
    release()
    expect(finalized).toBe(false)
    expect(admission.open('incoming')).toEqual({ _tag: 'Admitted', evict: 'outgoing' })

    // A same-turn handoff reuses the interest, but must immediately protect it again.
    admission.close('incoming')
    const releaseReacquired = registry.mount(interest)
    admission.open('outgoing')
    expect(admission.open('blocked')).toEqual({ _tag: 'Full' })
    releaseReacquired()
  } finally {
    registry.dispose()
  }
})

it('protects interest held through a mounted dependent atom', () => {
  const registry = AtomRegistry.make()
  const interest = Atom.make(() => undefined)
  const dependent = Atom.make((get) => get(interest))
  const policy = makeVisibilityPolicy<string>((key) =>
    key === 'outgoing' ? !registry.ensureNode(interest).canBeRemoved : undefined,
  )
  const admission = makeAdmission({ cap: 1, policy })
  try {
    registry.mount(dependent)
    policy.setVisible({ key: 'outgoing', visible: true })
    admission.open('outgoing')
    expect(registry.ensureNode(interest).listeners.size).toBe(0)
    expect(admission.open('incoming')).toEqual({ _tag: 'Full' })
  } finally {
    registry.dispose()
  }
})
