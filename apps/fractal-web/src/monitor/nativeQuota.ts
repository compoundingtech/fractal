/** Explicit presentation adapter for the canonical st3.client.v0 usage.period projection. */
import { UsagePeriod } from '@fractal/client/schema'
import type { UsageAccount, UsageLimit } from '@fractal/client/schema'
import { Result, Schema } from 'effect'

import type { QuotaFailure } from './model.ts'
import type { QuotaAccount, QuotaEnvelope, QuotaLimit } from './wire.ts'

/** Presentation freshness bound, matching the native default account-limits policy. */
export const NATIVE_LIMIT_FRESHNESS_MS = 60 * 60 * 1000
const NativeEnvelope = Schema.Struct({
  // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both:
  // sunset 60 days after the first release shipping the bridge; emitted provenance stays st3.
  api_version: Schema.Literals(['st3.client.v0', 'fractal.client.v0']),
  value: UsagePeriod,
})

const iso = (ms: number): string | undefined => {
  const date = new Date(ms)
  return Number.isFinite(date.getTime()) ? date.toISOString() : undefined
}

const limit = ({ reading, window, slot, now }: {
  readonly reading: UsageLimit
  readonly window: '5h' | 'weekly'
  readonly slot: 'pressure-1' | 'pressure-2'
  readonly now: number
}): QuotaLimit => {
  const percent = window === '5h' ? reading.five_hour_percent : reading.weekly_percent
  const resetMs = window === '5h' ? reading.five_hour_resets_at_unix_ms : reading.weekly_resets_at_unix_ms
  const fresh = now - reading.measured_at_unix_ms <= NATIVE_LIMIT_FRESHNESS_MS &&
    reading.measured_at_unix_ms <= now &&
    (resetMs === undefined || resetMs > now)
  const known = percent !== undefined && Number.isFinite(percent) && percent >= 0 && fresh
  return {
    limitId: window,
    label: window === '5h' ? '5-hour' : 'Weekly',
    shortLabel: window,
    // Fractal does not declare a governing limit or a ranked summary inventory.
    summaryRole: 'unknown',
    summarySlot: slot,
    scope: { class: 'account' },
    order: { source: 'native', position: slot === 'pressure-1' ? 0 : 1 },
    observation: percent === undefined ? { state: 'unreported' } : {
      state: 'reported',
      window: { id: window, label: window === '5h' ? '5-hour' : 'Weekly', kind: 'provider',
        ...(resetMs === undefined || iso(resetMs) === undefined ? {} : { resetsAt: iso(resetMs) }) },
      usage: {
        unit: 'fraction',
        ...(known ? { usedFraction: percent / 100 } : {}),
        ...(!fresh ? { rawStatus: 'stale, future-dated, or already reset observation' } : {}),
        exhaustion: { state: known ? (percent >= 100 ? 'exhausted' : 'available') : 'unknown',
          basis: known ? 'utilization' : 'unknown', conflict: false },
      },
    },
  }
}

