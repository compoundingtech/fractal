import { makeAdmission } from '@fractal/sdk/admission'
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
