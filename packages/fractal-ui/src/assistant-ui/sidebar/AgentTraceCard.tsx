// Explicit-clock presentation only: the host owns the clock, the row and the trace series.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria-components'
import { borderVars as border, geometryNumbers, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { compactTime, type SidebarAgentRow as Row } from './model'
import { Icon } from '../composition/Icons'
import { absenceNote, type AgentTrace, type ContextHistory, type SpendPoint, type SpendTotals, type TokenSplit, type TraceSubagent } from './trace-model'

const tokenCount = Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 })
const usd = Intl.NumberFormat('en', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 2 })
const percent = (part: number, whole: number) => Math.round((part / whole) * 100)
const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`
/** Chart drawing space; the SVGs scale to the card width. */
const chart = { width: 328, histogram: 48, spend: 40 } as const

export type AgentTraceCardProps = { readonly row: Row; readonly trace: AgentTrace; readonly now: number }
/** The card sits right of its row when the viewport has room for it, otherwise below the row (narrow windows), so it never runs off-screen. */
export function traceCardPlacement(anchor: HTMLElement | null): 'right' | 'bottom start' {
  if (anchor === null) return 'right'
  return anchor.getBoundingClientRect().right + geometryNumbers.tooltipOffset + geometryNumbers.panelMin + geometryNumbers.tooltipOffset <= window.innerWidth ? 'right' : 'bottom start'
}

/** Peak, marks and idle gaps of a context history; shared by the visible head and the chart label. */
function contextStats(history: ContextHistory) {
  const peaks = history.buckets.flatMap(bucket => bucket.usedTokens === undefined ? [] : [percent(bucket.usedTokens, bucket.windowTokens)])
  return {
    peak: peaks.length === 0 ? undefined : Math.max(...peaks),
    compactions: history.marks.filter(mark => mark.kind === 'compaction').length,
    changes: history.marks.filter(mark => mark.kind === 'model-change').length,
    gaps: history.buckets.filter((bucket, index) => bucket.usedTokens === undefined && history.buckets[index - 1]?.usedTokens !== undefined).length,
  }
}

function Note({ children }: { readonly children: React.ReactNode }) {
  return <p data-trace-note {...stylex.props(styles.note)}>{children}</p>
}
function Pending({ label, height }: { readonly label: string; readonly height: 'histogram' | 'spend' | 'bar' }) {
  return <div role="img" aria-label={label} aria-busy="true" data-trace-pending {...stylex.props(styles.pending, height === 'histogram' ? styles.histogram : height === 'spend' ? styles.spend : styles.pendingBar)} />
}
function SectionHead({ title, children }: { readonly title: string; readonly children?: React.ReactNode }) {
  return <div {...stylex.props(styles.sectionHead)}><span {...stylex.props(styles.sectionTitle)}>{title}</span>{children !== undefined && <span {...stylex.props(styles.sectionMeta)}>{children}</span>}</div>
}

function ContextHistogram({ history, range, label }: { readonly history: ContextHistory; readonly range: AgentTrace['range']; readonly label: string }) {
  const span = Math.max(1, range.end - range.start)
  const x = (at: number) => ((at - range.start) / span) * chart.width
  return <figure {...stylex.props(styles.figure)}>
    <svg role="img" aria-label={label} data-trace-chart="context" viewBox={`0 0 ${chart.width} ${chart.histogram}`} preserveAspectRatio="none" {...stylex.props(styles.svg, styles.histogram)}>
      <line x1={0} x2={chart.width} y1={0.5} y2={0.5} vectorEffect="non-scaling-stroke" {...stylex.props(styles.ceiling)} />
      {history.buckets.map(bucket => {
        if (bucket.usedTokens === undefined) return null
        const height = Math.min(1, bucket.usedTokens / bucket.windowTokens) * chart.histogram
        return <rect key={bucket.start} x={x(bucket.start)} width={Math.max(0.5, x(bucket.end) - x(bucket.start) - 0.5)} y={chart.histogram - height} height={height} {...stylex.props(styles.bar)} />
      })}
      {history.marks.map(mark => <line key={`${mark.kind}-${mark.at}`} data-trace-mark={mark.kind} x1={x(mark.at)} x2={x(mark.at)} y1={0} y2={chart.histogram} vectorEffect="non-scaling-stroke" {...stylex.props(mark.kind === 'compaction' ? styles.compaction : styles.modelChange)} />)}
    </svg>
    <figcaption aria-hidden="true" {...stylex.props(styles.axis)}>
      <span>24h ago</span>
      <span {...stylex.props(styles.legend)}><span {...stylex.props(styles.key, styles.keyCompaction)} />compaction<span {...stylex.props(styles.key, styles.keyModel)} />model change</span>
      <span>now</span>
    </figcaption>
  </figure>
}

function SpendLine({ points, range, label }: { readonly points: readonly SpendPoint[]; readonly range: AgentTrace['range']; readonly label: string }) {
  const span = Math.max(1, range.end - range.start)
  const top = Math.max(0.01, ...points.map(point => Math.max(point.sessionUsd, point.subagentsUsd)))
  const path = (pick: (point: SpendPoint) => number) => points.map(point => `${((point.at - range.start) / span) * chart.width},${chart.spend - 1 - (pick(point) / top) * (chart.spend - 2)}`).join(' ')
  return <svg role="img" aria-label={label} data-trace-chart="spend" viewBox={`0 0 ${chart.width} ${chart.spend}`} preserveAspectRatio="none" {...stylex.props(styles.svg, styles.spend)}>
    <polyline points={path(point => point.sessionUsd)} vectorEffect="non-scaling-stroke" {...stylex.props(styles.sessionLine)} />
    <polyline points={path(point => point.subagentsUsd)} vectorEffect="non-scaling-stroke" {...stylex.props(styles.subagentLine)} />
  </svg>
}

const splitParts = [
  { key: 'input', label: 'Input', style: 'input' },
  { key: 'cacheRead', label: 'Cache read', style: 'cacheRead' },
  { key: 'cacheWrite', label: 'Cache write', style: 'cacheWrite' },
  { key: 'output', label: 'Output', style: 'output' },
] as const
function TokenSplitBar({ split }: { readonly split: TokenSplit }) {
  const total = split.input + split.cacheRead + split.cacheWrite + split.output
  const parts = splitParts.map(part => ({ ...part, share: total === 0 ? 0 : percent(split[part.key], total) }))
  const label = `Tokens ${tokenCount.format(total)}: ${parts.map(part => `${part.label.toLowerCase()} ${part.share}%`).join(', ')}`
  return <>
    <div role="img" aria-label={label} data-trace-chart="tokens" {...stylex.props(styles.splitBar)}>
      {parts.map(part => <span key={part.key} {...stylex.props(styles.segment, styles[part.style], styles.grow(split[part.key]))} />)}
    </div>
    <ul aria-hidden="true" {...stylex.props(styles.splitLegend)}>
      {parts.map(part => <li key={part.key} {...stylex.props(styles.legendItem)}><span {...stylex.props(styles.key, styles.keySwatch, styles[part.style])} />{part.label} {part.share}%</li>)}
    </ul>
  </>
}

function ContextSection({ row, trace }: { readonly row: Row; readonly trace: AgentTrace }) {
  const reading = trace.contextReading
  const history = trace.context._tag === 'Known' ? trace.context.value : undefined
  const stats = history === undefined ? undefined : contextStats(history)
  const compactions = stats?.compactions ?? reading?.compactions
  if (trace.context._tag === 'Unknown' && reading === undefined) return <section data-trace-section="context" aria-label="Context" {...stylex.props(styles.section)}><SectionHead title="Context" /><Note>{absenceNote(trace.context.reason)}</Note></section>
  const label = [reading === undefined ? 'Context' : `Context ${percent(reading.usedTokens, reading.windowTokens)}% of ${tokenCount.format(reading.windowTokens)}`, stats?.peak === undefined ? undefined : `peak ${stats.peak}%`, compactions === undefined ? undefined : plural(compactions, 'compaction'), stats === undefined || stats.changes === 0 ? undefined : plural(stats.changes, 'model change'), stats === undefined || stats.gaps === 0 ? undefined : plural(stats.gaps, 'idle gap')].filter((part): part is string => part !== undefined).join(', ')
  return <section data-trace-section="context" aria-label="Context" {...stylex.props(styles.section)}>
    <SectionHead title="Context">{reading !== undefined && <><strong {...stylex.props(styles.strong)}>{percent(reading.usedTokens, reading.windowTokens)}%</strong> · {tokenCount.format(reading.usedTokens)} / {tokenCount.format(reading.windowTokens)}</>}{stats?.peak !== undefined && ` · max ${stats.peak}%`}{compactions !== undefined && ` · ${plural(compactions, 'compaction')}`}</SectionHead>
    {history !== undefined && <ContextHistogram history={history} range={trace.range} label={`${label}, over the last 24 hours`} />}
    {trace.context._tag === 'Pending' && <Pending label={`Loading context history for ${row.title}`} height="histogram" />}
    {trace.context._tag === 'Unknown' && <Note>{absenceNote(trace.context.reason)}</Note>}
  </section>
}

function SpendSection({ row, trace }: { readonly row: Row; readonly trace: AgentTrace }) {
  const totals: SpendTotals | undefined = trace.spendTotals._tag === 'Known' ? trace.spendTotals.value : undefined
  const scalar = row.usage._tag === 'Known' ? row.usage : undefined
  const unattributed = trace.spendTotals._tag === 'Unknown' && trace.spendTotals.reason === 'not-attributed'
  // The cumulative series ends at its own totals, so a Known series stays labelled even before the totals read arrives.
  const last = trace.spend._tag === 'Known' ? trace.spend.value[trace.spend.value.length - 1] : undefined
  const charted = totals ?? last
  const label = charted === undefined ? undefined : `Cumulative spend over the last 24 hours: session ${usd.format(charted.sessionUsd)}, subagents ${usd.format(charted.subagentsUsd)}`
  const scope = scalar?.scope === 'lifetime-incl-subagents' ? 'lifetime, includes subagents' : '24h, includes subagents'
  return <section data-trace-section="spend" aria-label="Spend" {...stylex.props(styles.section)}>
    <SectionHead title={trace.spend._tag === 'Known' ? 'Spend · 24h' : 'Spend'}>
      {totals !== undefined ? <>Session <strong {...stylex.props(styles.strong)}>{usd.format(totals.sessionUsd)}</strong> · Subagents <strong {...stylex.props(styles.strong)}>{usd.format(totals.subagentsUsd)}</strong></> : scalar !== undefined && !unattributed ? <><strong {...stylex.props(styles.strong)}>{usd.format(scalar.usd)}</strong> · {scope}</> : undefined}
    </SectionHead>
    {trace.spend._tag === 'Known' && label !== undefined && <SpendLine points={trace.spend.value} range={trace.range} label={label} />}
    {trace.spend._tag === 'Known' && label !== undefined && <div aria-hidden="true" {...stylex.props(styles.axis)}><span {...stylex.props(styles.legend)}><span {...stylex.props(styles.key, styles.keySession)} />session<span {...stylex.props(styles.key, styles.keySubagents)} />subagents</span></div>}
    {trace.spend._tag === 'Pending' && <Pending label={`Loading spend history for ${row.title}`} height="spend" />}
    {unattributed ? <Note>{absenceNote('not-attributed')}</Note> : trace.spend._tag === 'Unknown' && <Note>{absenceNote(trace.spend.reason)}</Note>}
  </section>
}

function TokenSection({ row, trace }: { readonly row: Row; readonly trace: AgentTrace }) {
  const split = trace.tokens._tag === 'Known' ? trace.tokens.value : undefined
  const total = split !== undefined ? split.input + split.cacheRead + split.cacheWrite + split.output : row.usage._tag === 'Known' ? row.usage.tokens : undefined
  return <section data-trace-section="tokens" aria-label="Tokens" {...stylex.props(styles.section)}>
    <SectionHead title={total === undefined ? 'Tokens' : `Tokens · ${tokenCount.format(total)}`}>{split !== undefined ? 'API-equivalent' : undefined}</SectionHead>
    {split !== undefined && <TokenSplitBar split={split} />}
    {trace.tokens._tag === 'Pending' && <Pending label={`Loading token split for ${row.title}`} height="bar" />}
    {trace.tokens._tag === 'Unknown' && <Note>{absenceNote(trace.tokens.reason)}</Note>}
  </section>
}

function SubagentTable({ subagents, now }: { readonly subagents: readonly TraceSubagent[]; readonly now: number }) {
  const captionId = React.useId()
  const running = subagents.filter(subagent => subagent.state === 'running').length
  return <section data-trace-section="subagents" aria-labelledby={captionId} {...stylex.props(styles.section)}>
    <span id={captionId} {...stylex.props(styles.sectionTitle)}>Subagents · {running > 0 ? `${running} running` : plural(subagents.length, 'subagent')}</span>
    <div role="table" aria-labelledby={captionId} data-trace-table="subagents" {...stylex.props(styles.table)}>
      <div role="rowgroup" {...stylex.props(styles.rowgroup)}>
        <div role="row" {...stylex.props(styles.tableRow, styles.headRow)}>
          <span role="columnheader" {...stylex.props(styles.cell)}>Name</span>
          <span role="columnheader" {...stylex.props(styles.cell)}>State</span>
          <span role="columnheader" {...stylex.props(styles.cell, styles.numeric)}>Spend</span>
          <span role="columnheader" {...stylex.props(styles.cell, styles.numeric)}>Age</span>
        </div>
      </div>
      <div role="rowgroup" {...stylex.props(styles.rowgroup)}>
        {subagents.map(subagent => <div role="row" key={subagent.ref} data-subagent-ref={subagent.ref} {...stylex.props(styles.tableRow)}>
          <span role="cell" {...stylex.props(styles.cell, styles.name)}>{subagent.name}</span>
          <span role="cell" {...stylex.props(styles.cell, styles.soft)}>{subagent.state === 'needs-you' ? 'needs you' : subagent.state}</span>
          <span role="cell" {...stylex.props(styles.cell, styles.numeric, subagent.spendUsd === undefined && styles.muted)}>{subagent.spendUsd === undefined ? 'Not attributed' : usd.format(subagent.spendUsd)}</span>
          <span role="cell" {...stylex.props(styles.cell, styles.numeric)}>{compactTime({ at: subagent.startedAt, now })}</span>
        </div>)}
      </div>
    </div>
  </section>
}

function Facts({ row, trace, now }: AgentTraceCardProps) {
  const { turns, toolCalls, lastTool } = trace.activity
  const activity = [turns === undefined ? undefined : plural(turns, 'turn'), toolCalls === undefined ? undefined : plural(toolCalls, 'tool call'), lastTool === undefined ? undefined : `last tool ${lastTool}`].filter((part): part is string => part !== undefined)
  const branch = [row.branch ?? row.worktree, row.pullRequest === undefined ? undefined : `PR #${row.pullRequest.number}`].filter((part): part is string => part !== undefined)
  return <>
    {(activity.length > 0 || branch.length > 0 || trace.subagents._tag !== 'Known') && <dl data-trace-section="facts" {...stylex.props(styles.facts)}>
      {activity.length > 0 && <><dt {...stylex.props(styles.factLabel)}>Activity</dt><dd {...stylex.props(styles.factValue)}>{activity.join(' · ')}</dd></>}
      {branch.length > 0 && <><dt {...stylex.props(styles.factLabel)}>Branch</dt><dd {...stylex.props(styles.factValue)}>{branch.join(' · ')}</dd></>}
      {trace.subagents._tag === 'Unknown' && <><dt {...stylex.props(styles.factLabel)}>Subagents</dt><dd {...stylex.props(styles.factValue, styles.muted)}>{row.childrenKnown === false ? 'Not reported by this harness' : absenceNote(trace.subagents.reason)}</dd></>}
      {trace.subagents._tag === 'Pending' && <><dt {...stylex.props(styles.factLabel)}>Subagents</dt><dd {...stylex.props(styles.factValue)}><Pending label={`Loading subagents for ${row.title}`} height="bar" /></dd></>}
    </dl>}
    {trace.subagents._tag === 'Known' && trace.subagents.value.length > 0 && <SubagentTable subagents={trace.subagents.value} now={now} />}
  </>
}

