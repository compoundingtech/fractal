import { describe, expect, it } from 'vitest'

import { accountOf, accounts, FIXTURE_NOW, incompatibleSummaryAccount, quotaEnvelope } from './fixtures.ts'
import { initialQuota, projectRows, reduceQuota } from './model.ts'
import { accountDetailView, selectedRow } from './usageAccountView.ts'
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
