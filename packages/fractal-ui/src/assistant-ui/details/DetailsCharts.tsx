import * as stylex from '@stylexjs/stylex'
import type { ReactNode } from 'react'
import { accentVars as accent, borderVars as border, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from '../composition-tokens.stylex.ts'
import type { TraceSeries, TraceUnknownReason } from './trace-series.ts'
import { contextPercent, type TraceAccumulationWindow, type TraceCoverage, type TraceFact, type TraceSpendScope } from './trace-series.ts'
import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'
import type { DetailsEmptyRule } from './DetailsResources.tsx'
import { compactNumber, money, clock } from './trace/chart.ts'
import { adaptTrace } from './trace/adapter.ts'
import { CompactSpend, CompactSpendPending } from './trace/CompactSpend.tsx'
import { ContextPanel, ContextPanelPending, contextPanelFootprint, type ContextPanelFootprint } from './trace/ContextPanel.tsx'
import { SpendPanel, SpendPanelPending } from './trace/SpendPanel.tsx'
export interface DetailsChartsProps {
  readonly trace: TraceSeries
  readonly context: 'C1' | 'C2' | 'C3'
  readonly spend: 'S1' | 'S2' | 'S3'
  readonly empty?: DetailsEmptyRule
  /** Render an unwrapped body for a parent's independently collapsible section. */
  readonly section?: 'context' | 'spend'
  /** Brand-new consolidates unavailable-source explanations outside the sections. */
  readonly showUnavailableReasons?: boolean
}

const UnknownSeries = ({ reason }: { readonly reason: string }): ReactNode => <p data-details-reason={reason} {...stylex.props(styles.unknown)}>{reason}</p>

const seriesExplanation: Record<TraceUnknownReason, string> = {
  'no-provider': 'The current source does not provide historical samples.',
  'not-covered': 'Historical samples are not available for this session.',
  'not-attributed': 'Historical samples are not attributed to this session.',
  'out-of-retention': 'Historical samples are no longer retained.',
  loading: 'Historical samples are still loading.',
  failed: 'Historical samples could not be loaded.',
}

const ContextMeter = ({ trace, showUnavailableReasons }: { readonly trace: TraceSeries; readonly showUnavailableReasons: boolean }): ReactNode => {
  const percentage = contextPercent(trace.meters.context)
  return <div {...stylex.props(styles.contextSnapshot)}>
    <div data-details-data-row={percentage!==undefined?true:undefined} {...stylex.props(styles.numberRow)}>{percentage!==undefined&&<strong>{Math.round(percentage)}%</strong>}</div>
    {percentage === undefined ? showUnavailableReasons && <UnknownSeries reason="Context occupancy unavailable" /> : <div {...stylex.props(styles.track)} role="meter" aria-label="Context occupancy" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.max(0,Math.min(100,percentage))} aria-valuetext={`${Math.round(percentage)} percent`}><div {...stylex.props(styles.fill(`${Math.max(0, Math.min(100, percentage))}%`))} /></div>}
    <div data-details-data-row={trace.meters.context.compactions._tag==='Known'||trace.meters.context.lastCompactionMs._tag==='Known'?true:undefined} {...stylex.props(styles.numberRow)}>{trace.meters.context.compactions._tag==='Known'&&<span>Compactions {compactNumber.format(trace.meters.context.compactions.value)}</span>}{trace.meters.context.lastCompactionMs._tag==='Known'&&<span>Last {clock.format(trace.meters.context.lastCompactionMs.value)} UTC</span>}</div>
  </div>
}

/** Loading twin of ContextMeter: formatted footprints wrap naturally under text-shaped ink. */
const ContextMeterPending = ({ footprint, showUnavailableReasons }: { readonly footprint: DetailsPendingShape['context']['meter']; readonly showUnavailableReasons: boolean }): ReactNode => <div {...stylex.props(styles.contextSnapshot)}>
  <div {...stylex.props(styles.numberRow)}>{footprint.percentage !== undefined && <strong><span {...stylex.props(styles.pendingText)}>{footprint.percentage}</span></strong>}</div>
  {footprint.percentage !== undefined ? <div {...stylex.props(styles.track)} /> : showUnavailableReasons && <p {...stylex.props(styles.unknown)}><span {...stylex.props(styles.pendingText)}>Context occupancy unavailable</span></p>}
  <div {...stylex.props(styles.numberRow)}>{footprint.compactions !== undefined && <span><span {...stylex.props(styles.pendingText)}>{footprint.compactions}</span></span>}{footprint.lastCompaction !== undefined && <span><span {...stylex.props(styles.pendingText)}>{footprint.lastCompaction}</span></span>}</div>
