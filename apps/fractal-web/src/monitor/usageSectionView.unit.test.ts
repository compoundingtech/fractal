import { describe, expect, it } from 'vitest'

import { accounts, FIXTURE_NOW, incompatibleSummaryAccount, quotaEnvelope } from './fixtures.ts'
import { initialQuota, reduceQuota, type QuotaState } from './model.ts'
import { usageSectionView } from './usageSectionView.ts'
import type { QuotaAccount } from './wire.ts'

const now = FIXTURE_NOW
const loaded = (list: readonly QuotaAccount[], generatedAgo = 40_000): QuotaState =>
  reduceQuota({ state: initialQuota(true), event: { _tag: 'Succeeded', at: now, envelope: quotaEnvelope({ list, generatedAgo }) } })
const ready = (state: QuotaState) => {
  const view = usageSectionView({ state, now })
  if (view._tag !== 'ready') throw new Error(`expected ready, got ${view._tag}`)
  return view
}
const withLimit = (id: string, usage: { fraction: number | null; exhausted?: boolean; resetsIn: number | null }): QuotaAccount => {
  const base = accounts.claudeAlpha
  const limit = base.limits[0]!
  if (limit.observation.state !== 'reported') throw new Error('fixture limit must be reported')
  return {
    ...base,
    accountId: id,
    limits: [{
      ...limit, summarySlot: 'governing', summaryRole: 'governing',
      observation: {
        ...limit.observation,
        window: { ...limit.observation.window, resetsAt: usage.resetsIn === null ? null : new Date(now + usage.resetsIn).toISOString() },
        usage: { ...limit.observation.usage, usedFraction: usage.fraction, exhaustion: { state: usage.exhausted === true ? 'exhausted' : 'available', basis: 'utilization', conflict: false } },
      },
    }],
  }
}

