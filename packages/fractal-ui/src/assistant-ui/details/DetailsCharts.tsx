import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { ReactNode } from 'react'
import { accentVars as accent, borderVars as border, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from '../composition-tokens.stylex.ts'
import type { TraceSeries } from './trace-series.ts'
import { contextOccupancy, estimatedOccupancyExplanation, traceCoverageState, type ContextOccupancy, type TraceAccumulationWindow, type TraceCoverage, type TraceCoverageState, type TraceFact, type TraceSpendScope } from './trace-series.ts'
import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'
import type { DetailsEmptyRule } from './DetailsResources.tsx'
import { compactNumber, money, clock } from './trace/chart.ts'
import { adaptTrace } from './trace/adapter.ts'
import { CompactSpend, CompactSpendPending } from './trace/CompactSpend.tsx'
import { ContextPanel, ContextPanelPending, contextPanelFootprint, type ContextPanelFootprint } from './trace/ContextPanel.tsx'
import { SpendPanel, SpendPanelPending } from './trace/SpendPanel.tsx'
import { DescribedTooltipButton, DetailsInlineReason, DetailsNotReported, type NotReportedField } from './DetailsUnavailable.tsx'
import { NarrowLayoutContext } from '../composition/NarrowLayout.tsx'
import { traceReasonText, joinDetailsReasons, traceFactReason } from './trace-reasons.ts'
export interface DetailsChartsProps {
  readonly trace: TraceSeries
  readonly context: 'C1' | 'C2' | 'C3'
  readonly spend: 'S1' | 'S2' | 'S3'
  readonly empty?: DetailsEmptyRule
  /** Render an unwrapped body for a parent's independently collapsible section. */
  readonly section?: 'context' | 'spend'
  /** Suppress a wholly unreported section's line when its parent hides that section; partial summaries keep their provenance. */
  readonly showUnavailableReasons?: boolean
}

const missingReason = (fact: TraceFact<unknown>): string => fact._tag === 'Unknown' ? traceFactReason(fact) : 'Context occupancy could not be derived from the reported values.'

/** A group's missing fields, named once; a field missing for several reasons keeps all of them. */
const gap = (name: string, reason: string): NotReportedField => ({ name, reason })
const mergeGaps = (gaps: readonly NotReportedField[]): readonly NotReportedField[] => {
  const merged = new Map<string, string[]>()
  for (const { name, reason } of gaps) {
    const reasons = merged.get(name) ?? []
    if (!reasons.includes(reason)) reasons.push(reason)
    merged.set(name, reasons)
  }
  return [...merged].map(([name, reasons]) => gap(name, joinDetailsReasons(reasons)))
}

type ContextMeters = TraceSeries['meters']['context']
const hasContextMeterData = (context: ContextMeters): boolean => contextOccupancy(context) !== undefined || context.compactions._tag === 'Known' || context.lastCompactionMs._tag === 'Known'
/** Only displayed facts are gaps: a token-derived occupancy is an estimate, not a missing percentage. */
const contextGaps = (context: ContextMeters): readonly NotReportedField[] => [
  ...(contextOccupancy(context) === undefined ? [gap('occupancy', joinDetailsReasons([context.reportedPercent, context.usedTokens, context.windowTokens].filter(fact => fact._tag === 'Unknown').map(missingReason)) || 'Context occupancy could not be derived from the reported values.')] : []),
  ...(context.compactions._tag === 'Unknown' ? [gap('compactions', missingReason(context.compactions))] : []),
  ...(context.lastCompactionMs._tag === 'Unknown' ? [gap('last compaction', missingReason(context.lastCompactionMs))] : []),
]

/**
 * Reported percentages stay plain; estimates carry `est.` and the harness explanation on keyboard focus and hover.
 * Narrow (phone) hosts have no hover: the explanation paints beneath the value.
 */
const OccupancyValue = ({ occupancy }: { readonly occupancy: ContextOccupancy }): ReactNode => {
  const narrow = React.useContext(NarrowLayoutContext)
  return <span data-details-occupancy={occupancy.estimated ? 'estimated' : 'reported'} {...stylex.props(styles.occupancy, narrow && occupancy.estimated && styles.occupancyNarrow)}>
    <strong>{Math.round(occupancy.percent)}%</strong>
    {occupancy.estimated && (narrow
      ? <><span data-details-occupancy-estimated {...stylex.props(styles.estimateText)}>est.</span><DetailsInlineReason block>{estimatedOccupancyExplanation}</DetailsInlineReason></>
      : <TooltipTrigger delay={0}><Button data-details-occupancy-estimated aria-label="est., estimated from tokens" {...stylex.props(styles.estimate)}>est.</Button><Tooltip {...stylex.props(styles.coverageTooltip)}>{estimatedOccupancyExplanation}</Tooltip></TooltipTrigger>)}
  </span>
}

/** Only reported facts paint; missing ones are named once in the section's Not reported line. */
const ContextMeter = ({ context }: { readonly context: ContextMeters }): ReactNode => {
  const occupancy = contextOccupancy(context)
  const { compactions, lastCompactionMs } = context
  if (!hasContextMeterData(context)) return null
  return <div {...stylex.props(styles.contextSnapshot)}>
    {occupancy !== undefined && <>
      <div data-details-data-row {...stylex.props(styles.numberRow)}><OccupancyValue occupancy={occupancy} /></div>
      <div {...stylex.props(styles.track)} role="meter" aria-label="Context occupancy" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.max(0, Math.min(100, occupancy.percent))} aria-valuetext={`${Math.round(occupancy.percent)} percent${occupancy.estimated ? ', estimated from tokens' : ''}`}><div {...stylex.props(styles.fill(`${Math.max(0, Math.min(100, occupancy.percent))}%`))} /></div>
    </>}
    {(compactions._tag === 'Known' || lastCompactionMs._tag === 'Known') && <div data-details-data-row {...stylex.props(styles.numberRow)}>
      {compactions._tag === 'Known' && <span>Compactions {compactNumber.format(compactions.value)}</span>}
      {lastCompactionMs._tag === 'Known' && <span>Last {clock.format(lastCompactionMs.value)} UTC</span>}
    </div>}
  </div>
}

