import { describe, expect, it } from 'vitest'
import { createReadAdvancementPolicy, gateReadAdvancement } from './readAdvancement.ts'

describe('read advancement boundary', () => {
  it('leaves an ordinary store and its return value unchanged', () => {
    const positions = new Map<string, number>()
    const policy = createReadAdvancementPolicy()
    const advance = gateReadAdvancement(policy, (id: string, position: number) => {
      positions.set(id, position)
      return position
    })
    expect(advance('message/seed', 100)).toBe(100)
    expect(positions.get('message/seed')).toBe(100)
  })

  it('suppresses persistence AND outbound sync at the store boundary', () => {
    const persisted: number[] = []
    const synchronized: number[] = []
    const policy = createReadAdvancementPolicy()
    const advance = gateReadAdvancement(policy, (position: number) => {
      persisted.push(position)
      synchronized.push(position)
    })
    // Both automatic seen and explicit mark-read retain the same store callback.
    const onSeen = () => advance(100)
    const markRead = () => advance(200)
    policy.prohibitForTestEnrollment()
    onSeen()
    markRead()
    expect(persisted).toEqual([])
    expect(synchronized).toEqual([])
  })

  it('never re-enables advancement when test enrollment ends', () => {
    const policy = createReadAdvancementPolicy()
    policy.prohibitForTestEnrollment()
    policy.prohibitForTestEnrollment()
    expect(policy.mayAdvance()).toBe(false)
  })
})
