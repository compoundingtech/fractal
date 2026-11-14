import { describe, expect, it } from 'vitest'

import { cellState } from './model.ts'
import { NATIVE_LIMIT_FRESHNESS_MS, projectNativeQuota } from './nativeQuota.ts'
import { decodeQuota } from './wire.ts'
import { quotaEnvelope } from './fixtures.ts'

const NOW = Date.parse('2026-10-07T12:00:00Z')
const reading = {
  account: 'claude/account-observation', account_ref: 'account/ada/claude-1', driver: 'claude',
  host: 'member-a', measured_by: 'agent/ada/test', measured_at_unix_ms: NOW - 1000,
  seats: ['agent/ada/test'], identified: true,
  five_hour_percent: 0, weekly_percent: 97,
  weekly_resets_at_unix_ms: NOW + 1000,
}
const envelope = (limits: unknown = [reading]) => ({
  api_version: 'st3.client.v0', value: { since_ms: NOW - 86400000, until_ms: NOW, rows: [],
    ...(limits === undefined ? {} : { limits }) },
})
const account = (input: unknown) => {
  const result = projectNativeQuota(input, NOW)
  if (result._tag !== 'ok') throw new Error(result.failure.reason)
  const row = result.value.accounts[0]
  if (row === undefined) throw new Error('Missing account')
  return row
}

