import * as stylex from '@stylexjs/stylex'
import type { ReactNode } from 'react'
import { accentVars as accent, borderVars as border, spaceVars as space, textVars as text, typeVars as type } from '../../composition-tokens.stylex.ts'
import { clock, money } from './chart.ts'
import type { ChartLane as Lane, TraceChartProjection } from './adapter.ts'
import { DetailsInk, DetailsSkeletonPlot } from '../DetailsSkeleton.tsx'
export interface CompactSpendProps {
  readonly lane: Lane
  /** Role aggregates when the provider cannot attribute marks to individual descendants. */
  readonly descendantSpend: TraceChartProjection['descendantSpend']
  readonly showUnavailableReasons?: boolean
  readonly showTotals?: boolean
}

/** Ported CompactSpend: separate self / full-subtree cumulative costs and honest retained marks. */
export const CompactSpend = ({ lane, descendantSpend, showUnavailableReasons = true, showTotals = true }: CompactSpendProps): ReactNode => {
  const descendantUsd = descendantSpend.totalUsd
  const descendantSeries = [descendantSpend.cumulativeValues]
  const times = [...new Set([...lane.cumulativeValues.map((point) => point.ts), ...descendantSpend.cumulativeValues.map((point) => point.ts)])].toSorted((a, b) => a - b)
  const start = times[0] ?? 0
  const end = times.at(-1) ?? start
  const maximum = Math.max(0.000001, lane.cumulativeValues.at(-1)?.usd ?? 0, descendantSpend.cumulativeValues.at(-1)?.usd ?? 0)
  const path = (series: ReadonlyArray<ReadonlyArray<{ readonly ts: number; readonly usd: number }>>): string => {
    if (!showUnavailableReasons) return series[0]!.map((point, index) => {
      const x = end === start ? 160 : 4 + ((point.ts - start) / (end - start)) * 312
      const y = 76 - point.usd / maximum * 72
      return `${index === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`
    }).join(' ')
    const cursors = series.map(() => 0)
    const values = series.map(() => 0)
    return times.map((ts, index) => {
      for (let i = 0; i < series.length; i++) {
        const points = series[i]!
        while (cursors[i]! < points.length && points[cursors[i]!]!.ts <= ts) {
          values[i] = points[cursors[i]!]!.usd
          cursors[i]!++
        }
      }
      const x = end === start ? 160 : 4 + ((ts - start) / (end - start)) * 312
      const y = 76 - (values.reduce((total, value) => total + value, 0) / maximum) * 72
      return `${index === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`
    }).join(' ')
  }
  const retainedRoot = lane.cumulativeValues.at(-1)?.usd ?? 0
  const retainedDescendants = descendantSpend.cumulativeValues.at(-1)?.usd ?? 0
  const partial = (lane.totalUsd._tag === 'Known' && Math.abs(retainedRoot - lane.totalUsd.value) > 0.005) || (descendantUsd._tag === 'Known' && Math.abs(retainedDescendants - descendantUsd.value) > 0.005)
  const hasSeries = lane.cumulativeValues.length > 0
  const hasDescendantSeries = descendantSpend.cumulativeValues.length > 0
  return <div {...stylex.props(styles.root)} aria-label="Session and descendant spend">
    {showTotals && <div data-details-data-row={lane.totalUsd._tag === 'Known' || descendantUsd._tag === 'Known' ? true : undefined} {...stylex.props(styles.totals)}>{lane.totalUsd._tag === 'Known' && <span {...stylex.props(styles.session)}>Main session <strong>{money.format(lane.totalUsd.value)}</strong></span>}{descendantUsd._tag === 'Known' && <span {...stylex.props(styles.descendants)}>Subagents <strong>{money.format(descendantUsd.value)}</strong></span>}</div>}
    {showTotals && showUnavailableReasons && (lane.totalUsd._tag === 'Unknown' || descendantUsd._tag === 'Unknown') && <p {...stylex.props(styles.note, styles.unknownNote)}>Unreported spend totals are omitted.</p>}
    {hasSeries || (!showUnavailableReasons && hasDescendantSeries) ? <svg {...stylex.props(styles.chart)} viewBox="0 0 320 80" preserveAspectRatio="none" role="img" aria-label="Cumulative session and descendant API-equivalent spend">
      {hasSeries && <path d={path([lane.cumulativeValues])} fill="none" stroke={accent.primary} strokeWidth={1.5} vectorEffect="non-scaling-stroke" />}
      {hasDescendantSeries && <path d={path(descendantSeries)} fill="none" stroke={text.fgSoft} strokeWidth={1.5} strokeDasharray="4 2" vectorEffect="non-scaling-stroke" />}
    </svg> : showUnavailableReasons && <p {...stylex.props(styles.note, styles.unknownNote)}>Session spend series was not retained.</p>}
    {showUnavailableReasons && !hasDescendantSeries && <p {...stylex.props(styles.note, styles.unknownNote)}>Descendant spend series was not retained.</p>}
    {(hasSeries || (!showUnavailableReasons && hasDescendantSeries)) && <div {...stylex.props(styles.axis)}><span>{clock.format(start)}</span><span>UTC</span><span>{clock.format(end)}</span></div>}
    <p {...stylex.props(styles.note)}>API-equivalent · role aggregates{partial ? ' · chart shows retained marks; reported totals are exact' : ''}</p>
</div>
}

/** Loading twin of CompactSpend: same wrappers, gaps and 4:1 chart box, ink instead of values. */
export const CompactSpendPending = (): ReactNode => <div {...stylex.props(styles.root)}>
  <div {...stylex.props(styles.totals)}><span {...stylex.props(styles.session)}><span {...stylex.props(styles.pendingText)}>Main session <strong>$0.00</strong></span></span><span {...stylex.props(styles.descendants)}><span {...stylex.props(styles.pendingText)}>Subagents <strong>$0.00</strong></span></span></div>
  <DetailsSkeletonPlot spend />
  <div {...stylex.props(styles.axis)}><span><DetailsInk ch={5} /></span><span><span {...stylex.props(styles.pendingText)}>UTC</span></span><span><DetailsInk ch={5} /></span></div>
  <p {...stylex.props(styles.note)}><span {...stylex.props(styles.pendingText)}>API-equivalent · role aggregates</span></p>
</div>


const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', gap: space.xs, minWidth: 0, color: text.fg, fontSize: type.metaSize, lineHeight: type.metaLeading },
  totals: { display: 'flex', justifyContent: 'space-between', flexWrap: 'wrap', gap: space.md, fontVariantNumeric: 'tabular-nums' },
  session: { color: text.fg }, descendants: { color: text.fgSoft },
  chart: { display: 'block', width: '100%', height: 'auto', aspectRatio: '4 / 1' },
  note: { margin: 0, color: text.fgMuted, fontSize: type.denseSize },
  pendingText: { color: 'transparent', backgroundImage: `linear-gradient(${border.borderStrong}, ${border.borderStrong})`, backgroundSize: `100% ${space.md}`, backgroundPosition: 'center', backgroundRepeat: 'no-repeat', boxDecorationBreak: 'clone' },
  unknownNote: { color: text.fgSoft },
  axis: { display: 'flex', justifyContent: 'space-between', color: text.fgMuted, fontSize: type.microSize, fontVariantNumeric: 'tabular-nums' },
})
