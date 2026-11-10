import { describe, it } from '@effect/vitest'
import { expect } from 'vitest'
import { compareFixtureDom, type FixtureSeedExpectation } from './fixtureDomEvidence.ts'

const seed: FixtureSeedExpectation = {
  askId: 'attention/unit-seed', title: 'Unit ask title',
  questionTextSha256: 'a'.repeat(64), subjectIds: ['attention/unit-seed', 'attention/unit-other'],
}
const observation = { title: seed.title, questionTextSha256: seed.questionTextSha256, itemIds: [seed.askId] }

describe('independent rendered fixture comparison', () => {
  it('requires exact ask title, exact question-text digest and manifest membership', () => {
    expect(compareFixtureDom(seed, observation)).toEqual({ titleMatches: true, questionMatches: true, catalogMatches: true, passed: true })
  })

  it('does not normalize title or accept a client-reported digest in place of a differing DOM digest', () => {
    expect(compareFixtureDom(seed, { ...observation, title: `${seed.title} ` }).passed).toBe(false)
    expect(compareFixtureDom(seed, { ...observation, questionTextSha256: 'b'.repeat(64) }).passed).toBe(false)
  })

  it('refuses a matching seed alongside any item outside the actual catalog', () => {
    expect(compareFixtureDom(seed, { ...observation, itemIds: [seed.askId, 'attention/outside-catalog'] })).toEqual({
      titleMatches: true, questionMatches: true, catalogMatches: false, passed: false,
    })
  })

  it('requires the seed to be rendered even when all other visible ids are allowed', () => {
    expect(compareFixtureDom(seed, { ...observation, itemIds: [] }).passed).toBe(false)
    expect(compareFixtureDom(seed, { ...observation, itemIds: ['attention/unit-other'] }).passed).toBe(false)
  })
})
