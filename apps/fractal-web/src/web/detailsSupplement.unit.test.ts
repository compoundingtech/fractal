import { describe, expect, it } from 'vitest'
import type { DetailsOverview, TraceSeries, TraceFact } from '@smalltalk/fractal-ui/assistant-ui'
import { detailsWithSupplement } from './detailsSupplement.ts'
import type { DetailsSupplement } from '../extensions/contract.ts'
import { observed, unavailable } from '../data/source.ts'

const known = <A>(value: A): TraceFact<A> => ({ _tag: 'Known', value, source: 'st', asOfMs: 2_000 })
const missing = { _tag: 'Unknown', reason: 'no-provider' } as const
const overview: DetailsOverview = { title: 'Synthetic seat', status: 'Working', machine: 'Synthetic host', harness: 'omp', model: undefined, since: undefined, age: '5m', durations: [{ label: 'Managed session elapsed · wall time', value: '3m' }] }
const trace = (): TraceSeries => ({
  agentId: 'agent/synthetic', nativeSessionId: known('00000000-0000-4000-8000-000000000001'),
  meters: {
    context: { reportedPercent: known(6.1206), usedTokens: known(61_206), windowTokens: known(1_000_000), compactions: missing, lastCompactionMs: missing },
    spend: {
      usdScope: { _tag: 'Unknown', reason: 'not-attributed' }, usdWindow: missing, totalUsdCoverage: { _tag: 'Unavailable', reason: 'no-provider' }, totalUsd: missing,
      mainUsdScope: { _tag: 'Self' }, mainUsdWindow: known({ _tag: 'Interval', fromMs: 1_000, toMs: 2_000 }), mainUsdCoverage: { _tag: 'Complete' }, mainUsd: known(1),
      subagentUsdScope: { _tag: 'Unknown', reason: 'not-attributed' }, subagentUsdWindow: missing, subagentUsdCoverage: { _tag: 'Unavailable', reason: 'no-provider' }, subagentUsd: missing,
      tokensScope: { _tag: 'Self' }, tokensWindow: known({ _tag: 'Interval', fromMs: 1_000, toMs: 2_000 }), tokens: known(480_035_214), split: missing,
    },
  },
  coverage: { _tag: 'Unavailable', reason: 'no-provider' }, series: missing, turns: missing,
  subagents: missing, subagentsWindow: missing, subagentCount: missing, history: missing,
})
const supplement = (): DetailsSupplement => ({
  nativeSessionId: '00000000-0000-4000-8000-000000000001', totalUsd: { _tag: 'Known', value: 185.5, source: 'series' },
  usdWindow: { _tag: 'Known', value: { _tag: 'Reported', label: '7d' }, source: 'series' },
  coverage: { _tag: 'Partial', attributedFraction: 0.995 }, subagents: { _tag: 'Known', value: [], source: 'series' },
  subagentsWindow: { _tag: 'Known', value: { _tag: 'Reported', label: '7d' }, source: 'series' }, subagentCount: { _tag: 'Known', value: 66, source: 'series' },
  durations: [{ label: 'Root ledger wall span · 7d as reported', value: '2h' }],
})
const join = (native: TraceSeries, value: DetailsSupplement) => detailsWithSupplement({ trace: native, overview, supplement: observed({ value }), harness: 'omp', nativeSessionId: '00000000-0000-4000-8000-000000000001' })

describe('independent Details metric composition', () => {
  it('takes the inclusive producer amount without summing children or overwriting native accounting', () => {
    const native = trace()
    const result = join(native, supplement())
    expect(result.trace.meters.spend.totalUsd).toEqual({ _tag: 'Known', value: 185.5, source: 'series' })
    expect(result.trace.meters.spend.usdScope).toEqual({ _tag: 'IncludingSubagents' })
    expect(result.trace.meters.spend.usdWindow).toEqual(supplement().usdWindow)
    expect(result.trace.meters.spend.mainUsd).toBe(native.meters.spend.mainUsd)
    expect(result.trace.meters.spend.mainUsdWindow).toBe(native.meters.spend.mainUsdWindow)
    expect(result.trace.meters.spend.tokens).toBe(native.meters.spend.tokens)
    expect(result.trace.meters.spend.tokensWindow).toBe(native.meters.spend.tokensWindow)
    expect(result.trace.meters.spend.subagentUsd).toBe(native.meters.spend.subagentUsd)
    expect(result.trace.meters.context).toBe(native.meters.context)
    expect(result.trace.subagentCount).toEqual(supplement().subagentCount)
    expect(result.trace.subagentsWindow).toEqual(supplement().subagentsWindow)
    expect(result.overview.durations?.map(item => item.label)).toEqual(['Managed session elapsed · wall time', 'Root ledger wall span · 7d as reported'])
    // A cumulative-token substitution must fail the same occupancy criterion.
    expect(() => expect({ ...result.trace.meters.context, reportedPercent: known(480_035_214 / 1_000_000 * 100) }).toEqual(native.meters.context)).toThrow()
  })
  it('never crosses roots, even when the observed supplement is retained', () => {
    const native = trace()
    const other = { ...supplement(), nativeSessionId: '00000000-0000-4000-8000-000000000002' }
    expect(join(native, other).trace).toBe(native)
    expect(detailsWithSupplement({ trace: native, overview, supplement: observed({ value: supplement(), freshness: 'stale' }), harness: 'omp' }).trace).toBe(native)
  })
  it('does not substitute OMP coverage for Claude Code or Codex native facts', () => {
    for (const harness of ['claude', 'codex']) {
      const native = trace()
      const result = detailsWithSupplement({ trace: native, overview, supplement: observed({ value: supplement() }), harness, nativeSessionId: supplement().nativeSessionId })
      expect(result.trace.coverage).toEqual({ _tag: 'NotCovered', harnessName: harness === 'claude' ? 'Claude Code' : 'Codex' })
      expect(result.trace.meters.spend.totalUsd).toBe(native.meters.spend.totalUsd)
      expect(result.trace.meters.spend.mainUsd).toBe(native.meters.spend.mainUsd)
      expect(result.trace.subagents).toEqual({ _tag: 'Unknown', reason: 'not-covered' })
    }
  })
  it('keeps matching stale financial facts visible without reconciling them with native self spend', () => {
    const native = trace()
    const value = { ...supplement(), receivedAtMs: 1_500 }
    const retained = observed({ value, freshness: 'stale' })
    const result = detailsWithSupplement({ trace: native, overview, supplement: retained, harness: 'omp', nativeSessionId: value.nativeSessionId })
    expect(result.trace.meters.spend.totalUsd).toBe(value.totalUsd)
    expect(result.trace.meters.spend.mainUsd).toBe(native.meters.spend.mainUsd)
    expect(result.trace.meters.context).toBe(native.meters.context)
    expect(detailsWithSupplement({ trace: native, overview, supplement: retained, harness: 'omp', nativeSessionId: 'different-root' }).trace).toBe(native)
  })
  it('keeps native facts when a private root is not found or the relay is unavailable', () => {
    const native = trace()
    const result = detailsWithSupplement({ trace: native, overview, supplement: unavailable({ reason: 'failed', detail: 'Root not found in reported cut; hub may lag.' }), harness: 'omp', nativeSessionId: supplement().nativeSessionId })
    expect(result.trace).toBe(native)
    expect(result.overview).toBe(overview)
  })
})