</div>


const scopeLabel = (scope:TraceSpendScope):string => scope._tag==='Self'?' · self':scope._tag==='IncludingSubagents'?' · including subagents':scope._tag==='AgentIncarnations'?' · agent incarnations':''
const spendScopeLabel = (trace:TraceSeries):string => scopeLabel(trace.meters.spend.usdScope)
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
const tokenScopeLabel = (trace:TraceSeries):{readonly label:string;readonly window?:readonly string[]|undefined} => ({
 label:`Cumulative tokens${trace.meters.spend.tokensScope._tag==='Self'?' · this agent':trace.meters.spend.tokensScope._tag==='IncludingSubagents'?' · this agent + subagents':trace.meters.spend.tokensScope._tag==='AgentIncarnations'?' · observed agent incarnations':''}`,
 window:accumulationParts(trace.meters.spend.tokensWindow),
})
const totalLabel=(trace:TraceSeries):string=>`${trace.meters.spend.totalUsdCoverage._tag==='Complete'?'Total':'Recorded spend'}${spendScopeLabel(trace)}`
/** A partial attribution is never rounded into a claim of complete coverage. */
const partialPercent=(fraction:number):number=>{
 if(!Number.isFinite(fraction)||fraction<0||fraction>1)throw new RangeError('Attribution fraction must be finite and between 0 and 1')
 return Math.min(99,Math.floor(fraction*100))
}
const coverageLabel=(coverage:TraceCoverage):string|undefined=>coverage._tag==='Complete'?undefined:coverage._tag==='Partial'?`Partial · ${partialPercent(coverage.attributedFraction)}% attributed`:coverage._tag==='NotCovered'?`Not tracked for ${coverage.harnessName}`:'Attribution coverage unavailable'
export const TraceCoverageNote = ({trace,coverage=trace.coverage}:{readonly trace:TraceSeries;readonly coverage?:TraceCoverage}):ReactNode => {
 if(coverage._tag==='Complete')return null
 if(coverage._tag==='NotCovered')return <TooltipTrigger delay={0}><Button data-details-coverage {...stylex.props(styles.coverageTrigger)}>{coverageLabel(coverage)}</Button><Tooltip {...stylex.props(styles.coverageTooltip)}>Spend is recorded only for OMP sessions today.</Tooltip></TooltipTrigger>
 return <p data-details-coverage {...stylex.props(styles.scopeNote)}>{coverageLabel(coverage)}</p>
}

/** Both live and pending rows use the same reported window and wrapping boundaries. */
const MetricWindow = ({ parts, pending = false, tokens = false }: { readonly parts: readonly string[]; readonly pending?: boolean;readonly tokens?:boolean }): ReactNode =>
  <span data-details-metric-period={pending?undefined:true} data-details-token-period={pending||!tokens ? undefined : true} {...stylex.props(styles.tokenWindow)}>{parts.map((part,index)=><span key={index}>{index>0?<wbr/>:null}<span {...stylex.props(styles.windowPart)}><span {...stylex.props(pending&&styles.pendingText)}>{part}</span></span></span>)}</span>

