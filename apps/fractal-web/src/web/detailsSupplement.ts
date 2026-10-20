import type { DetailsOverview, TraceSeries, TraceCoverage } from '@smalltalk/fractal-ui/assistant-ui'
import type { Feed } from '../data/source.ts'
import type { DetailsSupplement } from '../extensions/contract.ts'

const harnessName = (harness: string | undefined): string =>
  harness === 'claude' ? 'Claude Code' : harness === 'codex' ? 'Codex' : harness ?? 'this harness'

/** Separate agent and total spend reports: one plain visible sentence, precise detail behind Why?. */
export interface DetailsAccountingNote {
  readonly summary: string
  readonly reasons: readonly string[]
}
export const accountingNoteSummary = 'Agent spend and total spend come from separate reports and may not add up.'
export const spendWindowReason = 'Spend covers this time window, not only this agent.'

/** Merge only a proven matching root. Native money/tokens and provider windows remain independent. */
export const detailsWithSupplement = ({ trace, overview, supplement, harness, nativeSessionId }: {
  readonly trace: TraceSeries
  readonly overview: DetailsOverview
  readonly supplement: Feed<DetailsSupplement>
  readonly harness?: string
  readonly nativeSessionId?: string
}): { readonly trace: TraceSeries; readonly overview: DetailsOverview; readonly accountingNote?: DetailsAccountingNote } => {
  if (harness !== 'omp') {
    const coverage: TraceCoverage = harness === undefined
      ? { _tag: 'Unavailable', reason: 'not-attributed' }
      : { _tag: 'NotCovered', harnessName: harnessName(harness) }
    return {
      overview,
      trace: {
        ...trace,
        coverage,
        meters: {
          ...trace.meters,
          spend: { ...trace.meters.spend, totalUsdCoverage: coverage, subagentUsdCoverage: coverage },
        },
        subagents: { _tag: 'Unknown', reason: harness === undefined ? 'not-attributed' : 'not-covered' },
        subagentsWindow: { _tag: 'Unknown', reason: 'not-covered' },
        subagentCount: { _tag: 'Unknown', reason: 'not-covered' },
      },
    }
  }
  if (supplement._tag !== 'Observed' || nativeSessionId === undefined ||
    trace.nativeSessionId._tag !== 'Known' || trace.nativeSessionId.value !== nativeSessionId ||
    supplement.value.nativeSessionId !== nativeSessionId) return { trace, overview }
  const value = supplement.value
  return {
    ...(trace.meters.spend.mainUsd._tag === 'Known' && value.totalUsd._tag === 'Known' ? {
      // The two totals are independent reports, not a reconciled split: their difference is not subagent spend.
      accountingNote: { summary: accountingNoteSummary, reasons: [
        spendWindowReason,
        ...(value.subagentCount._tag === 'Known' && value.subagentCount.value === 0 ? ["No subagents were reported, so the difference isn't subagent spend."] : []),
      ] },
    } : {}),
    overview: value.durations === undefined ? overview : {
      ...overview,
      durations: [...(overview.durations ?? []), ...value.durations],
    },
    trace: {
      ...trace,
      coverage: value.coverage,
      meters: {
        ...trace.meters,
        spend: {
          ...trace.meters.spend,
          usdScope: { _tag: 'IncludingSubagents' },
          usdWindow: value.usdWindow,
          totalUsd: value.totalUsd,
          totalUsdCoverage: value.coverage,
          // An inclusive observed total does not establish a matching main/descendant split.
        },
      },
      subagents: value.subagents,
      subagentsWindow: value.subagentsWindow,
      subagentCount: value.subagentCount,
    },
  }
}
