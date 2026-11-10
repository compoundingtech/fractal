// @vitest-environment node
import { describe, it } from '@effect/vitest'
import { expect, vi } from 'vitest'
import { defaultWebCryptoSha256, type ClientControlSha256 } from '../../../../clients/typescript/fractal-client/ClientControl.generated.ts'
import { compareFixtureDom, observeFixtureDom, type FixtureDomEvidence, type FixtureSeedExpectation } from './fixtureDomEvidence.ts'

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

  it('hashes only exact question UTF-8 with the generated default or an injected recording implementation', async () => {
    const question = 'Is the unit fixture ready?'
    const questionBytes = new TextEncoder().encode(question)
    const expectedDigest = Buffer.from(await defaultWebCryptoSha256(questionBytes)).toString('hex')
    const observedSeed = { ...seed, questionTextSha256: expectedDigest }
    const hashInputs: Uint8Array[] = []
    const recordingSha256: ClientControlSha256 = async input => {
      hashInputs.push(input.slice())
      return defaultWebCryptoSha256(input)
    }
    for (const sha256 of [undefined, recordingSha256]) {
      // Model the single browser DOM-read boundary, not the hashing implementation.
      const evaluateAll = vi.fn().mockResolvedValue([{
        titles: [{ text: seed.title, leaf: true }],
        questions: [{ text: question, leaf: true }],
        itemIds: [seed.askId],
      }])
      const written: FixtureDomEvidence[] = []
      const evidence = await observeFixtureDom({
        boundary: {
          origin: 'http://127.0.0.1:43127', approvedOrigin: 'http://127.0.0.1:43127',
          normalOrigins: ['http://127.0.0.1:5173'],
          gateway: 'unix:/isolated/client-control-fixture/run/fractal-client.sock',
          resolvedGatewayPath: '/isolated/client-control-fixture/run/fractal-client.sock',
          approvedClientSocketPath: '/isolated/client-control-fixture/run/fractal-client.sock',
          approvedInstanceRoot: '/isolated/client-control-fixture',
          normalGatewayPaths: ['/run/fractal-client.sock'],
        },
        commit: 'a'.repeat(40),
        page: { url: () => 'http://127.0.0.1:43127/attention', locator: () => ({ evaluateAll }) },
        seed: observedSeed, titleSelector: '.title', questionSelector: '.question',
        noAnswerGuard: { assertNoAnswers: () => {} },
        ...(sha256 === undefined ? {} : { sha256 }),
        writeEvidence: async value => { written.push(value) },
      })
      expect(evidence.renderedQuestionTextSha256).toBe(expectedDigest)
      expect(evidence.questionMatches).toBe(true)
      expect(evidence.passed).toBe(true)
      expect(written).toEqual([evidence])
      expect(evaluateAll).toHaveBeenCalledOnce()
    }
    expect(hashInputs).toEqual([questionBytes])
  })
})