const UsdRow = ({trace,metric,label,fact,scope,window,coverage}:{readonly trace:TraceSeries;readonly metric:string;readonly label:string;readonly fact:TraceFact<number>;readonly scope:TraceSpendScope;readonly window:TraceFact<TraceAccumulationWindow>;readonly coverage:TraceCoverage}):ReactNode => {
 if(fact._tag!=='Known')return null
 const parts=accumulationParts(window)
 return <div data-details-metric={metric} {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}>{label}{scopeLabel(scope)}{parts!==undefined&&<MetricWindow parts={parts}/>}<TraceCoverageNote trace={trace} coverage={coverage}/></dt><dd {...stylex.props(styles.amount)}>{money.format(fact.value)}</dd></div>
}
const SpendMeter = ({ trace, showUnavailableReasons }: { readonly trace: TraceSeries; readonly showUnavailableReasons: boolean }): ReactNode => {
  const spend = trace.meters.spend
  const tokenLabel = tokenScopeLabel(trace)
  return <div {...stylex.props(styles.body)}><dl {...stylex.props(styles.spendSnapshot)} aria-label="Trace spend meters">
    <UsdRow trace={trace} metric="total-usd" label={spend.totalUsdCoverage._tag==='Complete'?'Total':'Recorded spend'} fact={spend.totalUsd} scope={spend.usdScope} window={spend.usdWindow} coverage={spend.totalUsdCoverage}/>
    <UsdRow trace={trace} metric="main-usd" label="Main session" fact={spend.mainUsd} scope={spend.mainUsdScope} window={spend.mainUsdWindow} coverage={spend.mainUsdCoverage}/>
    <UsdRow trace={trace} metric="subagent-usd" label={spend.subagentUsdCoverage._tag==='Partial'?'Attributed subagents':'Subagents'} fact={spend.subagentUsd} scope={spend.subagentUsdScope} window={spend.subagentUsdWindow} coverage={spend.subagentUsdCoverage}/>
    {spend.tokens._tag==='Known'&&<div {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}><span data-details-token-label>{tokenLabel.label}</span>{tokenLabel.window!==undefined&&<MetricWindow parts={tokenLabel.window} tokens/>}</dt><dd {...stylex.props(styles.tokens)}>{compactNumber.format(spend.tokens.value)}</dd></div>}
  </dl>
  {showUnavailableReasons&&spend.tokensScope._tag==='Unknown'&&spend.tokens._tag==='Known'&&<p data-details-scope-note {...stylex.props(styles.scopeNote)}>Scope of this total isn't reported by this source.</p>}
  <TraceCoverageNote trace={trace}/>
  {trace.subagents._tag==='Known'&&trace.subagents.value.length>0&&<p data-details-subagent-spend-window {...stylex.props(styles.scopeNote)}>Traced sessions · {accumulationParts(trace.subagentsWindow)?.join('')??'Accumulation window is unreported.'}</p>}
  {showUnavailableReasons&&[spend.totalUsd,spend.mainUsd,spend.subagentUsd,spend.tokens].some(fact=>fact._tag==='Unknown')&&<UnknownSeries reason="Unreported spend metrics are omitted" />}{trace.subagents._tag === 'Known' && trace.subagents.value.length > 0 && <dl {...stylex.props(styles.spendSnapshot)} aria-label="Subagent spend breakdown">{trace.subagents.value.map((child) => <div key={child.sessionId} {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}>{child.label._tag==='Known'?child.label.value:child.sessionId}</dt><dd {...stylex.props(styles.amount)}>{money.format(child.usd)}</dd></div>)}</dl>}</div>
}

/** Loading twin of SpendMeter for the populated (attributed, subagent-bearing) shape. */
const SpendMeterPending = ({ footprint }: { readonly footprint: DetailsPendingShape['spend'] }): ReactNode => <div aria-hidden="true" inert {...stylex.props(styles.body)}>
  <dl {...stylex.props(styles.spendSnapshot)}>
    {footprint.rows.map(row=><div key={row.label} {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}><span {...stylex.props(styles.pendingText)}>{row.label}</span>{row.window!==undefined&&<MetricWindow parts={row.window} pending tokens={row.tokens}/>} {row.coverage!==undefined&&<p {...stylex.props(styles.scopeNote)}><span {...stylex.props(styles.pendingText)}>{row.coverage}</span></p>}</dt><dd {...stylex.props(row.tokens?styles.tokens:styles.amount)}><span {...stylex.props(styles.pendingText)}>{row.value}</span></dd></div>)}
  </dl>
  {footprint.coverage!==undefined&&<p {...stylex.props(styles.scopeNote)}><span {...stylex.props(styles.pendingText)}>{footprint.coverage}</span></p>}
  {footprint.subagents.length>0&&footprint.subagentWindow!==undefined&&<p {...stylex.props(styles.scopeNote)}><span {...stylex.props(styles.pendingText)}>{footprint.subagentWindow}</span></p>}
  {footprint.subagents.length>0&&<dl {...stylex.props(styles.spendSnapshot)}>{footprint.subagents.map((row,index)=><div key={index} {...stylex.props(styles.spendRow)}><dt {...stylex.props(styles.label)}><span {...stylex.props(styles.pendingText)}>{row.label}</span></dt><dd {...stylex.props(styles.amount)}><span {...stylex.props(styles.pendingText)}>{row.value}</span></dd></div>)}</dl>}
</div>

const hasSpendMeterData = (trace: TraceSeries): boolean => {
  const spend = trace.meters.spend
  return spend.totalUsd._tag === 'Known' || spend.tokens._tag === 'Known'
    || spend.mainUsd._tag === 'Known' || spend.subagentUsd._tag === 'Known'
    || (trace.subagents._tag === 'Known' && trace.subagents.value.length > 0)
}

const hasReportedSplit = (trace: TraceSeries): boolean => trace.meters.spend.mainUsdScope._tag !== 'Unknown'
  && trace.meters.spend.split._tag === 'Known' && Object.values(trace.meters.spend.split.value).every(Number.isFinite)

/** Match the selected chart's usable rows, not merely the existence of a provider. */
export const hasDetailsChartData = ({ trace, context, spend, section, empty = 'E1' }: DetailsChartsProps): boolean => {
  const projection = adaptTrace(trace)
  const samples = projection?.lane.turnSamples !== undefined && (context === 'C2' || projection.lane.samples.length === 0) ? projection.lane.turnSamples : projection?.lane.samples
  const contextData = contextPercent(trace.meters.context)!==undefined || trace.meters.context.compactions._tag==='Known' || trace.meters.context.lastCompactionMs._tag==='Known'
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

const PartialNote = ({ reason }: { readonly reason: string }): ReactNode => <p data-details-partial-note {...stylex.props(styles.unknown)}>{reason}</p>

const tokenScopeReason = (trace: TraceSeries): string | undefined => {
  const { tokens, tokensScope, tokensWindow } = trace.meters.spend
  if (tokens._tag !== 'Known') return undefined
  return tokensScope._tag === 'Unknown' ? "Scope of this total isn't reported by this source."
    : tokensWindow._tag === 'Unknown' ? "Accumulation window isn't reported by this source." : undefined
}
const SectionReason = ({ section, trace }: { readonly section: 'context' | 'spend'; readonly trace: TraceSeries }): ReactNode => {
  const facts = section === 'context' ? Object.values(trace.meters.context) : [trace.meters.spend.totalUsd, trace.meters.spend.mainUsd, trace.meters.spend.subagentUsd, trace.meters.spend.tokens]
  const label = section === 'context' ? 'Context' : 'Spend'
  const reason = facts.some(fact => fact._tag === 'Unknown' && fact.reason === 'loading') ? `${label} values are still loading.`
    : facts.some(fact => fact._tag === 'Unknown' && fact.reason === 'failed') ? `${label} values could not be loaded.`
    : facts.some(fact => fact._tag === 'Unknown' && fact.reason === 'out-of-retention') ? `${label} values are no longer retained.`
    : `This source does not report ${section} values.`
  return <p data-details-reason={`${section} values unavailable`} {...stylex.props(styles.unknown)}>{reason}</p>
}

/** Meters and drawing evidence share one portable Known/Unknown contract. */
export const DetailsCharts = ({ trace, context, spend, section, empty = 'E1', showUnavailableReasons = true }: DetailsChartsProps): ReactNode => {
  const projection = adaptTrace(trace)
  const seriesReason = trace.series._tag === 'Unknown' ? seriesExplanation[trace.series.reason] : 'No historical samples were retained.'
  const spendProjection = trace.meters.spend.mainUsdScope._tag === 'Unknown' ? undefined : projection
  const spendReason = trace.meters.spend.mainUsdScope._tag === 'Unknown' ? 'Session spend scope is not attributed' : seriesReason
  if (empty === 'E3') {
    const contextData = hasDetailsChartData({ trace, context, spend, section: 'context', empty })
    const spendData = hasDetailsChartData({ trace, context, spend, section: 'spend', empty })
    const meters = trace.meters.spend
    const spendNote = tokenScopeReason(trace)
    const contextBody = <div {...stylex.props(styles.body)} data-details-chart={context}>
      {contextData ? <>
        <ContextMeter trace={trace} showUnavailableReasons={false} />
        {context !== 'C1' && projection !== undefined && <ContextPanel scene={projection.scene} lane={projection.lane} compact={context === 'C2'} showUnavailableReasons={false} />}
      </> : <SectionReason section="context" trace={trace} />}
    </div>
    // Charts supplement reported meter rows rather than replacing them.
    const compactSeries = spend === 'S2' && spendProjection !== undefined && (spendProjection.lane.cumulativeValues.length > 0 || spendProjection.descendantSpend.cumulativeValues.length > 0)
    const panelSeries = spend === 'S3' && spendProjection !== undefined && spendProjection.lane.cumulativeValues.length > 0
    const spendBody = <div {...stylex.props(styles.body)} data-details-chart={spend}>
      {spendData ? <>
        {hasSpendMeterData(trace) && <SpendMeter trace={trace} showUnavailableReasons={false} />}
        {compactSeries && <CompactSpend lane={spendProjection.lane} descendantSpend={spendProjection.descendantSpend} showTotals={false} showUnavailableReasons={false} />}
        {panelSeries && <SpendPanel scene={spendProjection.scene} lane={spendProjection.lane} {...(hasReportedSplit(trace) && meters.split._tag === 'Known' ? { split: meters.split.value } : {})} componentsKnown={hasReportedSplit(trace)} showTotals={false} showUnavailableReasons={false} />}
        {spend === 'S3' && !panelSeries && hasReportedSplit(trace) && <SpendPanel {...(meters.split._tag === 'Known' ? { split: meters.split.value } : {})} componentsKnown showUnavailableReasons={false} showTotals={false} />}
        {spendNote !== undefined && <PartialNote reason={spendNote} />}
      </> : <><TraceCoverageNote trace={trace}/><SectionReason section="spend" trace={trace} /></>}
    </div>
    if (section === 'context') return contextBody
    if (section === 'spend') return spendBody
    return <div {...stylex.props(styles.root)}><section aria-label="Context"><h3 {...stylex.props(styles.heading)}>Context</h3>{contextBody}</section><section aria-label="Spend"><h3 {...stylex.props(styles.heading)}>Spend</h3>{spendBody}</section></div>
  }
  const contextBody = <div {...stylex.props(styles.body)} data-details-chart={context}>
    <ContextMeter trace={trace} showUnavailableReasons={showUnavailableReasons} />
    {context !== 'C1' && (projection ? <ContextPanel scene={projection.scene} lane={projection.lane} compact={context === 'C2'} showUnavailableReasons={showUnavailableReasons} /> : showUnavailableReasons && <UnknownSeries reason={`Context series · ${seriesReason}`} />)}
  </div>
  const spendBody = <div {...stylex.props(styles.body)} data-details-chart={spend}>
    <SpendMeter trace={trace} showUnavailableReasons={showUnavailableReasons} />
    {spend !== 'S1' && (spendProjection ? spend === 'S2' ? <CompactSpend lane={spendProjection.lane} descendantSpend={spendProjection.descendantSpend} showTotals={false} showUnavailableReasons={showUnavailableReasons} /> : <SpendPanel scene={spendProjection.scene} lane={spendProjection.lane} componentsKnown={trace.meters.spend.split._tag === 'Known'} showTotals={false} showUnavailableReasons={showUnavailableReasons} /> : <>{showUnavailableReasons&&<UnknownSeries reason={`Spend series · ${spendReason}`} />}{showUnavailableReasons&&spend === 'S3' && <UnknownSeries reason="Four-way spend split is unavailable because attributed component samples were not supplied." />}</>)}
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
      readonly compactions: string | undefined
      readonly lastCompaction: string | undefined
    }
    readonly compactPanel: ContextPanelFootprint | undefined
    readonly fullPanel: ContextPanelFootprint | undefined
  }
  readonly spend: {
    readonly rows: readonly { readonly label: string; readonly window?: readonly string[]|undefined; readonly coverage?:string|undefined; readonly value: string; readonly tokens: boolean }[]
    readonly subagents: readonly { readonly label: string; readonly value: string }[]
    readonly subagentWindow?:string|undefined
    readonly partialNote?: string | undefined
    readonly coverage?: string | undefined
  }
}