/** Loading twin of ContextMeter: formatted footprints wrap naturally under text-shaped ink. */
const ContextMeterPending = ({ footprint }: { readonly footprint: DetailsPendingShape['context']['meter'] }): ReactNode => footprint.percentage === undefined && footprint.compactions === undefined && footprint.lastCompaction === undefined ? null : <div {...stylex.props(styles.contextSnapshot)}>
  {footprint.percentage !== undefined && <>
    <div {...stylex.props(styles.numberRow)}><span {...stylex.props(styles.occupancy)}><strong><span {...stylex.props(styles.pendingText)}>{footprint.percentage}</span></strong>{footprint.estimated && <span {...stylex.props(styles.estimate, styles.pendingText)}>est.</span>}</span></div>
    <div {...stylex.props(styles.track)} />
  </>}
  {(footprint.compactions !== undefined || footprint.lastCompaction !== undefined) && <div {...stylex.props(styles.numberRow)}>
    {footprint.compactions !== undefined && <span><span {...stylex.props(styles.pendingText)}>{footprint.compactions}</span></span>}
    {footprint.lastCompaction !== undefined && <span><span {...stylex.props(styles.pendingText)}>{footprint.lastCompaction}</span></span>}
  </div>}
</div>

const scopeLabel = (scope:TraceSpendScope):string => scope._tag==='Self'?' · self':scope._tag==='IncludingSubagents'?' · including subagents':scope._tag==='AgentIncarnations'?' · agent incarnations':''
const tokenScopeLabel = (scope:TraceSpendScope):string => scope._tag==='Self'?' · this agent':scope._tag==='IncludingSubagents'?' · this agent + subagents':scope._tag==='AgentIncarnations'?' · observed agent incarnations':''
export const accumulationParts = (fact:TraceFact<TraceAccumulationWindow>):readonly string[]|undefined => {
  if(fact._tag==='Unknown')return undefined
  const window=fact.value
  if(window._tag==='AllTime')return ['all time']
  if(window._tag==='Reported')return [`reported window · ${window.label}`]
  const from = new Date(window.fromMs).toISOString()
  const to = new Date(window.toMs).toISOString()
  return from.slice(0,10) === to.slice(0,10)
    ? [`${from.slice(0,10)} ${from.slice(11,16)}–${to.slice(11,16)} UTC`]
    : [`${from.slice(0,10)} ${from.slice(11,16)}–`, `${to.slice(0,10)} ${to.slice(11,16)} UTC`]
}
/** Both row props and pending footprints normalize only the malformed row's coverage; unavailable attribution is a Not reported field. */
const coverageLabel=(coverage:TraceCoverageState):string|undefined=>{
 const state=traceCoverageState(coverage)
 return state._tag==='Complete'||state._tag==='Unavailable'?undefined:state._tag==='PartialUnreported'?'Partial':state._tag==='Partial'?`Partial · ${Math.min(99,Math.floor(state.attributedFraction*100))}% attributed`:`Not tracked for ${state.harnessName}`
}
const notCoveredExplanation = 'Spend is recorded only for OMP sessions today.'
export const TraceCoverageNote = ({trace,coverage=trace.coverage}:{readonly trace:TraceSeries;readonly coverage?:TraceCoverage}):ReactNode => {
 const narrow=React.useContext(NarrowLayoutContext)
 const state=traceCoverageState(coverage)
 if(state._tag==='Complete'||state._tag==='Unavailable')return null
 if(state._tag==='NotCovered')return narrow
  ?<p data-details-coverage {...stylex.props(styles.scopeNote)}>{coverageLabel(state)}<DetailsInlineReason block>{notCoveredExplanation}</DetailsInlineReason></p>
  :<DescribedTooltipButton data-details-coverage description={notCoveredExplanation} tooltipStyle={styles.coverageTooltip} {...stylex.props(styles.coverageTrigger)}>{coverageLabel(state)}</DescribedTooltipButton>
 return <p data-details-coverage {...stylex.props(styles.scopeNote)}>{coverageLabel(state)}</p>
}