describe('native usage.period quota presentation', () => {
  it('accepts both wire version names but rejects unknown or conflicting versions', () => {
    for (const api_version of ['st3.client.v0', 'fractal.client.v0']) {
      expect(projectNativeQuota({ ...envelope(), api_version }, NOW)).toMatchObject({ _tag: 'ok' })
    }
    for (const api_version of ['fractal.client.v1', 'smalltalk.client.v0', ['st3.client.v0', 'fractal.client.v0']]) {
      expect(projectNativeQuota({ ...envelope(), api_version }, NOW))
        .toMatchObject({ _tag: 'failed', failure: { kind: 'incompatible' } })
    }
  })
  it('accepts only the two usage.period provenance spellings without relaxing ledger checks', () => {
    for (const sourceSchema of ['st3.client.v0/usage.period', 'fractal.client.v0/usage.period']) {
      expect(decodeQuota({ ...quotaEnvelope(), sourceSchema })._tag).toBe('ok')
      expect(decodeQuota({ ...quotaEnvelope(), accounts: [{ ...quotaEnvelope().accounts[0], ledgerAccountId: undefined }], sourceSchema })._tag).toBe('incompatible')
    }
    for (const sourceSchema of ['smalltalk.client.v0/usage.period', 'fractal.client.v1/usage.period', 'fractal.client.v0/usage.other']) {
      expect(decodeQuota({ ...quotaEnvelope(), sourceSchema })._tag).toBe('incompatible')
    }
  })
  it('keeps locally emitted provenance in the old spelling for either input spelling', () => {
    for (const api_version of ['st3.client.v0', 'fractal.client.v0']) {
      expect(projectNativeQuota({ ...envelope(), api_version }, NOW))
        .toMatchObject({ _tag: 'ok', value: { sourceSchema: 'st3.client.v0/usage.period' } })
    }
  })
  it('preserves native percentages including reported zero, identity and original measurement', () => {
    const row = account(envelope())
    expect(row.accountId).toBe(reading.account_ref)
    expect(row.valueObservedAt).toBe(new Date(reading.measured_at_unix_ms).toISOString())
    expect(row.limits.map(cellState)).toEqual([
      { _tag: 'fraction', percent: 97, exhausted: false },
      { _tag: 'fraction', percent: 0, exhausted: false },
    ])
    expect(row.ledgerAccountId).toBeUndefined()
    const native = projectNativeQuota(envelope(), NOW)
    if (native._tag !== 'ok') throw new Error(native.failure.reason)
    expect(native.value.coverage.expectedAccounts).toBeUndefined()
    expect(row.usage7d.usd).toBeUndefined()
    expect(row.limits.every((limit) => limit.summaryRole === 'unknown')).toBe(true)
    const projected = projectNativeQuota(envelope(), NOW)
    if (projected._tag !== 'ok') throw new Error(projected.failure.reason)
    expect(decodeQuota(projected.value, { nativeProjection: true })._tag).toBe('ok')
  })
  it('lists every declared account, an api-key account by its credits, and keeps undeclared readings', () => {
    const declared = [
      { name: 'ada/claude-1', account: reading.account, driver: 'claude', provider: 'anthropic', kind: 'subscription' },
      { name: 'ada/claude-2', account: 'claude/not-reported', driver: 'claude', provider: 'anthropic', kind: 'subscription' },
      { name: 'ada/key', account: 'omp/key', driver: 'omp', provider: 'anthropic', kind: 'api-key',
        credits: { budget_microusd: 200_000_000, used_microusd: 154_000_000, unpriced_tokens: 0,
          period_start_ms: Date.parse('2026-10-01T00:00:00Z') } },
    ]
    const stranger = { ...reading, account: 'claude/stranger', account_ref: null }
    const input = envelope([reading, stranger])
    const native = projectNativeQuota({ ...input, value: { ...input.value, accounts: declared } }, NOW)
    if (native._tag !== 'ok') throw new Error(native.failure.reason)
    expect(native.value.accounts.map((row) => row.accountId)).toEqual(['ada/claude-1', 'ada/claude-2', 'ada/key', 'claude/stranger'])
    const [first, second, key] = native.value.accounts
    expect(first?.limits.map(cellState)).toEqual([
      { _tag: 'fraction', percent: 97, exhausted: false },
      { _tag: 'fraction', percent: 0, exhausted: false },
    ])
    expect(second?.limits).toEqual([])
    expect(key?.limits.map(cellState)).toEqual([{ _tag: 'fraction', percent: 77, exhausted: false }])
    expect(key?.limits[0]?.observation).toMatchObject({ state: 'reported', window: { resetsAt: '2026-11-01T00:00:00.000Z' },
      usage: { unit: 'usd', used: 154, limit: 200 } })
    expect(decodeQuota(native.value, { nativeProjection: true })._tag).toBe('ok')
  })
  it('leaves missing percentages unreported rather than zero', () => {
    const { five_hour_percent: _fiveHour, weekly_percent: _weekly, ...withoutPercentages } = reading
    const row = account(envelope([withoutPercentages]))
    expect(row.limits.map(cellState)).toEqual([{ _tag: 'unreported' }, { _tag: 'unreported' }])
  })
  it('distinguishes a missing limits capability from an explicitly empty observation list', () => {
    expect(projectNativeQuota({ api_version: 'st3.client.v0', value: { since_ms: 0, until_ms: NOW, rows: [] } }, NOW))
      .toMatchObject({ _tag: 'failed', failure: { kind: 'unavailable' } })
    expect(projectNativeQuota(envelope([]), NOW)).toMatchObject({ _tag: 'ok', value: { accounts: [] } })
  })
  it('marks stale and future-dated source values unknown without changing their source time', () => {
    for (const measured of [NOW - NATIVE_LIMIT_FRESHNESS_MS - 1, NOW + 1, NOW + 60001]) {
      const row = account(envelope([{ ...reading, measured_at_unix_ms: measured }]))
      expect(row.limits.map(cellState)).toEqual([{ _tag: 'unknown' }, { _tag: 'unknown' }])
      expect(row.retained).toBe(true)
      expect(row.valueObservedAt).toBe(new Date(measured).toISOString())
    }
  })
  it('treats an already reset weekly window as unknown independently of the 5h value', () => {
    const row = account(envelope([{ ...reading, weekly_resets_at_unix_ms: NOW }]))
    expect(row.limits.map(cellState)).toEqual([{ _tag: 'unknown' }, { _tag: 'fraction', percent: 0, exhausted: false }])
  })
  it('rejects malformed native envelopes and unrepresentable dates', () => {
    for (const input of [{}, { ...envelope(), api_version: 'future' }, envelope([{ ...reading, measured_at_unix_ms: 1e20 }])]) {
      expect(projectNativeQuota(input, NOW)).toMatchObject({ _tag: 'failed', failure: { kind: 'incompatible' } })
    }
  })
  it('still requires genuine ledger identities in the usage v3 contract', () => {
    const fixture = quotaEnvelope()
    const withoutLedger = { ...fixture, accounts: fixture.accounts.map(({ ledgerAccountId: _id, ...row }) => row) }
    expect(decodeQuota(withoutLedger)._tag).toBe('incompatible')
    expect(decodeQuota({ ...withoutLedger, sourceSchema: 'st3.client.v0/usage.period' })._tag).toBe('incompatible')
    expect(decodeQuota(fixture)._tag).toBe('ok')
  })
})
