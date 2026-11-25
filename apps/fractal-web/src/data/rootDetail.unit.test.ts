import { describe, expect, it } from 'vitest'
import { Schema } from 'effect'
import type { DetailsSupplement, DetailsSupplementQuery } from '../extensions/contract.ts'
import type { Feed } from './source.ts'
import { detailsRequestEligibility, projectRootDetail, RootDetailObservation } from './rootDetail.ts'

const root = '8cf4a5a1-3423-4e43-86cc-615f442f4ce2'
const query: DetailsSupplementQuery = { agentRef: 'agent/fictional', harness: 'omp', host: 'host/fictional', incarnationId: 'fixture-incarnation', nativeSessionId: root }
const observation: RootDetailObservation = {
  agent: query.agentRef, host: 'host/fictional', incarnationId: 'fixture-incarnation', nativeSessionId: root, window: '1d', inclusiveUsd: 99, childCount: 25,
  children: Array.from({ length: 20 }, (_, i) => ({ sessionId: `descendant-${i}`, usd: 1, requests: 2 })), attributedFraction: 1,
}
const observedValue = (feed: Feed<DetailsSupplement>): DetailsSupplement => {
  if (feed._tag !== 'Observed') throw new TypeError('Expected an observation')
  return feed.value
}

describe('detailsRequestEligibility', () => {
  it('reads only for an omp agent with a verified current root', () => {
    for (const harness of ['claude', 'codex', undefined]) expect(detailsRequestEligibility({ ...query, harness })).toMatchObject({ _tag: 'Unavailable', code: 'root-detail-not-covered' })
    for (const missing of ['nativeSessionId', 'incarnationId', 'host'] as const)
      expect(detailsRequestEligibility({ ...query, [missing]: undefined })).toMatchObject({ _tag: 'Unavailable', code: 'root-detail-not-attributed' })
    expect(detailsRequestEligibility(query)).toBeUndefined()
  })

  it('waits while native root facts acquire, except for a known other harness', () => {
    expect(detailsRequestEligibility({ ...query, nativeSessionId: undefined, nativeRootPending: true })).toEqual({ _tag: 'Waiting' })
    expect(detailsRequestEligibility({ agentRef: query.agentRef, nativeRootPending: true })).toEqual({ _tag: 'Waiting' })
    expect(detailsRequestEligibility({ ...query, nativeSessionId: undefined, nativeRootPending: false })).toMatchObject({ _tag: 'Unavailable', code: 'root-detail-not-attributed' })
    for (const harness of ['claude', 'codex']) expect(detailsRequestEligibility({ ...query, harness, nativeRootPending: true })).toMatchObject({ _tag: 'Unavailable', code: 'root-detail-not-covered' })
  })
})