/** Both live and pending rows use the same reported window and wrapping boundaries. */
const MetricWindow = ({ parts, pending = false, tokens = false }: { readonly parts: readonly string[]; readonly pending?: boolean;readonly tokens?:boolean }): ReactNode =>
  <span data-details-metric-period={pending?undefined:true} data-details-token-period={pending||!tokens ? undefined : true} {...stylex.props(styles.tokenWindow)}>{parts.map((part,index)=><span key={index}>{index>0?<wbr/>:null}<span {...stylex.props(styles.windowPart)}><span {...stylex.props(pending&&styles.pendingText)}>{part}</span></span></span>)}</span>

/** One spend meter with its own scope, window and attribution; `prefix` names its metadata gaps. */
interface SpendMetric {
  readonly metric: 'total-usd' | 'main-usd' | 'subagent-usd' | 'tokens'
  readonly label: string
  readonly name: string
  readonly prefix: string
  readonly fact: TraceFact<number>
  readonly scope: TraceSpendScope
  readonly window: TraceFact<TraceAccumulationWindow>
  readonly coverage?: TraceCoverage
  readonly tokens: boolean
}
const spendMetrics = (trace: TraceSeries): readonly SpendMetric[] => {
  const spend = trace.meters.spend
  return [
    { metric: 'total-usd', label: spend.totalUsdCoverage._tag === 'Complete' ? 'Total' : 'Recorded spend', name: 'total spend', prefix: 'total', fact: spend.totalUsd, scope: spend.usdScope, window: spend.usdWindow, coverage: spend.totalUsdCoverage, tokens: false },
    { metric: 'main-usd', label: 'Main session', name: 'main session spend', prefix: 'main session', fact: spend.mainUsd, scope: spend.mainUsdScope, window: spend.mainUsdWindow, coverage: spend.mainUsdCoverage, tokens: false },
    { metric: 'subagent-usd', label: spend.subagentUsdCoverage._tag === 'Partial' ? 'Attributed subagents' : 'Subagents', name: 'subagent spend', prefix: 'subagent', fact: spend.subagentUsd, scope: spend.subagentUsdScope, window: spend.subagentUsdWindow, coverage: spend.subagentUsdCoverage, tokens: false },
    { metric: 'tokens', label: 'Cumulative tokens', name: 'cumulative tokens', prefix: 'token', fact: spend.tokens, scope: spend.tokensScope, window: spend.tokensWindow, tokens: true },
  ]
}
const metricLabel = (row: SpendMetric): string => `${row.label}${row.tokens ? tokenScopeLabel(row.scope) : scopeLabel(row.scope)}`
const metricValue = (row: SpendMetric, value: number): string => row.tokens ? compactNumber.format(value) : money.format(value)
const reportedMetrics = (trace: TraceSeries): readonly (SpendMetric & { readonly value: number })[] =>
  spendMetrics(trace).flatMap(row => row.fact._tag === 'Known' ? [{ ...row, value: row.fact.value }] : [])
