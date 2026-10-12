import type { QuotaAccount, QuotaEconomics, QuotaHistoryDay, QuotaStatus, QuotaWindow } from './quota-design-types.ts'

/** Neutral fictional presentation fixtures. No producer envelopes or real identities.
 * Semantics follow the private v3 conformance cases: producer slots, unknown fractions,
 * explicit exhaustion, zero/absent/truncated reset counts and retained evidence.
 * All rates, identities, amounts and dates below are synthetic, not data-owner assumptions.
 * Confirmed history: Berlin buckets, five nullable token types, USD coverage >= .95.
 * Confirmed economics: last 30 days normalized, not calendar-month billing.
 * Pending publication timestamps and host grouping are intentionally not fabricated.
 * Remaining boundary awaiting owner: exact economics transport envelope and
 * invoice-amount serialization; private adapter accepts explicit typed inputs.
 */
export const quotaFixtureAssumptions = [
  'Exact economics transport envelope and invoice-amount serialization await the data owner; no public wire decoder is assumed.',
] as const
export const quotaClock = '2030-04-30T12:00:00Z'
export const quotaColumns = [{ id: 'short', label: '5h' }, { id: 'week', label: 'Week' }, { id: 'model', label: 'Model' }] as const
const windowOf = (id: string, column: string, fraction: number | undefined, governing: boolean, comparison: boolean, extra: Partial<QuotaWindow> = {}): QuotaWindow => ({
  id, column, label: column === 'short' ? '5 hour rolling' : column === 'week' ? 'Weekly allowance' : 'Model allowance',
  ...(fraction === undefined ? {} : { fraction }), exhausted: fraction !== undefined && fraction >= 1,
  reported: true, governing, comparison, resetAt: column === 'short' ? '2030-04-30T16:00:00Z' : '2030-05-03T12:00:00Z',
  scope: column === 'model' ? 'model · Cedar' : 'account', facts: [
    { label: 'Window kind', value: 'Rolling' }, { label: 'Evidence', value: 'Observed · synthetic fixture' },
    { label: 'Utilization', value: fraction === undefined ? 'Unreported' : `${Math.round(fraction * 100)}%` },
  ], ...extra,
})
const accountOf = (id: string, label: string, provider: string, fraction: number | undefined, index: number): QuotaAccount => ({
  id, label, provider, plan: index % 3 === 0 ? 'Team' : 'Personal', state: 'current',
  windows: [windowOf(`${id}-week`, 'week', fraction, true, false),
    ...(provider === 'Orbit' ? [] : [windowOf(`${id}-short`, 'short', ((index * 17) % 90) / 100, false, true)]),
    ...(index % 2 === 0 ? [windowOf(`${id}-model`, 'model', ((index * 11) % 70) / 100, false, provider === 'Orbit')] : []),
  ], usd: 14.6 + index * 9.12, tokens: 480000 + index * 175000, usdCoverage: index !== 4,
  ...(index % 3 === 0 ? { resetCount: index === 0 ? 0 : 3 } : {}),
  resetCredits: index % 3 === 0 ? [{ label: 'Reported available resets', value: index === 0 ? '0' : '3' }, ...(index === 0 ? [] : [{ label: 'Credit 1 expiry', value: '2030-05-02T12:00:00Z' }, { label: 'Undated or unsupplied slots', value: '2' }])] : [{ label: 'Available resets', value: 'Unreported' }],
  facts: [{ label: 'Identity', value: id }, { label: 'Provider', value: provider }, { label: 'Plan', value: index % 3 === 0 ? 'Team' : 'Personal' }, { label: 'Value observed', value: '2030-04-30T11:59:00Z' }, { label: 'Latest collection', value: 'Determinate · 2030-04-30T12:00:00Z' }, { label: 'Source confidence', value: 'Observed' }],
})
export const quotaAccounts: readonly QuotaAccount[] = [
  { ...accountOf('harbor', 'Harbor', 'Orbit', undefined, 0), windows: [windowOf('harbor-week', 'week', undefined, true, false, { exhausted: true, facts: [{ label: 'Exhaustion', value: 'Provider status · fraction unknown' }] })] },
  accountOf('north', 'North studio', 'Cedar', .83, 1),
  accountOf('delta', 'Delta lab', 'Cedar', .60, 2),
  accountOf('birch', 'Birch team', 'Orbit', .60, 3),
  accountOf('meadow', 'Meadow', 'Cedar', .58, 4),
  accountOf('lake', 'Lake studio', 'Orbit', .19, 5),
  { ...accountOf('elm', 'Elm sandbox', 'Cedar', .14, 6), state: 'stale', facts: [{ label: 'Retained evidence', value: 'Latest collection failed; value observed 2030-04-29T18:00:00Z' }] },
  accountOf('cove', 'Cove', 'Orbit', .08, 7),
  { ...accountOf('unknown', 'Unreported seat', 'Cedar', undefined, 8), state: 'unknown', usd: undefined, tokens: undefined, usdCoverage: false, windows: [windowOf('unknown-week', 'week', undefined, true, false, { reported: false, resetAt: undefined, facts: [{ label: 'Observation', value: 'Declared but unreported' }] })] },
  accountOf('spruce', 'Spruce', 'Prism', .12, 9),
  accountOf('willow', 'Willow', 'Prism', .03, 10),
]
export const quotaDays: readonly QuotaHistoryDay[] = Array.from({ length: 30 }, (_, index) => {
  const id = `2030-04-${String(index + 1).padStart(2, '0')}`
  return { id, label: `${String(index + 1).padStart(2, '0')} Apr`, partial: index === 29,
    values: quotaAccounts.slice(0, 6).flatMap((account, accountIndex) => ['Cedar', 'Pine'].map((model, modelIndex) => {
      const missing = index === 11 && accountIndex === 4
      const tokens = index % 7 === 5 ? 0 : 20000 + ((index * 3757 + accountIndex * 17111 + modelIndex * 7231) % 95000)
      const input = Math.round(tokens * .3), read = Math.round(tokens * .4), write = Math.round(tokens * .05), output = Math.round(tokens * .2)
      return { id: `${account.id}-${model}`, label: account.label, accountId: account.id, model, scope: modelIndex === 0 ? 'shared account' : 'model-specific',
        ...(missing ? {} : { tokens, usd: tokens / 50000 }), coverage: accountIndex === 4 ? .91 : .99,
        tokenTypes: [{ label: 'Input', value: missing ? undefined : input }, { label: 'Cache read', value: missing ? undefined : read }, { label: 'Cache write', value: missing ? undefined : write }, { label: 'Output', value: missing ? undefined : output }, { label: 'Reasoning', value: missing ? undefined : tokens - input - read - write - output }],
        requests: missing ? undefined : Math.round(tokens / 4000), turns: missing ? undefined : Math.round(tokens / 6000),
      }
    })) }
})
/** One low-coverage day surrounded by admitted records exposes stale controlled day selections. */
export const quotaCoverageDays: readonly QuotaHistoryDay[] = quotaDays.map(day => ({
  ...day,
  values: day.values.map(value => ({ ...value, coverage: day.id === '2030-04-15' ? .50 : .99 })),
}))
export const quotaEconomics: readonly QuotaEconomics[] = quotaAccounts.slice(0, 6).map((account, index) => ({
  id: `${account.id}-economics`, label: account.label, accountId: account.id, limitId: `${account.id}-week`, usageValue: 54 + index * 31,
  ledgerValue: index === 4 ? undefined : 52 + index * 29, price: index % 3 === 0 ? 120 : 24, basis: index === 2 ? 'list' : 'invoiced', coverageDays: index === 4 ? 22 : 30,
}))
export const quotaStatuses: Readonly<Record<QuotaStatus['kind'], QuotaStatus>> = {
  current: { kind: 'current', text: 'Updated 1m ago · observed evidence' },
  loading: { kind: 'loading', text: 'Refreshing · showing last good data' },
  stale: { kind: 'stale', text: 'Stale · retained values from last good collection' },
  failed: { kind: 'failed', text: 'Collection failed · last good data retained; retry available' },
  incompatible: { kind: 'incompatible', text: 'Source version incompatible · last good data retained' },
  unpublished: { kind: 'unpublished', text: 'Not yet published · no history generation available' },
}
export const quotaRefreshedAccounts: readonly QuotaAccount[] = quotaAccounts.map(account => ({ ...account, windows: account.windows.map(window => ({ ...window, ...(window.fraction === undefined ? {} : { fraction: Math.min(1, window.fraction + .01) }) })) }))