describe('usage section view', () => {
  it('says no quota source is configured for an undeclared source or a daemon without the relay, never empty or healthy', () => {
    expect(usageSectionView({ state: initialQuota(false), now })).toEqual({ _tag: 'not-configured' })
    const missing = reduceQuota({ state: initialQuota(true), event: { _tag: 'Failed', at: now, failure: { kind: 'unavailable', reason: 'quota returned HTTP 404' } } })
    expect(usageSectionView({ state: missing, now })).toEqual({ _tag: 'not-configured' })
  })

  it('separates loading, a failed first read and an empty source', () => {
    expect(usageSectionView({ state: initialQuota(true), now })).toEqual({ _tag: 'loading' })
    const failed = reduceQuota({ state: initialQuota(true), event: { _tag: 'Failed', at: now, failure: { kind: 'timeout', reason: 'late' } } })
    expect(usageSectionView({ state: failed, now })).toEqual({ _tag: 'failed', kind: 'timeout' })
    expect(usageSectionView({ state: loaded([]), now })).toEqual({ _tag: 'empty', old: undefined })
  })

  it('sorts by the anchor as the terminal gov column does: exhausted, then used share, then no reading, then id', () => {
    const hiddenHigh = { ...withLimit('x/x', { fraction: 0.95, resetsIn: 3_600_000 }).limits[0]!, limitId: 'other', summarySlot: 'none' as const, summaryRole: 'ordinary' as const }
    const low = withLimit('a/low', { fraction: 0.1, resetsIn: 3_600_000 })
    const view = ready(loaded([
      { ...low, limits: [...low.limits, hiddenHigh] },
      withLimit('b/full', { fraction: 1, resetsIn: 3_600_000 }),
      withLimit('c/out', { fraction: 0.5, exhausted: true, resetsIn: 86_400_000 }),
      withLimit('d/mid', { fraction: 0.3, resetsIn: 3_600_000 }),
      withLimit('e/none', { fraction: null, resetsIn: 3_600_000 }),
      withLimit('f/mid', { fraction: 0.3, resetsIn: 3_600_000 }),
    ]))
    expect(view.rows.map(row => row.id)).toEqual(['c/out', 'b/full', 'd/mid', 'f/mid', 'a/low', 'e/none'])
  })

  it('uses the terminal tiers, 60 and 80, and calls a full reading or an exhausted state at limit', () => {
    const level = (fraction: number, exhausted = false) => ready(loaded([withLimit('a/a', { fraction, exhausted, resetsIn: 3_600_000 })])).worst
    expect(level(0.599).level).toBe('ok')
    expect(level(0.6).level).toBe('warn')
    expect(level(0.799).level).toBe('warn')
    expect(level(0.8).level).toBe('high')
    expect(level(1).level).toBe('limit')
    expect(level(1).words).toContain('100% used, at limit')
    expect(level(0.5, true)).toMatchObject({ level: 'limit', percent: '50%', fill: 100 })
    expect(level(0.996)).toMatchObject({ level: 'high', percent: '99%' })
  })

  it('counts at limit and running short, and gives the reset', () => {
    const view = ready(loaded([withLimit('a/low', { fraction: 0.1, resetsIn: 3_600_000 }), withLimit('b/full', { fraction: 1, exhausted: true, resetsIn: 86_400_000 }), withLimit('c/mid', { fraction: 0.8, resetsIn: 3_600_000 })]))
    expect(view.worst.reset).toBe('resets 1d')
    expect(view.atLimit).toBe(1)
    expect(view.running).toBe(1)
  })

  it('moves a window that has reset below readings and draws no meter for it', () => {
    const view = ready(loaded([withLimit('a/over', { fraction: 1, exhausted: true, resetsIn: -86_400_000 * 17 }), withLimit('b/now', { fraction: 0.2, resetsIn: 3_600_000 })]))
    expect(view.rows.map(row => row.id)).toEqual(['b/now', 'a/over'])
    expect(view.rows[1]).toMatchObject({ fill: undefined, percent: '?', level: 'unknown', reset: 'reset 17d ago' })
    expect(view.rows[1]!.words).toContain('no reading since its reset')
    expect(view.atLimit).toBe(0)
  })

  it('labels a producer-retained row and a retained snapshot old, keeps its place, and leaves it out of the at-limit count', () => {
    const retained = { ...withLimit('a/held', { fraction: 1, exhausted: true, resetsIn: 3_600_000 }), retained: true, valueObservedAt: new Date(now - 3 * 3_600_000).toISOString() }
    const view = ready(loaded([retained, withLimit('b/now', { fraction: 0.2, resetsIn: 3_600_000 })]))
    expect(view.rows.map(row => row.id)).toEqual(['a/held', 'b/now'])
    expect(view.rows[0]!.old).toBe('3h old')
    expect(view.atLimit).toBe(0)
    const failed = reduceQuota({ state: loaded([withLimit('b/now', { fraction: 0.2, resetsIn: 3_600_000 })]), event: { _tag: 'Failed', at: now, failure: { kind: 'http', reason: '500' } } })
    expect(ready(failed).old).toBe('40s old')
  })

  it('names unreadable limits rather than dropping the account, and counts unreported accounts', () => {
    const view = ready(loaded([incompatibleSummaryAccount]))
    expect(view.worst).toMatchObject({ percent: '?', reset: 'unreadable' })
    expect(view.unreported).toBe(1)
  })

  it('flags a blocking limit behind the +N', () => {
    const base = withLimit('a/mix', { fraction: 0.3, resetsIn: 3_600_000 })
    const hidden = { ...withLimit('x/x', { fraction: 1, exhausted: true, resetsIn: 3_600_000 }).limits[0]!, limitId: 'other', summarySlot: 'none' as const, summaryRole: 'ordinary' as const }
    const view = ready(loaded([{ ...base, limits: [...base.limits, hidden] }]))
    expect(view.worst.more).toEqual({ count: 1, atLimit: true })
    expect(view.worst.words).toContain('1 more limit, one at limit')
  })
})