export const pendingShapeFromTrace = (trace: TraceSeries): DetailsPendingShape => {
  const projection = adaptTrace(trace)
  const { compactions, lastCompactionMs } = trace.meters.context
  const percentage = contextPercent(trace.meters.context)
  return {
    context: {
      meter: {
        percentage: percentage !== undefined ? `${Math.round(percentage)}%` : undefined,
        compactions: compactions._tag === 'Known' ? `Compactions ${compactNumber.format(compactions.value)}` : undefined,
        lastCompaction: lastCompactionMs._tag === 'Known' ? `Last ${clock.format(lastCompactionMs.value)} UTC` : undefined,
      },
      compactPanel: projection !== undefined ? contextPanelFootprint(projection.lane, true) : undefined,
      fullPanel: projection !== undefined ? contextPanelFootprint(projection.lane, false) : undefined,
    },
    spend: {
      rows: [
        ...(trace.meters.spend.totalUsd._tag==='Known'?[{label:totalLabel(trace),window:accumulationParts(trace.meters.spend.usdWindow),coverage:coverageLabel(trace.meters.spend.totalUsdCoverage),value:money.format(trace.meters.spend.totalUsd.value),tokens:false}]:[]),
        ...(trace.meters.spend.mainUsd._tag==='Known'?[{label:`Main session${scopeLabel(trace.meters.spend.mainUsdScope)}`,window:accumulationParts(trace.meters.spend.mainUsdWindow),coverage:coverageLabel(trace.meters.spend.mainUsdCoverage),value:money.format(trace.meters.spend.mainUsd.value),tokens:false}]:[]),
        ...(trace.meters.spend.subagentUsd._tag==='Known'?[{label:`${trace.meters.spend.subagentUsdCoverage._tag==='Partial'?'Attributed subagents':'Subagents'}${scopeLabel(trace.meters.spend.subagentUsdScope)}`,window:accumulationParts(trace.meters.spend.subagentUsdWindow),coverage:coverageLabel(trace.meters.spend.subagentUsdCoverage),value:money.format(trace.meters.spend.subagentUsd.value),tokens:false}]:[]),
        ...(trace.meters.spend.tokens._tag==='Known'?[{...tokenScopeLabel(trace),value:compactNumber.format(trace.meters.spend.tokens.value),tokens:true}]:[]),
      ],
      subagents: trace.subagents._tag==='Known'?trace.subagents.value.map(child=>({label:child.label._tag==='Known'?child.label.value:child.sessionId,value:money.format(child.usd)})):[],
      subagentWindow:`Traced sessions · ${accumulationParts(trace.subagentsWindow)?.join('')??'Accumulation window is unreported.'}`,
      partialNote: tokenScopeReason(trace),
      coverage: coverageLabel(trace.coverage),
    },
  }
}