/**
 * Sidebar trace card (layout B): header, 24 h context histogram, cumulative spend, token split and compact facts.
 * Every chart renders only for a Known series; otherwise its section collapses to the scalar reading plus the reason it is absent.
 */
export function AgentTraceCard({ row, trace, now }: AgentTraceCardProps) {
  const headerId = React.useId()
  const subline = [row.statusSince === undefined ? row.statusLabel : `${row.statusLabel} · ${compactTime({ at: row.statusSince, now })}`, row.model, row.harness, row.host].filter((part): part is string => part !== undefined)
  return <div data-testid="agent-trace-card" aria-labelledby={headerId} role="group" {...stylex.props(styles.card)}>
    <div {...stylex.props(styles.header)}>
      <span {...stylex.props(styles.glyph, row.status === 'working' && styles.glyphWorking, row.status === 'waiting' && styles.glyphWaiting)}><Icon name="dot" scale="status" /></span>
      <strong id={headerId} {...stylex.props(styles.title)}>{row.title}</strong>
      <span {...stylex.props(styles.subline)}>{subline.join(' · ')}</span>
    </div>
    {row.description !== undefined && <p {...stylex.props(styles.goal)}>{row.description}</p>}
    <ContextSection row={row} trace={trace} />
    <SpendSection row={row} trace={trace} />
    <TokenSection row={row} trace={trace} />
    <Facts row={row} trace={trace} now={now} />
    {trace.context._tag === 'Pending' && <VisuallyHidden><span role="status">Loading trace history</span></VisuallyHidden>}
  </div>
}