const tracedSubagents = (trace: TraceSeries) => trace.subagents._tag === 'Known' ? trace.subagents.value : []
const tracedSessionsLine = (trace: TraceSeries): string => {
  const parts = accumulationParts(trace.subagentsWindow)
  return parts === undefined ? 'Traced sessions' : `Traced sessions · ${parts.join('')}`
}
/** A missing value names its metric; a reported value's missing scope, window or attribution names that metadata. */
const spendGaps = (trace: TraceSeries): readonly NotReportedField[] => [
  ...spendMetrics(trace).flatMap(row => row.fact._tag === 'Unknown' ? [gap(row.name, missingReason(row.fact))] : [
    ...(row.scope._tag === 'Unknown' ? [gap(`${row.prefix} scope`, traceReasonText(row.scope.reason))] : []),
    ...(row.window._tag === 'Unknown' ? [gap(`${row.prefix} window`, missingReason(row.window))] : []),
    ...(row.coverage?._tag === 'Unavailable' ? [gap('attribution', traceReasonText(row.coverage.reason))] : []),
  ]),
  ...(trace.coverage._tag === 'Unavailable' ? [gap('attribution', traceReasonText(trace.coverage.reason))] : []),
  ...(tracedSubagents(trace).length > 0 && trace.subagentsWindow._tag === 'Unknown' ? [gap('traced-session window', missingReason(trace.subagentsWindow))] : []),
]

const SpendMeter = ({ trace }: { readonly trace: TraceSeries }): ReactNode => {
  const rows = reportedMetrics(trace)
  const subagents = tracedSubagents(trace)
  if (rows.length === 0 && subagents.length === 0) return null
  return <div {...stylex.props(styles.body)}>
    {rows.length > 0 && <dl {...stylex.props(styles.spendSnapshot)} aria-label="Trace spend meters">{rows.map(row => {
      const parts = accumulationParts(row.window)
      return <div key={row.metric} data-details-metric={row.metric} {...stylex.props(styles.spendRow)}>
        <dt {...stylex.props(styles.label)}>{row.tokens ? <span data-details-token-label>{metricLabel(row)}</span> : metricLabel(row)}{parts !== undefined && <MetricWindow parts={parts} tokens={row.tokens} />}{row.coverage !== undefined && <TraceCoverageNote trace={trace} coverage={row.coverage} />}</dt>
        <dd {...stylex.props(row.tokens ? styles.tokens : styles.amount)}>{metricValue(row, row.value)}</dd>
      </div>
    })}</dl>}
    <TraceCoverageNote trace={trace} />
    {subagents.length > 0 && <p data-details-subagent-spend-window {...stylex.props(styles.scopeNote)}>{tracedSessionsLine(trace)}</p>}
    {subagents.length > 0 && <dl {...stylex.props(styles.spendSnapshot)} aria-label="Subagent spend breakdown">{subagents.map((child) => <div key={child.sessionId} {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}>{child.label._tag==='Known'?child.label.value:child.sessionId}</dt><dd {...stylex.props(styles.amount)}>{money.format(child.usd)}</dd></div>)}</dl>}
  </div>
}

/** Loading twin of SpendMeter: the same reported rows and the same absent ones. */
const SpendMeterPending = ({ footprint }: { readonly footprint: DetailsPendingShape['spend'] }): ReactNode => footprint.rows.length === 0 && footprint.subagents.length === 0 ? null : <div aria-hidden="true" inert {...stylex.props(styles.body)}>
  {footprint.rows.length > 0 && <dl {...stylex.props(styles.spendSnapshot)}>
    {footprint.rows.map(row=><div key={row.label} {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}><span {...stylex.props(styles.pendingText)}>{row.label}</span>{row.window!==undefined&&<MetricWindow parts={row.window} pending tokens={row.tokens}/>}{row.coverage!==undefined&&<p {...stylex.props(styles.scopeNote)}><span {...stylex.props(styles.pendingText)}>{row.coverage}</span></p>}</dt><dd {...stylex.props(row.tokens?styles.tokens:styles.amount)}><span {...stylex.props(styles.pendingText)}>{row.value}</span></dd></div>)}
  </dl>}
  {footprint.coverage!==undefined&&<p {...stylex.props(styles.scopeNote)}><span {...stylex.props(styles.pendingText)}>{footprint.coverage}</span></p>}
  {footprint.subagents.length>0&&<p {...stylex.props(styles.scopeNote)}><span {...stylex.props(styles.pendingText)}>{footprint.subagentWindow}</span></p>}
  {footprint.subagents.length>0&&<dl {...stylex.props(styles.spendSnapshot)}>{footprint.subagents.map((row,index)=><div key={index} {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}><span {...stylex.props(styles.pendingText)}>{row.label}</span></dt><dd {...stylex.props(styles.amount)}><span {...stylex.props(styles.pendingText)}>{row.value}</span></dd></div>)}</dl>}
