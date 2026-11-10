import { describe, it } from '@effect/vitest'
import { expect } from 'vitest'
import { compareFixtureDom, type FixtureSeedExpectation } from './fixtureDomEvidence.ts'

const seed: FixtureSeedExpectation = {
  itemId: 'message/fixture-seed', title: 'client-control fixture 0001',
  bodySha256: 'a'.repeat(64), subjectIds: ['message/fixture-seed', 'message/fixture-other'],
}
const observation = { title: seed.title, bodySha256: seed.bodySha256, itemIds: [seed.itemId] }

describe('independent rendered fixture comparison', () => {
  it('requires exact title, exact body digest and manifest membership', () => {
    expect(compareFixtureDom(seed, observation)).toEqual({ titleMatches: true, bodyMatches: true, catalogMatches: true, passed: true })
  })

  it('does not normalize title or accept a client-reported digest in place of a differing DOM digest', () => {
    expect(compareFixtureDom(seed, { ...observation, title: `${seed.title} ` }).passed).toBe(false)
    expect(compareFixtureDom(seed, { ...observation, bodySha256: 'b'.repeat(64) }).passed).toBe(false)
  })

  it('refuses a matching seed alongside any item outside the actual catalog', () => {
    expect(compareFixtureDom(seed, { ...observation, itemIds: [seed.itemId, 'message/outside-catalog'] })).toEqual({
      titleMatches: true, bodyMatches: true, catalogMatches: false, passed: false,
    })
  })

  it('requires the seed to be rendered even when all other visible ids are allowed', () => {
    expect(compareFixtureDom(seed, { ...observation, itemIds: [] }).passed).toBe(false)
    expect(compareFixtureDom(seed, { ...observation, itemIds: ['message/fixture-other'] }).passed).toBe(false)
  })
})