describe('projectRootDetail', () => {
  it('reports the inclusive total independently of the listed descendants, with no native main-session accounting', () => {
    const value = observedValue(projectRootDetail(query, observation))
    expect(value.totalUsd).toEqual({ _tag: 'Known', value: 99, source: 'series' })
    expect(value.subagentCount).toMatchObject({ _tag: 'Known', value: 25 })
    expect(value.subagents).toMatchObject({ _tag: 'Known' })
    expect('mainUsd' in value).toBe(false)
    expect('tokens' in value).toBe(false)
    expect(value.coverage).toEqual({ _tag: 'Partial', attributedFraction: 1 })
  })

  it('labels the reported window it was given', () => {
    expect(observedValue(projectRootDetail(query, observation)).usdWindow)
      .toEqual({ _tag: 'Known', source: 'series', value: { _tag: 'Reported', label: 'last 24 hours (as reported; exact cut bounds and as-of unknown)' } })
    const weekly = observedValue(projectRootDetail(query, { ...observation, window: '7d' }))
    expect(weekly.usdWindow).toMatchObject({ value: { label: 'last 7 days (as reported; exact cut bounds and as-of unknown)' } })
    expect(weekly.subagentsWindow).toEqual(weekly.usdWindow)
  })

  it('keeps unreported descendant facts unknown and reported roles and models as given', () => {
    const children = [
      { sessionId: 'descendant-review', role: 'reviewer', model: 'reported-review-model', usd: 1, requests: 2 },
      { sessionId: 'descendant-generic', role: 'subagent', usd: 3, requests: 4 },
      { sessionId: 'descendant-missing', usd: 4, requests: 5 },
      { sessionId: 'descendant-unknown', role: '.unknown', model: '.unknown', usd: 5, requests: 6 },
    ]
    const value = observedValue(projectRootDetail(query, { ...observation, childCount: children.length, children }))
    if (value.subagents._tag !== 'Known') throw new TypeError('Expected reported descendants')
    const rows = value.subagents.value
    expect(rows.map((row) => row.sessionId)).toEqual(children.map((child) => child.sessionId))
    expect(rows.slice(0, 2).map((row) => row.label)).toEqual(['reviewer', 'subagent'].map((label) => ({ _tag: 'Known', value: label, source: 'series' })))
    for (const row of rows.slice(2)) expect(row.label).toMatchObject({ _tag: 'Unknown', detail: 'Descendant role was not reported' })
    expect(rows[0]?.model).toMatchObject({ _tag: 'Known', value: 'reported-review-model' })
    expect(rows[3]?.model._tag).toBe('Unknown')
    for (const row of rows) for (const fact of [row.parentSessionId, row.firstMs, row.lastMs]) expect(fact._tag).toBe('Unknown')
  })

  it('distinguishes omitted descendants, count and coverage from a reported zero', () => {
    expect(projectRootDetail(query, { ...observation, children: undefined, childCount: undefined, attributedFraction: undefined }))
      .toMatchObject({ _tag: 'Observed', value: { subagents: { _tag: 'Unknown' }, subagentCount: { _tag: 'Unknown' }, coverage: { _tag: 'Unavailable' } } })
    expect(projectRootDetail(query, { ...observation, inclusiveUsd: 0, children: [], childCount: 0 }))
      .toMatchObject({ _tag: 'Observed', value: { totalUsd: { _tag: 'Known', value: 0 }, subagents: { _tag: 'Known', value: [] }, subagentCount: { _tag: 'Known', value: 0 } } })
  })

  it('drops an answer for another root, incarnation, owner or agent', () => {
    for (const changed of [{ nativeSessionId: 'other' }, { incarnationId: 'other' }, { host: 'host/other' }, { agent: 'agent/other' }])
      expect(projectRootDetail(query, { ...observation, ...changed })).toMatchObject({ _tag: 'Unavailable', code: 'root-detail-not-attributed' })
  })

  it('shows only reported durations, with their provenance', () => {
    expect(observedValue(projectRootDetail(query, observation)).durations).toEqual([])
    const value = observedValue(projectRootDetail(query, { ...observation, wallHours: 2.5, activeHours: 0.4, producerHost: 'fictional-ledger-host' }))
    expect(value.durations).toEqual([
      { label: expect.stringContaining('Root observed wall span'), value: '2.5 h' },
      { label: expect.stringContaining('gaps ≤10 min'), value: '0.4 h' },
    ])
    expect(value.durations?.[0]?.label).toContain('reported ledger host fictional-ledger-host')
    expect(value.durations?.[0]?.label).toContain('reported 1d, cut bounds unknown')
  })

  it('decodes at most 20 descendants and a fraction within 0..1', () => {
    const decode = Schema.decodeUnknownSync(RootDetailObservation)
    expect(decode(observation)).toEqual(observation)
    expect(() => decode({ ...observation, children: Array.from({ length: 21 }, () => observation.children?.[0]) })).toThrow()
    expect(() => decode({ ...observation, attributedFraction: 1.01 })).toThrow()
    expect(() => decode({ ...observation, window: 'all' })).toThrow()
  })
})