</div>

const hasSpendMeterData = (trace: TraceSeries): boolean => reportedMetrics(trace).length > 0 || tracedSubagents(trace).length > 0

const hasReportedSplit = (trace: TraceSeries): boolean => trace.meters.spend.mainUsdScope._tag !== 'Unknown'
  && trace.meters.spend.split._tag === 'Known' && Object.values(trace.meters.spend.split.value).every(Number.isFinite)

/** Match the selected chart's usable rows, not merely the existence of a provider. */
export const hasDetailsChartData = ({ trace, context, spend, section, empty = 'E1' }: DetailsChartsProps): boolean => {
  const projection = adaptTrace(trace)
  const samples = projection?.lane.turnSamples !== undefined && (context === 'C2' || projection.lane.samples.length === 0) ? projection.lane.turnSamples : projection?.lane.samples
  const contextData = hasContextMeterData(trace.meters.context)
    || (context !== 'C1' && samples?.some(sample => sample.contextPercent !== null) === true)
  if (section === 'context') return contextData
  const meters = trace.meters.spend
  let spendData: boolean
  if (empty === 'E3') {
    spendData = hasSpendMeterData(trace) || (meters.mainUsdScope._tag !== 'Unknown' && projection !== undefined && spend !== 'S1'
      && (projection.lane.cumulativeValues.length > 0 || (spend === 'S2' && projection.descendantSpend.cumulativeValues.length > 0)))
      || (spend === 'S3' && hasReportedSplit(trace))
  } else if (spend === 'S1' || meters.mainUsdScope._tag === 'Unknown' || projection === undefined) {
    spendData = hasSpendMeterData(trace)
  } else {
    spendData = projection.lane.totalUsd._tag === 'Known' || (spend === 'S2'
      ? projection.descendantSpend.totalUsd._tag === 'Known' || projection.lane.cumulativeValues.length > 0
      : meters.split._tag === 'Known' && projection.lane.samples.length > 0)
  }
  spendData ||= hasSpendMeterData(trace)
  return section === 'spend' ? spendData : contextData || spendData
}

/** Meters and drawing evidence share one portable Known/Unknown contract; each group paints at most one missing-data line. */
export const DetailsCharts = ({ trace, context, spend, section, showUnavailableReasons = true }: DetailsChartsProps): ReactNode => {
  const projection = adaptTrace(trace)
  const seriesReason = trace.series._tag === 'Unknown' ? traceFactReason(trace.series) : 'No historical samples were retained.'
  const spendProjection = trace.meters.spend.mainUsdScope._tag === 'Unknown' ? undefined : projection
  const spendReason = trace.meters.spend.mainUsdScope._tag === 'Unknown' ? 'Session spend scope is not attributed' : seriesReason
  const contextSamples = projection?.lane.turnSamples !== undefined && (context === 'C2' || projection.lane.samples.length === 0) ? projection.lane.turnSamples : projection?.lane.samples
  const contextSeries = context !== 'C1' && contextSamples?.some(sample => sample.contextPercent !== null) === true
  const compactSeries = spend === 'S2' && spendProjection !== undefined && (spendProjection.lane.cumulativeValues.length > 0 || spendProjection.descendantSpend.cumulativeValues.length > 0)
  const panelSeries = spend === 'S3' && spendProjection !== undefined && spendProjection.lane.cumulativeValues.length > 0
  const splitOnly = spend === 'S3' && !panelSeries && hasReportedSplit(trace)
  const contextFields = mergeGaps([...contextGaps(trace.meters.context), ...(context !== 'C1' && !contextSeries ? [gap('context series', seriesReason)] : [])])
  const spendFields = mergeGaps([...spendGaps(trace),
    ...(spend !== 'S1' && !compactSeries && !panelSeries ? [gap('spend series', spendReason)] : []),
    ...(compactSeries && spendProjection !== undefined && spendProjection.lane.cumulativeValues.length === 0 ? [gap('main session series', 'Session spend series was not retained.')] : []),
    ...(compactSeries && spendProjection !== undefined && spendProjection.descendantSpend.cumulativeValues.length === 0 ? [gap('subagent series', 'Descendant spend series was not retained.')] : []),
    ...(spend === 'S3' && !hasReportedSplit(trace) ? [gap('spend split', trace.meters.spend.split._tag === 'Unknown' ? missingReason(trace.meters.spend.split) : 'Valid session-attributed spend components were not supplied.')] : [])])
  const contextWhole = !hasContextMeterData(trace.meters.context) && !contextSeries
  const spendWhole = !hasSpendMeterData(trace) && !compactSeries && !panelSeries && !splitOnly
  const contextBody = <div {...stylex.props(styles.body)} data-details-chart={context}>
    {contextWhole ? showUnavailableReasons && <DetailsNotReported group="Context" fields={contextFields} /> : <>
      <ContextMeter context={trace.meters.context} />
      {contextSeries && projection !== undefined && <ContextPanel scene={projection.scene} lane={projection.lane} compact={context === 'C2'} />}
      <DetailsNotReported fields={contextFields} />
    </>}
  </div>
  const spendBody = <div {...stylex.props(styles.body)} data-details-chart={spend}>
    {spendWhole ? showUnavailableReasons && <DetailsNotReported group="Spend" fields={spendFields} /> : <>
      <SpendMeter trace={trace} />
      {compactSeries && <CompactSpend lane={spendProjection!.lane} descendantSpend={spendProjection!.descendantSpend} showTotals={false}/>}
      {panelSeries && <SpendPanel lane={spendProjection!.lane} {...(hasReportedSplit(trace) && trace.meters.spend.split._tag === 'Known' ? {split:trace.meters.spend.split.value} : {})} componentsKnown={hasReportedSplit(trace)} showTotals={false}/>}
      {splitOnly && trace.meters.spend.split._tag === 'Known' && <SpendPanel split={trace.meters.spend.split.value} componentsKnown showTotals={false}/>}
      <DetailsNotReported fields={spendFields} />
    </>}
  </div>
  if (section === 'context') return contextBody
  if (section === 'spend') return spendBody
  return <div {...stylex.props(styles.root)}><section aria-label="Context"><h3 {...stylex.props(styles.heading)}>Context</h3>{contextBody}</section><section aria-label="Spend"><h3 {...stylex.props(styles.heading)}>Spend</h3>{spendBody}</section></div>
}