const styles = stylex.create({
  card: { display: 'flex', flexDirection: 'column', gap: s.lg, width: g.panelMin, maxWidth: '100%', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', overflowWrap: 'anywhere' },
  header: { display: 'grid', gridTemplateColumns: `${g.icon} minmax(0, 1fr)`, columnGap: s.md, rowGap: s.xs2, alignItems: 'center' },
  glyph: { display: 'flex', alignItems: 'center', justifyContent: 'center', color: ink.fgMuted },
  glyphWorking: { color: tone.runningFg },
  glyphWaiting: { color: tone.attention },
  title: { fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold, color: ink.fg },
  subline: { gridColumn: '2', color: ink.fgSoft },
  goal: { margin: s.zero, color: ink.fgSoft },
  section: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  sectionHead: { display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: s.md, flexWrap: 'wrap' },
  sectionTitle: { color: ink.fg, fontWeight: t.weightMedium },
  sectionMeta: { color: ink.fgMuted, textAlign: 'end' },
  strong: { color: ink.fg, fontWeight: t.weightMedium },
  note: { margin: s.zero, color: ink.fgMuted },
  figure: { margin: s.zero, display: 'flex', flexDirection: 'column', gap: s.xs },
  svg: { display: 'block', width: '100%', overflow: 'visible' },
  histogram: { height: '48px' },
  spend: { height: '40px' },
  bar: { fill: tone.running },
  ceiling: { stroke: ink.fgMuted, strokeWidth: 1, strokeDasharray: '2 3' },
  compaction: { stroke: tone.attention, strokeWidth: 1.5, strokeDasharray: '3 2' },
  modelChange: { stroke: ink.fg, strokeWidth: 1.5 },
  sessionLine: { fill: 'none', stroke: tone.runningFg, strokeWidth: 1.5 },
  subagentLine: { fill: 'none', stroke: ink.fgSoft, strokeWidth: 1.5, strokeDasharray: '4 3' },
  axis: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: s.md, color: ink.fgMuted, fontSize: t.denseSize },
  legend: { display: 'inline-flex', alignItems: 'center', gap: s.xs },
  key: { display: 'inline-block', flexShrink: 0, marginInlineStart: s.xs },
  keyCompaction: { width: 0, height: '10px', borderInlineStartWidth: '2px', borderInlineStartStyle: 'dashed', borderInlineStartColor: tone.attention },
  keyModel: { width: 0, height: '10px', borderInlineStartWidth: '2px', borderInlineStartStyle: 'solid', borderInlineStartColor: ink.fg },
  keySession: { width: '12px', height: 0, borderTopWidth: '2px', borderTopStyle: 'solid', borderTopColor: tone.runningFg },
  keySubagents: { width: '12px', height: 0, borderTopWidth: '2px', borderTopStyle: 'dashed', borderTopColor: ink.fgSoft },
  keySwatch: { width: '8px', height: '8px', marginInlineStart: s.zero, borderRadius: r.keyboard },
  splitBar: { display: 'flex', gap: s.xxxs, height: g.progressMd, borderRadius: r.full, overflow: 'hidden' },
  segment: { display: 'block', minWidth: s.xs2, height: '100%' },
  grow: (value: number) => ({ flexGrow: value, flexBasis: 0 }),
  input: { backgroundColor: tone.runningFg },
  cacheRead: { backgroundColor: ink.fgMuted },
  cacheWrite: { backgroundColor: tone.attention },
  output: { backgroundColor: ink.fg },
  splitLegend: { display: 'flex', flexWrap: 'wrap', columnGap: s.lg, rowGap: s.xs2, margin: s.zero, padding: s.zero, listStyleType: 'none', color: ink.fgMuted, fontSize: t.denseSize },
  legendItem: { display: 'inline-flex', alignItems: 'center', gap: s.xs },
  pending: { borderRadius: r.sm, backgroundColor: surface.rowActive },
  pendingBar: { height: g.progressMd },
  facts: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: s.lg, rowGap: s.xs, margin: s.zero, paddingTop: s.md, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  factLabel: { margin: s.zero, color: ink.fgMuted },
  factValue: { margin: s.zero, minWidth: 0, color: ink.fg },
  table: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) max-content max-content max-content', columnGap: s.lg },
  rowgroup: { display: 'grid', gridColumn: '1 / -1', gridTemplateColumns: 'subgrid' },
  tableRow: { display: 'grid', gridColumn: '1 / -1', gridTemplateColumns: 'subgrid', paddingBlock: s.xs2 },
  headRow: { color: ink.fgMuted, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  cell: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  name: { color: ink.fg },
  soft: { color: ink.fgSoft },
  muted: { color: ink.fgMuted },
  numeric: { textAlign: 'end', fontVariantNumeric: 'tabular-nums' },
})