/**
 * Loading bodies for the pane's collapsible Context and Spend sections. They
 * mirror the populated per-variant structure (wrappers, gaps, chart aspect
 * ratios, row counts) so arrival cannot shift either section; the pane's one
 * SyncLine connection status is the only announced loading text.
 */
export const DetailsChartsPending = ({ context, spend, section, shape, empty = 'E1' }: {
  readonly context: 'C1' | 'C2' | 'C3'
  readonly spend: 'S1' | 'S2' | 'S3'
  readonly section?: 'context' | 'spend'
  readonly shape: DetailsPendingShape
  /** E3 retains the spend meter above populated charts; E1 keeps its chart-only shape. */
  readonly empty?: DetailsEmptyRule
}): ReactNode => {
  const panelFootprint = context === 'C2' ? shape.context.compactPanel : shape.context.fullPanel
  const contextBody = <div aria-hidden="true" inert data-details-skeleton="context" data-details-chart={context} {...stylex.props(styles.body)}>
    <ContextMeterPending footprint={shape.context.meter} showUnavailableReasons={empty !== 'E3'} />
    {context !== 'C1' && panelFootprint !== undefined && <ContextPanelPending compact={context === 'C2'} footprint={panelFootprint} />}
  </div>
  const spendBody = <div aria-hidden="true" data-details-skeleton="spend" data-details-chart={spend} {...stylex.props(styles.body)}>
    <SpendMeterPending footprint={shape.spend} />
    {spend === 'S2' && <CompactSpendPending showTotals={false}/>}
    {spend === 'S3' && <SpendPanelPending showTotals={false}/>}
    {empty === 'E3' && shape.spend.partialNote !== undefined && <p {...stylex.props(styles.unknown)}><span {...stylex.props(styles.pendingText)}>{shape.spend.partialNote}</span></p>}
  </div>
  if (section === 'context') return contextBody
  if (section === 'spend') return spendBody
  return <div {...stylex.props(styles.root)} data-details-skeleton><section aria-label="Context"><h3 {...stylex.props(styles.heading)}>Context</h3>{contextBody}</section><section aria-label="Spend"><h3 {...stylex.props(styles.heading)}>Spend</h3>{spendBody}</section></div>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', gap: space.lg, minWidth: 0 },
  body: { display: 'flex', flexDirection: 'column', gap: space.md, minWidth: 0, color: text.fg, fontSize: type.metaSize, lineHeight: type.metaLeading },
  heading: { margin: 0, marginBottom: space.md, fontSize: type.metaSize, lineHeight: type.metaLeading, fontWeight: type.weightMedium, color: text.fg },
  unknown: { margin: 0, color: text.fgSoft, fontSize: type.denseSize, lineHeight: type.metaLeading },
  scopeNote: { margin: 0, color: text.fgSoft, fontSize: type.denseSize, lineHeight: type.metaLeading },
  coverageTrigger: { appearance:'none', borderWidth:0, padding:0, backgroundColor:surface.transparent, textAlign:'left', color:text.fgSoft, fontSize:type.denseSize, cursor:'help', outlineColor:accent.primary, outlineOffset:2 },
  coverageTooltip: { backgroundColor:surface.canvas, color:text.fg, borderWidth:1, borderStyle:'solid', borderColor:border.borderStrong, padding:space.md, fontSize:type.metaSize, maxWidth:280 },
  contextSnapshot: { display: 'flex', flexDirection: 'column', gap: space.xs, minWidth: 0 },
  numberRow: { display: 'flex', justifyContent: 'space-between', flexWrap: 'wrap', gap: space.md, fontVariantNumeric: 'tabular-nums' },
  track: { height: geometry.progressMd, backgroundColor: surface.rowActive, borderRadius: radius.full, overflow: 'hidden' },
  fill: (width: string) => ({ height: '100%', width, backgroundColor: accent.primary, borderRadius: radius.full }),
  spendSnapshot: { display: 'flex', flexDirection: 'column', gap: space.xs, margin: 0 },
  spendRow: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) max-content', alignItems: 'baseline', gap: space.md, fontVariantNumeric: 'tabular-nums' },
  label: { color: text.fgMuted, overflowWrap: 'anywhere' },
  tokenWindow: { display: 'block', color: text.fgSoft, fontSize: type.denseSize, lineHeight: type.metaLeading },
  windowPart: { whiteSpace: 'nowrap', overflowWrap: 'normal' },
  pendingText: { color: surface.transparent, backgroundImage: `linear-gradient(${border.borderStrong}, ${border.borderStrong})`, backgroundSize: `100% ${space.md}`, backgroundPosition: 'center', backgroundRepeat: 'no-repeat', boxDecorationBreak: 'clone' },
  amount: { margin: 0, textAlign: 'right' },
  tokens: { margin: 0, color: text.fgMuted },
})