/** Only formatted wrap footprints and structural counts; no pending observations are displayed. */
export interface DetailsPendingShape {
  readonly context: {
    readonly meter: {
      readonly percentage: string | undefined
      /** Token-derived occupancy reserves the live `est.` marker. */
      readonly estimated: boolean
      readonly compactions: string | undefined
      readonly lastCompaction: string | undefined
    }
    readonly compactPanel: ContextPanelFootprint | undefined
    readonly fullPanel: ContextPanelFootprint | undefined
    /** Not reported field names, in the live line's order. */
    readonly gaps: readonly string[]
  }
  readonly spend: {
    /** Reported metrics only; absent metrics are named in `gaps`. */
    readonly rows: readonly { readonly label: string; readonly window?: readonly string[]|undefined; readonly coverage?:string|undefined; readonly value: string; readonly tokens: boolean }[]
    readonly subagents: readonly { readonly label: string; readonly value: string }[]
    readonly subagentWindow?:string|undefined
    /** Not reported field names, in the live line's order. */
    readonly gaps: readonly string[]
    readonly coverage?: string | undefined
    readonly compactChart?: { readonly series: boolean; readonly descendants: boolean; readonly partial: boolean }
    readonly fullChart?: { readonly series: boolean; readonly split: boolean; readonly partial: boolean }
  }
}