export const projectNativeQuota = (input: unknown, now: number):
  | { readonly _tag: 'ok'; readonly value: QuotaEnvelope }
  | { readonly _tag: 'failed'; readonly failure: QuotaFailure } => {
  const decoded = Schema.decodeUnknownResult(NativeEnvelope)(input)
  if (Result.isFailure(decoded)) return { _tag: 'failed', failure: {
    kind: 'incompatible', reason: `incompatible Fractal usage.period: ${decoded.failure.message}`,
  } }
  const period = decoded.success.value
  if (period.limits === undefined) return { _tag: 'failed', failure: {
    kind: 'unavailable', reason: 'This Fractal usage.period response does not report account limits.',
  } }
  if (period.limits.some((reading) => iso(reading.measured_at_unix_ms) === undefined)) {
    return { _tag: 'failed', failure: { kind: 'incompatible', reason: 'Fractal usage.period observation time is not representable.' } }
  }
  const fromReading = (reading: UsageLimit, name?: string): QuotaAccount => {
    const observedAt = new Date(reading.measured_at_unix_ms).toISOString()
    const stale = now - reading.measured_at_unix_ms > NATIVE_LIMIT_FRESHNESS_MS ||
      reading.measured_at_unix_ms > now
    return {
      provider: reading.driver,
      accountId: name ?? reading.account_ref ?? reading.account,
      accountLabel: name ?? reading.account_ref ?? reading.account,
      // No ledgerAccountId: a declared/native account is not a per-request ledger identity.
      latestAttempt: { at: observedAt, outcome: 'observed',
        ...(stale ? { reason: 'source observation stale or future-dated' } : {}) },
      valueObservedAt: observedAt,
      retained: stale,
      source: { authority: 'st3.usage.period', confidence: reading.identified === false ? 'identity unknown' : 'harness observation' },
      ...(reading.plan === undefined ? {} : { plan: { id: reading.plan, displayName: reading.plan, authority: 'harness observation' } }),
      limits: [limit({ reading, window: 'weekly', slot: 'pressure-1', now }),
        limit({ reading, window: '5h', slot: 'pressure-2', now })],
      // The period's default 24h spend is not seven-day spend or per-account ledger history.
      usage7d: { basis: 'not reported by Fractal usage.period' },
    }
  }
  // An api-key account has no provider limit to read: its budget is what it may spend this month.
  const fromCredits = (declared: UsageAccount, credits: NonNullable<UsageAccount['credits']>): QuotaAccount => {
    const used = credits.used_microusd / 1_000_000
    const budget = credits.budget_microusd / 1_000_000
    const start = new Date(credits.period_start_ms)
    const resets = new Date(Date.UTC(start.getUTCFullYear(), start.getUTCMonth() + 1, 1)).toISOString()
    const observedAt = new Date(now).toISOString()
    const usedFraction = budget > 0 ? used / budget : undefined
    return {
      provider: declared.provider,
      accountId: declared.name,
      accountLabel: declared.name,
      latestAttempt: { at: observedAt, outcome: 'observed' },
      valueObservedAt: observedAt,
      retained: false,
      source: { authority: 'st3.usage.period', confidence: 'api-equivalent list prices' },
      limits: [{
        limitId: 'monthly-credits',
        label: 'Credits this month',
        shortLabel: 'month',
        summaryRole: 'unknown',
        summarySlot: 'pressure-1',
        scope: { class: 'account' },
        order: { source: 'native', position: 0 },
        observation: {
          state: 'reported',
          window: { id: 'month', label: 'Credits this month', kind: 'calendar', resetsAt: resets },
          usage: {
            unit: 'usd', used, limit: budget, remaining: Math.max(0, budget - used),
            ...(usedFraction === undefined ? {} : { usedFraction }),
            ...(credits.unpriced_tokens > 0 ? { rawStatus: 'some tokens had no price, so the spend is at least this' } : {}),
            exhaustion: { state: usedFraction === undefined ? 'unknown' : usedFraction >= 1 ? 'exhausted' : 'available',
              basis: usedFraction === undefined ? 'unknown' : 'utilization', conflict: false },
          },
        },
      }],
      usage7d: { basis: 'not reported by st usage.period' },
    }
  }
  // Declared accounts first, in name order, whether or not they reported; then any reading no
  // declaration claims, so nothing st recorded is dropped.
  const declared = period.accounts ?? []
  const claimed = new Set<UsageLimit>()
  const listed: QuotaAccount[] = declared.map((account) => {
    const reading = period.limits?.find((candidate) => candidate.account === account.account)
    if (reading !== undefined) {
      claimed.add(reading)
      return fromReading(reading, account.name)
    }
    if (account.credits !== undefined) return fromCredits(account, account.credits)
    const observedAt = new Date(now).toISOString()
    return {
      provider: account.driver ?? account.provider,
      accountId: account.name,
      accountLabel: account.name,
      latestAttempt: { at: observedAt, outcome: 'unreported' },
      valueObservedAt: observedAt,
      retained: true,
      source: { authority: 'st3.usage.period', confidence: 'no harness observation' },
      limits: [],
      usage7d: { basis: 'not reported by st usage.period' },
    }
  })
  const accounts: QuotaAccount[] = [
    ...listed,
    ...period.limits.filter((reading) => !claimed.has(reading)).map((reading) => fromReading(reading)),
  ]
  return { _tag: 'ok', value: {
    schemaVersion: 3,
    sourceSchema: 'st3.client.v0/usage.period',
    generatedAt: new Date(now).toISOString(),
    freshnessHorizonSeconds: 30,
    coverage: { reportedAccounts: accounts.length,
      unreportedAccountIds: [], disabledAccountIds: [] },
    accounts,
  } }
}
