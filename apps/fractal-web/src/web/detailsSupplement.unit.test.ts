import { describe, expect, it, vi } from 'vitest'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { DetailsResources, traceSubagentSection } from '../../../../packages/fractal-ui/src/assistant-ui/details/DetailsResources.tsx'
import { DetailsPane } from '../../../../packages/fractal-ui/src/assistant-ui/details/DetailsPane.tsx'
import type { DetailsOverview, TraceSeries, TraceFact } from '@smalltalk/fractal-ui/assistant-ui'
import { accountingNoteSummary, detailsWithSupplement, spendWindowReason } from './detailsSupplement.ts'
import type { DetailsSupplement } from '../extensions/contract.ts'
import { observed, unavailable } from '../data/source.ts'
import { traceUnknownReasons } from '../../../../packages/fractal-ui/src/assistant-ui/details/trace-series.ts'
import { detailsReasonText, joinDetailsReasons, traceFactReason, traceReasonText } from '../../../../packages/fractal-ui/src/assistant-ui/details/trace-reasons.ts'

// Match the existing node-rendering lane: semantic DOM, not compiled CSS.
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))

describe('Details reason copy exhausts the trace wire contract', () => {
  it.each(traceUnknownReasons)('maps %s to plain language rather than a raw code or fallback', reason => {
    const text = traceReasonText(reason)
    expect(text).not.toBe(reason)
    expect(text).not.toBe('Not reported')
    expect(text).not.toMatch(/^[a-z]+(-[a-z]+)+$/)
    expect(traceFactReason({ _tag: 'Unknown', reason })).toBe(text)
  })
  it('uses a safe fallback for unknown codes and collapses repeated input reasons', () => {
    expect(detailsReasonText('future-reason-code')).toBe('Not reported')
    expect(traceReasonText('future')).toBe('Not reported')
    expect(traceFactReason({ _tag: 'Unknown', reason: 'failed', detail: 'future-reason-code' })).toBe('Not reported')
    expect(joinDetailsReasons(['no-provider', 'no-provider', 'no-provider'])).toBe('This harness does not report it.')
    expect(detailsReasonText('The host has not reported a percentage.')).toBe('The host has not reported a percentage.')
  })
})

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
  durations: [{ label: 'Session wall time · last 7 days', value: '2h' }],
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
    expect(result.overview.durations?.map(item => item.label)).toEqual(['Managed session elapsed · wall time', 'Session wall time · last 7 days'])
    // A cumulative-token substitution must fail the same occupancy criterion.
    expect(() => expect({ ...result.trace.meters.context, reportedPercent: known(480_035_214 / 1_000_000 * 100) }).toEqual(native.meters.context)).toThrow()
  })
  it('explains independently sourced totals in plain words without treating their difference as subagent spend', () => {
    const native = trace()
    const result = join({ ...native, meters: { ...native.meters, spend: { ...native.meters.spend, mainUsd: known(6.19) } } }, { ...supplement(), totalUsd: known(9.05), subagentCount: known(0) })
    expect(result.accountingNote).toEqual({ summary: accountingNoteSummary, reasons: [spendWindowReason, "No subagents were reported, so the difference isn't subagent spend."] })
    expect(accountingNoteSummary).toBe('Agent spend and total spend come from separate reports and may not add up.')
    expect(join({ ...native, meters: { ...native.meters, spend: { ...native.meters.spend, mainUsd: known(6.19) } } }, supplement()).accountingNote?.reasons).toEqual([spendWindowReason])
    for (const copy of [result.accountingNote!.summary, ...result.accountingNote!.reasons])
      expect(copy).not.toMatch(/\b(native\w*|descendants?|incarnations?|provenance|series|ledger|attribution|window-global)\b|root completeness|this source/i)
    expect(result.trace.meters.spend.subagentUsd._tag).toBe('Unknown')
    expect(join(native, { ...supplement(), nativeSessionId: 'unrelated-root' }).accountingNote).toBeUndefined()
    expect(join({ ...native, meters: { ...native.meters, spend: { ...native.meters.spend, mainUsd: missing } } }, supplement()).accountingNote).toBeUndefined()
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

it('shows the reported traced count in the heading, not the bounded list size or an invented zero', () => {
  const render = (count: TraceFact<number>) => renderToStaticMarkup(createElement(DetailsResources, {
    state: 'partial', sections: [{ title: 'Subagents', items: { _tag: 'Known', value: [] } }],
    trace: { ...trace(), subagents: known([]), subagentCount: count }, now: 2000, empty: 'E3', onOpen: () => {},
  }))
  const heading = (html: string) => html.match(/<button[^>]*aria-expanded[^>]*>(.*?)<\/button>/)?.[1]?.replace(/<[^>]+>/g, '')
  expect(heading(render(known(309)))).toBe('Subagents309')
  expect(heading(render(known(0)))).toBe('Subagents0')
  expect(heading(render(missing))).toBe('Subagents')
})

it('keeps elapsed/age acquiring independently and omits missing timing fields', () => {
  const render = (durations: DetailsOverview['durations']) => renderToStaticMarkup(createElement(DetailsPane, {
    state: 'partial', overview: { ...overview, durations }, trace: trace(), sections: [], now: 2000,
    variants: { overview: 'O2', context: 'C3', spend: 'S2', resources: 'R1', empty: 'E3' },
    intent: { hiddenSections: [] }, onIntentChange: () => {}, onOpen: () => {}, onRetry: () => {},
  }))
  const pending = render(['Managed elapsed', 'Session age'].map(label => ({ label, value: 'Loading', pending: true })))
  expect(pending.match(/data-details-duration-pending="true"/g)).toHaveLength(2)
  expect(pending.match(/data-details-skeleton="duration"/g)).toHaveLength(2)
  expect(pending).toContain('Synthetic seat')
  expect(pending).not.toContain('Unreported')
  const reason = 'Session start matches the producer snapshot time; elapsed time and age are unreported.'
  const rejected = render(['Managed elapsed', 'Session age'].map(label => ({ label, value: 'Unreported', detail: reason })))
  expect(rejected).not.toContain('aria-label="Managed elapsed: unavailable"')
  expect(rejected).not.toContain('aria-label="Session age: unavailable"')
  expect(rejected).not.toContain(reason)
  expect(rejected).not.toContain('data-details-duration-pending')
})

it('labels an unreported subagent role before the full identity without fabricating a model', () => {
  const section = traceSubagentSection({ ...trace(), subagents: known([{
    sessionId: 'subagent-session', parentSessionId: missing, label: missing, model: missing,
    usd: 1, requests: 1, firstMs: missing, lastMs: missing,
  }]) })
  expect(section.items._tag === 'Known' ? section.items.value[0]?.card.label : undefined).toBe('role unreported · subagent-session')
})