export const pendingShapeFromTrace = (trace: TraceSeries): DetailsPendingShape => {
  const projection = adaptTrace(trace)
  const { compactions, lastCompactionMs } = trace.meters.context
  const occupancy = contextOccupancy(trace.meters.context)
  const spendProjection = trace.meters.spend.mainUsdScope._tag === 'Unknown' ? undefined : projection
  const retainedUsd = spendProjection?.lane.cumulativeValues.at(-1)?.usd ?? 0
  const retainedDescendants = spendProjection?.descendantSpend.cumulativeValues.at(-1)?.usd ?? 0
  const mainPartial = spendProjection?.lane.totalUsd._tag === 'Known' && Math.abs(retainedUsd - spendProjection.lane.totalUsd.value) > 0.005
  const descendantPartial = spendProjection?.descendantSpend.totalUsd._tag === 'Known' && Math.abs(retainedDescendants - spendProjection.descendantSpend.totalUsd.value) > 0.005
  return {
    context: {
      meter: {
        percentage: occupancy !== undefined ? `${Math.round(occupancy.percent)}%` : undefined,
        estimated: occupancy?.estimated === true,
        compactions: compactions._tag === 'Known' ? `Compactions ${compactNumber.format(compactions.value)}` : undefined,
        lastCompaction: lastCompactionMs._tag === 'Known' ? `Last ${clock.format(lastCompactionMs.value)} UTC` : undefined,
      },
      compactPanel: projection !== undefined ? contextPanelFootprint(projection.lane, true) : undefined,
      fullPanel: projection !== undefined ? contextPanelFootprint(projection.lane, false) : undefined,
      gaps: mergeGaps(contextGaps(trace.meters.context)).map(field => field.name),
    },
    spend: {
      rows: reportedMetrics(trace).map(row => ({ label: metricLabel(row), window: accumulationParts(row.window), coverage: row.coverage === undefined ? undefined : coverageLabel(row.coverage), value: metricValue(row, row.value), tokens: row.tokens })),
      subagents: tracedSubagents(trace).map(child=>({label:child.label._tag==='Known'?child.label.value:child.sessionId,value:money.format(child.usd)})),
      subagentWindow: tracedSessionsLine(trace),
      gaps: mergeGaps(spendGaps(trace)).map(field => field.name),
      coverage: coverageLabel(trace.coverage),
      compactChart: { series: (spendProjection?.lane.cumulativeValues.length ?? 0) > 0, descendants: (spendProjection?.descendantSpend.cumulativeValues.length ?? 0) > 0, partial: mainPartial || descendantPartial },
      fullChart: { series: (spendProjection?.lane.cumulativeValues.length ?? 0) > 0, split: hasReportedSplit(trace), partial: mainPartial },
    },
  }
}

/** Unpainted twin of DetailsNotReported, reserving the same line and the same unbreakable names. */
const NotReportedPending = ({ group, names }: { readonly group?: string; readonly names: readonly string[] }): ReactNode => names.length === 0 ? null
  : <p {...stylex.props(styles.notReported)}><span {...stylex.props(styles.pendingText)}>{group !== undefined ? `${group} · not reported` : <>Not reported: {names.map((name, index) => <span key={name}>{index > 0 && ', '}<span {...stylex.props(styles.nowrap)}>{name}</span></span>)}</>}</span></p>

/**
 * Loading bodies for the pane's collapsible Context and Spend sections. They
 * mirror the populated per-variant structure (wrappers, gaps, chart aspect
 * ratios, row counts) so arrival cannot shift either section; the pane's one
 * SyncLine connection status is the only announced loading text.
 */
