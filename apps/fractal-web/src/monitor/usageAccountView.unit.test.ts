import { describe, expect, it } from 'vitest'

import { accountOf, accounts, FIXTURE_NOW, historyEnvelope, incompatibleSummaryAccount, quotaEnvelope, type HistorySpec } from './fixtures.ts'
import { initialHistory, initialQuota, projectHistory, projectRows, reduceQuota, type HistoryState, type HistoryUnit } from './model.ts'
import { accountDetailView, historyView, selectedRow } from './usageAccountView.ts'
import { usageSectionView } from './usageSectionView.ts'
import type { QuotaAccount } from './wire.ts'

const now = FIXTURE_NOW
const DAY = 86_400_000
const detail = (account: QuotaAccount) => accountDetailView({ row: projectRows(quotaEnvelope({ list: [account] }))[0]!, now })
const mixed = accountOf({
  harness: 'codex', provider: 'openai', name: 'mixed', label: 'Mixed', plan: 'Pro',
  limits: [
    { id: 'week', label: 'Weekly', slot: 'governing', fraction: 1, exhausted: true, resetsIn: 2 * DAY },
    { id: 'five', label: '5 Hour', slot: 'pressure-1', fraction: 0.62, resetsIn: 3_600_000 },
    { id: 'month', label: 'Monthly', slot: 'none', fraction: 0.4, resetsIn: -3 * DAY },
    { id: 'gb', label: 'Build', slot: 'none', unreported: true },
  ],
})

describe('usage account detail', () => {
  it('puts every limit on its own line with the section tiers, an at-limit or unreported state and the reset', () => {
    expect(detail(mixed).limits).toMatchObject([
      { label: 'Weekly', percent: '100%', level: 'limit', note: 'at limit', reset: 'resets 2d', fill: 100 },
      { label: '5 Hour', percent: '62%', level: 'warn', note: undefined, reset: 'resets 1h' },
      { label: 'Monthly', percent: '?', level: 'unknown', note: 'no reading since its reset', reset: 'reset 3d ago', fill: undefined },
      { label: 'Build', percent: 'unreported', level: 'unknown', note: 'not reported', reset: 'reset ?' },
    ])
  })

  it('shows banked resets with their expiries, a reported zero, and nothing when the producer is silent', () => {
    expect(detail(accounts.claudeAlpha).banked).toBe('2 banked resets · expires in 21d · 1 undated')
    expect(detail(accounts.claudeBravo).banked).toBe('No banked resets')
    expect(detail(mixed).banked).toBeUndefined()
  })

  it('gives the reading age and plan, and the last attempt only when it did not read a value', () => {
    expect(detail(mixed)).toMatchObject({ plan: 'Pro', reading: { age: '42s old', held: false }, attempt: undefined })
    const held = detail(accountOf({ harness: 'codex', provider: 'openai', name: 'held', label: 'Held', retained: { observedAgo: 3 * 3_600_000, reason: 'provider returned 401' }, limits: [{ id: 'w', label: 'Weekly', slot: 'governing', fraction: 0.5 }] }))
    expect(held).toMatchObject({ plan: undefined, reading: { age: '3h old', held: true }, attempt: 'indeterminate 40s ago: provider returned 401' })
  })

  it('keeps the limits of an account whose summary is unreadable', () => {
    const unreadable = detail(incompatibleSummaryAccount)
    expect(unreadable.unreadable).toBe('duplicate summary slot pressure-1')
    expect(unreadable.limits.map(limit => limit.percent)).toEqual(['20%', '40%'])
  })

  it('shows the selected account, else the first in the list, never an absent one', () => {
    const state = reduceQuota({ state: initialQuota(true), event: { _tag: 'Succeeded', at: now, envelope: quotaEnvelope({ list: [accounts.claudeAlpha, mixed] }) } })
    const view = usageSectionView({ state, now })
    if (view._tag !== 'ready') throw new Error('expected ready')
    expect(selectedRow({ state, view, selected: 'claude/alpha' })?.accountId).toBe('claude/alpha')
    expect(selectedRow({ state, view, selected: undefined })?.accountId).toBe(view.rows[0]!.id)
    expect(selectedRow({ state, view, selected: 'gone/away' })?.accountId).toBe('codex/mixed')
  })
})

const ledger = 'openai/alpha'
const seriesOf = (spec: Partial<HistorySpec> = {}): HistoryState['last'] => {
  const series = projectHistory({ envelope: historyEnvelope({ ledgerAccountId: ledger, days: 45, ...spec }), ledgerAccountId: ledger })
  if (series._tag !== 'series') throw new Error(series.reason)
  return { series: series.series, at: now }
}
const history = (state: Partial<HistoryState>, preferred: HistoryUnit = 'usd') =>
  historyView({ ledgerAccountId: ledger, history: { ...initialHistory, ...state }, preferred })