export const DetailsChartsPending = ({ context, spend, section, shape }: {
  readonly context: 'C1' | 'C2' | 'C3'
  readonly spend: 'S1' | 'S2' | 'S3'
  readonly section?: 'context' | 'spend'
  readonly shape: DetailsPendingShape
  /** Empty policy does not alter the reported meter geometry. */
  readonly empty?: DetailsEmptyRule
}): ReactNode => {
  const panelFootprint = context === 'C1' ? undefined : context === 'C2' ? shape.context.compactPanel : shape.context.fullPanel
  const meter = shape.context.meter
  const contextWhole = meter.percentage === undefined && meter.compactions === undefined && meter.lastCompaction === undefined && panelFootprint === undefined
  const contextNames = [...shape.context.gaps, ...(context !== 'C1' && panelFootprint === undefined ? ['context series'] : [])]
  const compact = shape.spend.compactChart, full = shape.spend.fullChart
  const compactShown = spend === 'S2' && (compact === undefined || compact.series || compact.descendants)
  const fullShown = spend === 'S3' && (full === undefined || full.series || full.split)
  const spendNames = [...shape.spend.gaps,
    ...(spend === 'S2' && compact !== undefined ? !compact.series && !compact.descendants ? ['spend series'] : [...(compact.series ? [] : ['main session series']), ...(compact.descendants ? [] : ['subagent series'])] : []),
    ...(spend === 'S3' && full !== undefined ? [...(full.series ? [] : ['spend series']), ...(full.split ? [] : ['spend split'])] : [])]
  const spendWhole = shape.spend.rows.length === 0 && shape.spend.subagents.length === 0 && !compactShown && !fullShown
  const contextBody = <div aria-hidden="true" inert data-details-skeleton="context" data-details-chart={context} {...stylex.props(styles.body)}>
    {contextWhole ? <NotReportedPending group="Context" names={contextNames} /> : <>
      <ContextMeterPending footprint={meter} />
      {panelFootprint !== undefined && <ContextPanelPending compact={context === 'C2'} footprint={panelFootprint} />}
      <NotReportedPending names={contextNames} />
    </>}
  </div>
  const spendBody = <div aria-hidden="true" data-details-skeleton="spend" data-details-chart={spend} {...stylex.props(styles.body)}>
    {spendWhole ? <NotReportedPending group="Spend" names={spendNames} /> : <>
      <SpendMeterPending footprint={shape.spend} />
      {compactShown && <CompactSpendPending showTotals={false} {...(compact === undefined ? {} : {footprint:compact})}/>}
      {fullShown && <SpendPanelPending showTotals={false} {...(full === undefined ? {} : {footprint:full})}/>}
      <NotReportedPending names={spendNames} />
    </>}
  </div>
  if (section === 'context') return contextBody
  if (section === 'spend') return spendBody
  return <div {...stylex.props(styles.root)} data-details-skeleton><section aria-label="Context"><h3 {...stylex.props(styles.heading)}>Context</h3>{contextBody}</section><section aria-label="Spend"><h3 {...stylex.props(styles.heading)}>Spend</h3>{spendBody}</section></div>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', gap: space.lg, minWidth: 0 },
  body: { display: 'flex', flexDirection: 'column', gap: space.md, minWidth: 0, color: text.fg, fontSize: type.metaSize, lineHeight: type.metaLeading },
  heading: { margin: 0, marginBottom: space.md, fontSize: type.metaSize, lineHeight: type.metaLeading, fontWeight: type.weightMedium, color: text.fg },
  notReported: { margin: 0, color: text.fgMuted, fontSize: type.metaSize, lineHeight: type.metaLeading },
  nowrap: { whiteSpace: 'nowrap' },
  // Narrow hosts: the estimate's explanation wraps onto its own row beneath the value.
  occupancyNarrow: { flexWrap: 'wrap' },
  estimateText: { color: text.fgMuted, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading },
  scopeNote: { margin: 0, color: text.fgSoft, fontSize: type.denseSize, lineHeight: type.metaLeading },
  coverageTrigger: { appearance:'none', borderWidth:0, padding:0, backgroundColor:surface.transparent, textAlign:'left', color:text.fgSoft, fontSize:type.denseSize, cursor:'help', outlineColor:accent.primary, outlineOffset:2 },
  coverageTooltip: { backgroundColor:surface.canvas, color:text.fg, borderWidth:1, borderStyle:'solid', borderColor:border.borderStrong, padding:space.md, fontSize:type.metaSize, maxWidth:280 },
  occupancy: { display: 'inline-flex', alignItems: 'baseline', gap: space.xs },
  estimate: { appearance: 'none', borderWidth: 0, padding: 0, backgroundColor: surface.transparent, color: text.fgMuted, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, cursor: 'help', borderRadius: radius.sm, ':focus-visible': { outline: `2px solid ${accent.primary}`, outlineOffset: geometry.focusOffset } },
  contextSnapshot: { display: 'flex', flexDirection: 'column', gap: space.xs, minWidth: 0 },
  numberRow: { display: 'flex', justifyContent: 'space-between', flexWrap: 'wrap', gap: space.md, fontVariantNumeric: 'tabular-nums' },
  track: { height: geometry.progressMd, backgroundColor: surface.rowActive, borderRadius: radius.full, overflow: 'hidden' },
  fill: (width: string) => ({ height: '100%', width, backgroundColor: accent.primary, borderRadius: radius.full }),
  spendSnapshot: { display: 'flex', flexDirection: 'column', gap: space.xs, margin: 0, minWidth: 0 },
  spendRow: { display: 'grid', gridTemplateColumns: `var(--details-spend-columns, minmax(0, 1fr) minmax(${geometry.detailsValueMin}, max-content))`, alignItems: 'baseline', gap: space.md, fontVariantNumeric: 'tabular-nums' },
  label: { minWidth: 0, color: text.fgMuted, overflowWrap: 'normal' },
  tokenWindow: { display: 'block', minWidth: 0, color: text.fgSoft, fontSize: type.denseSize, lineHeight: type.metaLeading },
  // Reported window descriptions wrap between words at narrow widths without shortening scope or provenance; words never break.
  windowPart: { whiteSpace: 'normal', overflowWrap: 'normal' },
  pendingText: { color: surface.transparent, backgroundImage: `linear-gradient(${border.borderStrong}, ${border.borderStrong})`, backgroundSize: `100% ${space.md}`, backgroundPosition: 'center', backgroundRepeat: 'no-repeat', boxDecorationBreak: 'clone' },
  amount: { margin: 0, textAlign: 'var(--details-value-align, right)', overflowWrap: 'normal' },
  tokens: { margin: 0, color: text.fgMuted, textAlign: 'var(--details-value-align, right)', overflowWrap: 'normal' },
})