describe('usage account history', () => {
  it('names each state before a series exists', () => {
    expect(historyView({ ledgerAccountId: undefined, history: initialHistory, preferred: 'usd' })).toEqual({ _tag: 'not-configured' })
    expect(history({ attempt: { _tag: 'inFlight' } })).toEqual({ _tag: 'loading' })
    expect(history({ attempt: { _tag: 'failed', kind: 'unavailable', reason: 'history returned HTTP 404' } })).toEqual({ _tag: 'not-configured' })
    expect(history({ attempt: { _tag: 'failed', kind: 'incompatible', reason: 'bad rows' } })).toEqual({ _tag: 'incompatible', reason: 'bad rows' })
    expect(history({ attempt: { _tag: 'failed', kind: 'http', reason: 'history returned HTTP 502' } })).toEqual({ _tag: 'unavailable', reason: 'history returned HTTP 502' })
  })

  it('charts the last 30 days in USD, the open day last and partial', () => {
    const view = history({ last: seriesOf({ idle: [3] }), attempt: { _tag: 'ok' } })
    if (view._tag !== 'chart') throw new Error('expected chart')
    expect(view.caption).toBe('Daily spend, USD · last 30 days')
    expect(view.bars).toHaveLength(30)
    expect(view.bars.filter(bar => bar.open)).toEqual([view.bars.at(-1)])
    expect(view.bars.at(-1)!.words).toMatch(/so far: \$\d+\.\d\d$/)
    expect(view.bars.at(-4)!).toMatchObject({ state: 'zero', fill: 0, words: expect.stringMatching(/: \$0\.00$/) })
    expect(Math.max(...view.bars.map(bar => bar.fill ?? 0))).toBe(100)
    expect(view.summary).toMatch(/^\$\d+ over 29 full days · peak \$\d+\.\d\d on \d{4}-\d\d-\d\d · today so far \$\d+\.\d\d$/)
    expect(view.notes).toEqual([])
  })

  it('falls back to tokens below coverage and keeps the last series through a failed refresh', () => {
    const view = history({ last: seriesOf({ coverageOk: false, omittedBuckets: 4 }), attempt: { _tag: 'failed', kind: 'timeout', reason: 'history timed out' } })
    if (view._tag !== 'chart') throw new Error('expected chart')
    expect(view.caption).toBe('Daily tokens · last 30 days')
    expect(view.notes).toEqual([
      'USD coverage is below the producer’s gate, so this shows tokens',
      'The producer dropped 4 older days',
      'Refresh failed: history timed out; showing the last good reading',
    ])
    expect(history({ last: seriesOf(), attempt: { _tag: 'ok' } }, 'tokens')).toMatchObject({ caption: 'Daily tokens · last 30 days', notes: [] })
  })

  it('spans 30 calendar days back from the open day, marking days before the series as not reported', () => {
    const view = history({ last: seriesOf({ days: 3 }), attempt: { _tag: 'ok' } })
    if (view._tag !== 'chart') throw new Error('expected chart')
    expect(view.bars).toHaveLength(30)
    expect(view.bars.at(-1)!).toMatchObject({ day: '2026-10-01', open: true })
    expect(view.bars[0]!).toMatchObject({ day: '2026-09-02', state: 'unreported', fill: undefined, words: '2026-09-02: not reported', value: undefined, status: 'unreported' })
    expect(view.bars.at(-1)!).toMatchObject({ value: expect.stringMatching(/^\$\d+\.\d\d$/), status: 'partial' })
    expect(view.unit).toBe('USD')
    expect(view.bars.map(bar => bar.state === 'unreported')).toEqual([...Array(27).fill(true), false, false, false])
    expect(view.summary).toMatch(/^\$\d+\.\d\d over 2 full days · 27 days not reported · peak /)
  })

  const unrated = (usd: ReadonlyArray<number | undefined>): HistoryState['last'] => {
    const days = usd.map((value, index) => ({ day: `2026-09-0${index + 1}`, tokens: 1000, usd: value, open: index === usd.length - 1 }))
    return { series: { days, openDay: days.at(-1)!.day, coverageOk: true, omittedDays: 0, computedAt: '2026-09-04T12:00:00Z' }, at: now }
  }

  it('sums only rated days and says how many lack a rate', () => {
    const view = history({ last: unrated([10, undefined, 5, 2]), attempt: { _tag: 'ok' } })
    if (view._tag !== 'chart') throw new Error('expected chart')
    expect(view.summary).toBe('$15.00 rated subtotal over 2 full days · 1 without a rate · 26 days not reported · peak $10.00 on 2026-09-01 · today so far $2.00')
    expect(view.bars.at(-3)!).toMatchObject({ state: 'unrated', fill: undefined, words: '2026-09-02: no rate', value: undefined, status: 'unrated' })
    expect(view.bars.at(-2)!).toMatchObject({ value: '$5.00', status: 'recorded' })
  })

  it('shows no USD total when no day is rated', () => {
    expect(history({ last: unrated([undefined, undefined, undefined]), attempt: { _tag: 'ok' } })).toMatchObject({
      summary: 'no rate for 2 full days · 27 days not reported · today so far no rate',
    })
  })
})
