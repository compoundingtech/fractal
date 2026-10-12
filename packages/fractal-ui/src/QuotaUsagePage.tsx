import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { quotaGeometry } from './quota-geometry.stylex.ts'
import { Icon } from './assistant-ui/composition/Icons.tsx'
import { Button, Label, ListBox, ListBoxItem, Radio, RadioGroup, Slider, SliderOutput, SliderThumb, SliderTrack, Tab, TabList, TabPanel, Tabs, VisuallyHidden } from 'react-aria-components'
import { accentVars as accent, borderVars as border, radiusVars as radius, spaceVars as space, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import type { QuotaAccount, QuotaEconomics, QuotaHistoryDay, QuotaHistoryValue, QuotaUnit, QuotaUsageProps, QuotaWindow } from './quota-design-types'
import { quotaStateLabels } from './quota-design-copy.ts'
import { FullValueTooltip } from './FullValueTooltip.tsx'

/** USD is only shown when the in-range values carry at least this much USD coverage (TUI rule). */
const usdCoverageGate = 0.95
const chartUnits = 100
const slotUnits = 10
const barUnits = 7
const unknownCapUnits = 6
const seriesTones = 6
const weekdays = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] as const
const detailTabs = [{ id: 'limits', label: 'Limits' }, { id: 'account', label: 'Account' }, { id: 'history', label: 'History' }] as const
type DetailTab = QuotaUsageProps['detailTab']
type StackBy = QuotaUsageProps['stackBy']

/** Exact readouts keep measured sub-cent precision (at least 6 fractional digits); cost tables round to cents. */
const usdExact = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 10 })
const usdCents = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 2 })
const usdCompact = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', notation: 'compact', maximumFractionDigits: 1 })
const tokensExact = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 })
const tokensCompact = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
const countExact = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 })
const percent = new Intl.NumberFormat('en-US', { style: 'percent', maximumFractionDigits: 0 })

/** Known values summed separately from unknown ones; a tally with no rows is neither zero nor unknown. */
interface Tally { readonly sum: number; readonly known: number; readonly unknown: number }
const emptyTally: Tally = { sum: 0, known: 0, unknown: 0 }
const addValue = (tally: Tally, value: number | undefined): Tally => value === undefined ? { ...tally, unknown: tally.unknown + 1 } : { sum: tally.sum + value, known: tally.known + 1, unknown: tally.unknown }
const addTally = (left: Tally, right: Tally): Tally => ({ sum: left.sum + right.sum, known: left.known + right.known, unknown: left.unknown + right.unknown })
const tallyText = (tally: Tally, format: (value: number) => string): string =>
  tally.known === 0 && tally.unknown === 0 ? 'No rows'
    : tally.known === 0 ? 'Unreported'
      : tally.unknown > 0 ? `${format(tally.sum)} + ${tally.unknown} Unreported`
        : format(tally.sum)

const amountOf = (value: QuotaHistoryValue, unit: QuotaUnit): number | undefined => unit === 'usd' ? value.usd : value.tokens
const formatExact = (unit: QuotaUnit) => (value: number): string => unit === 'usd' ? usdExact.format(value) : `${tokensExact.format(value)} tok`
const formatCompact = (unit: QuotaUnit, value: number): string => unit === 'usd' ? usdCompact.format(value) : `${tokensCompact.format(value)} tok`
const formatUsd = (value: number | undefined): string => value === undefined ? 'Unreported' : usdCents.format(value)
const formatUsdExact = (value: number | undefined): string => value === undefined ? 'Unreported' : usdExact.format(value)

interface Series { readonly key: string; readonly label: string; readonly tone: number }
interface DayCell { readonly day: QuotaHistoryDay; readonly bySeries: ReadonlyMap<string, Tally> }

const seriesKeyOf = (value: QuotaHistoryValue, stackBy: StackBy): string => stackBy === 'account' ? value.accountId : value.model

const buildSeries = (days: readonly QuotaHistoryDay[], accounts: readonly QuotaAccount[], stackBy: StackBy): readonly Series[] => {
  const labels = new Map<string, string>()
  for (const day of days) for (const value of day.values) {
    const key = seriesKeyOf(value, stackBy)
    if (!labels.has(key)) labels.set(key, stackBy === 'account' ? accounts.find(account => account.id === key)?.label ?? key : key)
  }
  return [...labels].sort((left, right) => left[1].localeCompare(right[1]) || left[0].localeCompare(right[0])).map(([key, label], index) => ({ key, label, tone: index % seriesTones }))
}

const buildCell = (day: QuotaHistoryDay, unit: QuotaUnit, stackBy: StackBy): DayCell => {
  const bySeries = new Map<string, Tally>()
  for (const value of day.values) {
    const key = seriesKeyOf(value, stackBy)
    bySeries.set(key, addValue(bySeries.get(key) ?? emptyTally, amountOf(value, unit)))
  }
  return { day, bySeries }
}

const totalOf = (cell: DayCell, series: readonly Series[]): Tally => series.reduce((tally, item) => addTally(tally, cell.bySeries.get(item.key) ?? emptyTally), emptyTally)
const partialText = (day: QuotaHistoryDay): string => day.partial ? ' (partial Berlin day, still open)' : ''
const describeDay = (cell: DayCell, series: readonly Series[], unit: QuotaUnit): string => {
  const format = formatExact(unit)
  const parts = series.flatMap(item => {
    const tally = cell.bySeries.get(item.key)
    return tally === undefined ? [] : [`${item.label}: ${tallyText(tally, format)}`]
  })
  return `${cell.day.label}${partialText(cell.day)}. Total ${tallyText(totalOf(cell, series), format)}${parts.length > 0 ? `. ${parts.join('; ')}` : ''}`
}
const scaleOf = (cells: readonly DayCell[], series: readonly Series[]): number => Math.max(0, ...cells.map(cell => totalOf(cell, series).sum)) || 1

/** Civil Berlin calendar date from an ISO `YYYY-MM-DD` id, as a UTC day number; non-date ids return undefined. */
const dayNumberOf = (id: string): number | undefined => {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(id)
  if (match === null) return undefined
  return Math.floor(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])) / 86_400_000)
}
/** Monday-based weekday (0 = Monday) of a UTC day number; 1970-01-01 was a Thursday. */
const weekdayOf = (dayNumber: number): number => (((dayNumber + 3) % 7) + 7) % 7

const clampIndex = (value: number, min: number, max: number): number => Math.min(max, Math.max(min, Math.round(value)))

const fractionTone = (fraction: number | undefined, exhausted: boolean): 'normal' | 'warning' | 'danger' =>
  exhausted || (fraction !== undefined && fraction >= 0.8) ? 'danger' : fraction !== undefined && fraction >= 0.6 ? 'warning' : 'normal'
const windowReading = (window: QuotaWindow): string =>
  window.fraction !== undefined ? `${percent.format(window.fraction)}${window.exhausted ? ' · exhausted' : ''}`
    : window.exhausted ? 'Exhausted · fraction Unreported'
      : 'Unreported'

const isDetailTab = (key: React.Key): key is DetailTab => detailTabs.some(tab => tab.id === key)

const tones = stylex.create({
  fill0: { fill: accent.primary }, fill1: { fill: status.attention }, fill2: { fill: status.dangerFg }, fill3: { fill: status.diffAddTint }, fill4: { fill: ink.fgSoft }, fill5: { fill: ink.fgMuted },
  bg0: { backgroundColor: accent.primary }, bg1: { backgroundColor: status.attention }, bg2: { backgroundColor: status.dangerFg }, bg3: { backgroundColor: status.diffAddTint }, bg4: { backgroundColor: ink.fgSoft }, bg5: { backgroundColor: ink.fgMuted },
  heat0: { opacity: 0.28 }, heat1: { opacity: 0.5 }, heat2: { opacity: 0.74 }, heat3: { opacity: 1 },
})
const toneFill = [tones.fill0, tones.fill1, tones.fill2, tones.fill3, tones.fill4, tones.fill5] as const
const toneBackground = [tones.bg0, tones.bg1, tones.bg2, tones.bg3, tones.bg4, tones.bg5] as const
const heatLevels = [tones.heat0, tones.heat1, tones.heat2, tones.heat3] as const

const Swatch = ({ tone }: { readonly tone: number }) => <span aria-hidden="true" {...stylex.props(styles.swatch, toneBackground[tone])} />

/** Plain SVG stacked bars with one real button per day; the SVG itself is decorative. */
const StackedChart = ({ cells, series, unit, height, label, selectedDay, onDay, onHover }: {
  readonly cells: readonly DayCell[]
  readonly series: readonly Series[]
  readonly unit: QuotaUnit
  readonly height: 'main' | 'small'
  readonly label: string
  readonly selectedDay?: string
  readonly onDay: (day?: string) => void
  readonly onHover: (day?: string) => void
}) => {
  // useId may contain characters that break `url(#…)` references.
  const patternId = `quota-hatch-${React.useId().replace(/[^a-zA-Z0-9_-]/g, '')}`
  const max = scaleOf(cells, series)
  const width = Math.max(1, cells.length) * slotUnits
  const dayDescriptionId = React.useId()
  return <div {...stylex.props(styles.chart, height === 'main' ? styles.chartMain : styles.chartSmall)}>
    <svg aria-hidden="true" viewBox={`0 0 ${width} ${chartUnits}`} preserveAspectRatio="none" {...stylex.props(styles.svg)}>
      <defs><pattern id={patternId} width="3" height="3" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="1" height="3" {...stylex.props(styles.hatchInk)} /></pattern></defs>
      <line x1="0" x2={width} y1={chartUnits - 0.5} y2={chartUnits - 0.5} vectorEffect="non-scaling-stroke" {...stylex.props(styles.baseline)} />
      {cells.map((cell, index) => {
        const x = index * slotUnits + (slotUnits - barUnits) / 2
        const total = totalOf(cell, series)
        const selected = cell.day.id === selectedDay
        if (total.known === 0 && total.unknown > 0) return <rect key={cell.day.id} x={x} y={0} width={barUnits} height={chartUnits} fill={`url(#${patternId})`} vectorEffect="non-scaling-stroke" {...stylex.props(styles.unknownBar)} />
        if (total.sum === 0) return <rect key={cell.day.id} x={x} y={chartUnits - 1.5} width={barUnits} height={1.5} {...stylex.props(total.known === 0 ? styles.emptyTick : styles.zeroTick)} />
        let top = chartUnits
        const segments = series.flatMap(item => {
          const tally = cell.bySeries.get(item.key)
          if (tally === undefined || tally.sum === 0) return []
          const segment = (tally.sum / max) * (chartUnits - unknownCapUnits)
          top -= segment
          return [<rect key={item.key} x={x} y={top} width={barUnits} height={segment} {...stylex.props(toneFill[item.tone], cell.day.partial && styles.partialBar, selectedDay !== undefined && !selected && styles.dimmedBar)} />]
        })
        return <g key={cell.day.id}>
          {segments}
          {total.unknown > 0 && <rect x={x} y={top - unknownCapUnits} width={barUnits} height={unknownCapUnits} fill={`url(#${patternId})`} vectorEffect="non-scaling-stroke" {...stylex.props(styles.unknownBar)} />}
          {cell.day.partial && <rect x={x} y={top} width={barUnits} height={chartUnits - top} vectorEffect="non-scaling-stroke" {...stylex.props(styles.partialOutline)} />}
        </g>
      })}
    </svg>
    <div role="group" aria-label={label} {...stylex.props(styles.dayButtons)} onPointerLeave={() => onHover(undefined)}>
      {cells.map(cell => {
        const description = describeDay(cell, series, unit)
        const selected = cell.day.id === selectedDay
        const describedBy = `${dayDescriptionId}-${cell.day.id}`
        return <Button key={cell.day.id} aria-label={cell.day.label} aria-describedby={describedBy} aria-pressed={selected} data-quota-day={cell.day.id} data-partial={cell.day.partial || undefined} onPress={() => onDay(selected ? undefined : cell.day.id)} onHoverStart={() => onHover(cell.day.id)} onFocus={() => onHover(cell.day.id)} onBlur={() => onHover(undefined)} {...stylex.props(styles.dayButton, selected && styles.dayButtonSelected)}>
          <VisuallyHidden elementType="span"><span id={describedBy}>{description}</span></VisuallyHidden>
        </Button>
      })}
    </div>
  </div>
}

const Axis = ({ cells }: { readonly cells: readonly DayCell[] }) => {
  const first = cells[0]
  const last = cells.at(-1)
  const partial = cells.some(cell => cell.day.partial)
  return <div {...stylex.props(styles.axis)}>
    <span>{first?.day.label ?? ''}</span>
    {partial && <span {...stylex.props(styles.partialLegend)}>Dashed outline: partial Berlin day, still open</span>}
    <span>{last === first ? '' : last?.day.label ?? ''}</span>
  </div>
}

/** Fixed-height exact readout for the hovered (or selected) day. */
const Readout = ({ cell, series, unit, landmarkContext }: { readonly cell?: DayCell; readonly series: readonly Series[]; readonly unit: QuotaUnit; readonly landmarkContext: string }) => {
  const format = formatExact(unit)
  return <div role="region" aria-label={`${landmarkContext} exact usage values`} tabIndex={0} aria-live="polite" data-quota-readout={cell?.day.id ?? ''} {...stylex.props(styles.readout)}>
    {cell === undefined ? <span {...stylex.props(styles.muted)}>Hover, focus or select a day for exact values.</span> : <>
      <span {...stylex.props(styles.readoutHead)}>{cell.day.label}{cell.day.partial && <span {...stylex.props(styles.partialBadge)}>Partial · Berlin</span>}<span {...stylex.props(styles.figure)}>{tallyText(totalOf(cell, series), format)}</span></span>
      <ul {...stylex.props(styles.readoutList)}>
        {series.flatMap(item => {
          const tally = cell.bySeries.get(item.key)
          return tally === undefined ? [] : [<li key={item.key} {...stylex.props(styles.readoutItem)}><Swatch tone={item.tone} /><span {...stylex.props(styles.truncate)}>{item.label}</span><span {...stylex.props(styles.figure)}>{tallyText(tally, format)}</span></li>]
        })}
      </ul>
    </>}
  </div>
}

const Legend = ({ series }: { readonly series: readonly Series[] }) => <ul aria-label="Series" {...stylex.props(styles.legend)}>
  {series.map(item => <li key={item.key} {...stylex.props(styles.legendItem)}><Swatch tone={item.tone} /><span {...stylex.props(styles.truncate)}>{item.label}</span></li>)}
</ul>

/** Two-thumb React Aria range slider; the overview variant draws the whole history inside the track as a brush. */
const RangeControl = ({ allCells, series, start, end, strip, onRange }: {
  readonly allCells: readonly DayCell[]
  readonly series: readonly Series[]
  readonly start: number
  readonly end: number
  readonly strip: boolean
  readonly onRange: (range: readonly [number, number]) => void
}) => {
  const max = scaleOf(allCells, series)
  const last = Math.max(1, allCells.length - 1)
  const width = Math.max(1, allCells.length) * slotUnits
  return <Slider aria-label="History range" value={[start, end]} minValue={0} maxValue={last} step={1} isDisabled={allCells.length < 2} onChange={value => {
    const [from, to] = value
    if (from !== undefined && to !== undefined) onRange([from, to])
  }} {...stylex.props(styles.range)}>
    <div {...stylex.props(styles.rangeHead)}>
      <span {...stylex.props(styles.rangeLabel)}>History range</span>
      <SliderOutput {...stylex.props(styles.rangeOutput)}>{() => allCells.length === 0 ? 'No days' : `${allCells[start]?.day.label ?? ''} – ${allCells[end]?.day.label ?? ''} · ${end - start + 1} days`}</SliderOutput>
    </div>
    <SliderTrack {...stylex.props(styles.track, strip ? styles.trackStrip : styles.trackRail)}>
      {({ state }) => <>
        {strip && <svg aria-hidden="true" viewBox={`0 0 ${width} ${chartUnits}`} preserveAspectRatio="none" {...stylex.props(styles.svg)}>
          {allCells.map((cell, index) => {
            const total = totalOf(cell, series)
            const height = total.known === 0 ? (total.unknown > 0 ? chartUnits : 2) : Math.max(2, (total.sum / max) * chartUnits)
            return <rect key={cell.day.id} x={index * slotUnits + 1} y={chartUnits - height} width={slotUnits - 2} height={height} {...stylex.props(styles.stripBar, total.known === 0 && total.unknown > 0 && styles.stripUnknown)} />
          })}
        </svg>}
        <span aria-hidden="true" {...stylex.props(styles.brush, strip ? styles.brushStrip : styles.brushRail, styles.brushSpan(`${state.getThumbPercent(0) * 100}%`, `${(state.getThumbPercent(1) - state.getThumbPercent(0)) * 100}%`))} />
        <SliderThumb index={0} aria-label="Range start" {...stylex.props(styles.thumb, strip ? styles.thumbStrip : styles.thumbRail)} />
        <SliderThumb index={1} aria-label="Range end" {...stylex.props(styles.thumb, strip ? styles.thumbStrip : styles.thumbRail)} />
      </>}
    </SliderTrack>
  </Slider>
}

const SmallMultiples = ({ cells, series, unit, selectedDay, onDay, onHover }: { readonly cells: readonly DayCell[]; readonly series: readonly Series[]; readonly unit: QuotaUnit; readonly selectedDay?: string; readonly onDay: (day?: string) => void; readonly onHover: (day?: string) => void }) => {
  const format = formatExact(unit)
  return <ul aria-label="Small multiples" {...stylex.props(styles.multiples)}>
    {series.map(item => {
      const only = [item]
      const total = cells.reduce((tally, cell) => addTally(tally, cell.bySeries.get(item.key) ?? emptyTally), emptyTally)
      return <li key={item.key} data-quota-series={item.key} {...stylex.props(styles.multiple)}>
        <div {...stylex.props(styles.multipleHead)}><Swatch tone={item.tone} /><FullValueTooltip full={item.label} style={styles.truncate}>{item.label}</FullValueTooltip><span {...stylex.props(styles.figure)}>{tallyText(total, format)}</span></div>
        <StackedChart cells={cells} series={only} unit={unit} height="small" label={`${item.label} days`} selectedDay={selectedDay} onDay={onDay} onHover={onHover} />
        <span {...stylex.props(styles.scaleNote)}>Own scale · peak {formatCompact(unit, scaleOf(cells, only))}</span>
      </li>
    })}
    {series.length === 0 && <li {...stylex.props(styles.muted)}>No series in this range.</li>}
  </ul>
}

/** Honest calendar: cells sit on their Berlin weekday; absent dates stay blank, unknown is hatched, zero is outlined. */
const Heatmap = ({ cells, series, unit, selectedDay, onDay, onHover, landmarkContext }: { readonly cells: readonly DayCell[]; readonly series: readonly Series[]; readonly unit: QuotaUnit; readonly selectedDay?: string; readonly onDay: (day?: string) => void; readonly onHover: (day?: string) => void; readonly landmarkContext: string }) => {
  const max = scaleOf(cells, series)
  const firstNumber = dayNumberOf(cells[0]?.day.id ?? '')
  const origin = firstNumber === undefined ? 0 : firstNumber - weekdayOf(firstNumber)
  const selected = cells.find(cell => cell.day.id === selectedDay)
  return <div {...stylex.props(styles.heatmapWrap)}>
    <div role="group" aria-label="Calendar days" {...stylex.props(styles.heatmap)} onPointerLeave={() => onHover(undefined)}>
      {weekdays.map((name, column) => <span key={name} aria-hidden="true" {...stylex.props(styles.weekday, styles.cellAt(1, column + 1))}>{name}</span>)}
      {cells.map((cell, index) => {
        const dayNumber = firstNumber === undefined ? undefined : dayNumberOf(cell.day.id)
        const offset = dayNumber === undefined ? index : dayNumber - origin
        const total = totalOf(cell, series)
        const description = describeDay(cell, series, unit)
        const isSelected = cell.day.id === selectedDay
        const level = Math.min(3, Math.floor((total.sum / max) * 4 - 1e-9))
        return <Button key={cell.day.id} aria-label={description} aria-pressed={isSelected} data-quota-day={cell.day.id} data-partial={cell.day.partial || undefined} onPress={() => onDay(isSelected ? undefined : cell.day.id)} onHoverStart={() => onHover(cell.day.id)} onFocus={() => onHover(cell.day.id)} onBlur={() => onHover(undefined)} {...stylex.props(styles.heatCell, styles.cellAt(Math.floor(offset / 7) + 2, (offset % 7) + 1), cell.day.partial && styles.heatPartial, isSelected && styles.dayButtonSelected)}>
          {total.known === 0 && total.unknown > 0 ? <span aria-hidden="true" {...stylex.props(styles.heatFill, styles.heatUnknown)} />
            : total.sum === 0 ? <span aria-hidden="true" {...stylex.props(styles.heatFill, styles.heatZero)} />
              : <span aria-hidden="true" {...stylex.props(styles.heatFill, styles.heatKnown, heatLevels[Math.max(0, level)])} />}
          <span aria-hidden="true" {...stylex.props(styles.heatLabel)}>{cell.day.label}</span>
        </Button>
      })}
    </div>
    <section aria-label={`${landmarkContext} day drilldown`} data-quota-drilldown={selected?.day.id ?? ''} {...stylex.props(styles.drilldown)}>
      <h3 {...stylex.props(styles.sectionTitle)}>{selected === undefined ? 'Drilldown' : `Drilldown · ${selected.day.label}`}{selected?.day.partial && <span {...stylex.props(styles.partialBadge)}>Partial · Berlin</span>}</h3>
      {selected === undefined ? <p {...stylex.props(styles.muted)}>Select a calendar day to split it by series.</p> : <ul {...stylex.props(styles.drillList)}>
        {series.flatMap(item => {
          const tally = selected.bySeries.get(item.key)
          if (tally === undefined) return []
          const total = totalOf(selected, series)
          const share = total.sum === 0 ? 0 : tally.sum / total.sum
          return [<li key={item.key} {...stylex.props(styles.drillItem)}>
            <FullValueTooltip full={item.label} style={styles.truncate}>{item.label}</FullValueTooltip>
            <span aria-hidden="true" {...stylex.props(styles.drillTrack)}>{tally.known > 0 && <span {...stylex.props(styles.drillFill, toneBackground[item.tone], styles.share(`${share * 100}%`))} />}</span>
            <span {...stylex.props(styles.figure)}>{tallyText(tally, formatExact(unit))}</span>
          </li>]
        })}
      </ul>}
    </section>
  </div>
}

interface BreakdownRow { readonly key: string; readonly model: string; readonly scope: string; readonly accounts: readonly string[]; readonly value: Tally; readonly coverage: Tally; readonly tokenTypes: ReadonlyMap<string, Tally>; readonly requests: Tally; readonly turns: Tally }

const buildBreakdown = (days: readonly QuotaHistoryDay[], accounts: readonly QuotaAccount[], unit: QuotaUnit, tokenLabels: readonly string[]): readonly BreakdownRow[] => {
  const rows = new Map<string, { model: string; scope: string; accounts: Set<string>; value: Tally; coverage: Tally; tokenTypes: Map<string, Tally>; requests: Tally; turns: Tally }>()
  for (const day of days) for (const value of day.values) {
    const key = `${value.model}\u0000${value.scope}`
    const row = rows.get(key) ?? { model: value.model, scope: value.scope, accounts: new Set<string>(), value: emptyTally, coverage: emptyTally, tokenTypes: new Map<string, Tally>(), requests: emptyTally, turns: emptyTally }
    row.accounts.add(accounts.find(account => account.id === value.accountId)?.label ?? value.accountId)
    row.value = addValue(row.value, amountOf(value, unit))
    row.coverage = addValue(row.coverage, value.coverage)
    for (const label of tokenLabels) row.tokenTypes.set(label, addValue(row.tokenTypes.get(label) ?? emptyTally, value.tokenTypes.find(item => item.label === label)?.value))
    row.requests = addValue(row.requests, value.requests)
    row.turns = addValue(row.turns, value.turns)
    rows.set(key, row)
  }
  return [...rows].map(([key, row]) => ({ key, ...row, accounts: [...row.accounts].sort() })).sort((left, right) => right.value.sum - left.value.sum || left.model.localeCompare(right.model) || left.scope.localeCompare(right.scope))
}

const Breakdown = ({ days, scopeDays, accounts, unit, selected, rangeLabel, landmarkContext }: { readonly days: readonly QuotaHistoryDay[]; readonly scopeDays: readonly QuotaHistoryDay[]; readonly accounts: readonly QuotaAccount[]; readonly unit: QuotaUnit; readonly selected?: QuotaHistoryDay; readonly rangeLabel: string; readonly landmarkContext: string }) => {
  // Token-type columns come from the whole history so selection never changes the column set.
  const tokenLabels = [...new Set(days.flatMap(day => day.values.flatMap(value => value.tokenTypes.map(item => item.label))))]
  const rows = buildBreakdown(scopeDays, accounts, unit, tokenLabels)
  const format = formatExact(unit)
  const count = (value: number) => countExact.format(value)
  const coverageText = (tally: Tally) => tally.known === 0 ? 'Unreported' : `${percent.format(tally.sum / tally.known)}${tally.unknown > 0 ? ` + ${tally.unknown} Unreported` : ''}`
  const titleId = React.useId()
  return <section aria-label={`${landmarkContext} by model and scope`} data-quota-breakdown={selected?.id ?? 'range'} {...stylex.props(styles.card)}>
    <header {...stylex.props(styles.cardHead)}>
      <h2 id={titleId} {...stylex.props(styles.sectionTitle)}>By model and scope</h2>
      <span {...stylex.props(styles.scopeText)}>{selected === undefined ? rangeLabel : `${selected.label}${selected.partial ? ' · partial Berlin day' : ''}`}</span>
    </header>
    <div tabIndex={0} role="group" aria-label="Model and scope table scroll area" {...stylex.props(styles.tableScroll)}>
      <table {...stylex.props(styles.table)}>
        <colgroup><col {...stylex.props(styles.colModel)} /><col {...stylex.props(styles.colScope)} /><col {...stylex.props(styles.colAccounts)} /><col {...stylex.props(styles.colValue)} /><col {...stylex.props(styles.colCoverage)} />{tokenLabels.map(label => <col key={label} {...stylex.props(styles.colCount)} />)}<col {...stylex.props(styles.colCount)} /><col {...stylex.props(styles.colCount)} /></colgroup>
        <thead><tr>
          <th scope="col" {...stylex.props(styles.th)}>Model</th><th scope="col" {...stylex.props(styles.th)}>Scope</th><th scope="col" {...stylex.props(styles.th)}>Accounts</th>
          <th scope="col" {...stylex.props(styles.th, styles.numeric)}>{unit === 'usd' ? 'USD' : 'Tokens'}</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>USD coverage</th>
          {tokenLabels.map(label => <th key={label} scope="col" {...stylex.props(styles.th, styles.numeric)}>{label}</th>)}
          <th scope="col" {...stylex.props(styles.th, styles.numeric)}>Requests</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>Turns</th>
        </tr></thead>
        <tbody>
          {rows.map(row => <tr key={row.key} data-quota-breakdown-row={row.key}>
            <th scope="row" {...stylex.props(styles.td, styles.rowHead)}><FullValueTooltip full={row.model} style={styles.block}>{row.model}</FullValueTooltip></th>
            <td {...stylex.props(styles.td)}><FullValueTooltip full={row.scope} style={styles.block}>{row.scope}</FullValueTooltip></td>
            <td {...stylex.props(styles.td)}><FullValueTooltip full={row.accounts.join(', ')} style={styles.block}>{row.accounts.join(', ')}</FullValueTooltip></td>
            <td {...stylex.props(styles.td, styles.numeric)}>{tallyText(row.value, format)}</td>
            <td {...stylex.props(styles.td, styles.numeric)}>{coverageText(row.coverage)}</td>
            {tokenLabels.map(label => <td key={label} {...stylex.props(styles.td, styles.numeric)}>{tallyText(row.tokenTypes.get(label) ?? emptyTally, count)}</td>)}
            <td {...stylex.props(styles.td, styles.numeric)}>{tallyText(row.requests, count)}</td>
            <td {...stylex.props(styles.td, styles.numeric)}>{tallyText(row.turns, count)}</td>
          </tr>)}
          {rows.length === 0 && <tr><td colSpan={7 + tokenLabels.length} {...stylex.props(styles.td, styles.muted)}>No usage rows for {selected === undefined ? 'this range' : selected.label}.</td></tr>}
        </tbody>
      </table>
    </div>
  </section>
}

const Facts = ({ facts, label }: { readonly facts: readonly { readonly label: string; readonly value: string }[]; readonly label: string }) => facts.length === 0
  ? <p {...stylex.props(styles.muted)}>No {label} reported.</p>
  : <dl aria-label={label} {...stylex.props(styles.facts)}>{facts.map((fact, index) => <React.Fragment key={`${fact.label}-${index}`}><dt {...stylex.props(styles.factLabel)}>{fact.label}</dt><dd {...stylex.props(styles.factValue)}>{fact.value}</dd></React.Fragment>)}</dl>

const LimitsPanel = ({ account }: { readonly account: QuotaAccount }) => <ul aria-label={`${account.label} limits`} {...stylex.props(styles.windowList)}>
  {account.windows.map(window => {
    const tone = fractionTone(window.fraction, window.exhausted)
    return <li key={window.id} data-quota-window={window.id} {...stylex.props(styles.window)}>
      <div {...stylex.props(styles.windowHead)}>
        <span {...stylex.props(styles.windowLabel)}>{window.label}</span>
        {window.governing && <span {...stylex.props(styles.tag)}>Governing</span>}
        {window.comparison && <span {...stylex.props(styles.tag)}>Comparison</span>}
        <span {...stylex.props(styles.figure, tone === 'warning' && styles.warningText, tone === 'danger' && styles.dangerText)}>{windowReading(window)}</span>
      </div>
      <span aria-hidden="true" {...stylex.props(styles.meter)}>{window.fraction !== undefined && <span {...stylex.props(styles.meterFill, tone === 'warning' && styles.warningFill, tone === 'danger' && styles.dangerFill, styles.share(`${Math.min(1, Math.max(0, window.fraction)) * 100}%`))} />}</span>
      <dl {...stylex.props(styles.facts)}>
        <dt {...stylex.props(styles.factLabel)}>Scope</dt><dd {...stylex.props(styles.factValue)}>{window.scope}</dd>
        <dt {...stylex.props(styles.factLabel)}>Column</dt><dd {...stylex.props(styles.factValue)}>{window.column}</dd>
        <dt {...stylex.props(styles.factLabel)}>Resets</dt><dd {...stylex.props(styles.factValue)}>{window.resetAt === undefined ? 'Unreported' : <time dateTime={window.resetAt}>{window.resetAt}</time>}</dd>
        {window.facts.map((fact, index) => <React.Fragment key={`${fact.label}-${index}`}><dt {...stylex.props(styles.factLabel)}>{fact.label}</dt><dd {...stylex.props(styles.factValue)}>{fact.value}</dd></React.Fragment>)}
      </dl>
    </li>
  })}
  {account.windows.length === 0 && <li {...stylex.props(styles.muted)}>No windows reported for this account.</li>}
</ul>

const AccountPanel = ({ account }: { readonly account: QuotaAccount }) => <div {...stylex.props(styles.panelStack)}>
  <dl aria-label="Account summary" {...stylex.props(styles.facts)}>
    <dt {...stylex.props(styles.factLabel)}>Provider</dt><dd {...stylex.props(styles.factValue)}>{account.provider}</dd>
    <dt {...stylex.props(styles.factLabel)}>Plan</dt><dd {...stylex.props(styles.factValue)}>{account.plan}</dd>
    <dt {...stylex.props(styles.factLabel)}>State</dt><dd {...stylex.props(styles.factValue)}>{quotaStateLabels[account.state]}</dd>
    <dt {...stylex.props(styles.factLabel)}>USD</dt><dd {...stylex.props(styles.factValue)}>{account.usdCoverage ? formatUsdExact(account.usd) : `${formatUsdExact(account.usd)} · coverage below 95%`}</dd>
    <dt {...stylex.props(styles.factLabel)}>Tokens</dt><dd {...stylex.props(styles.factValue)}>{account.tokens === undefined ? 'Unreported' : tokensExact.format(account.tokens)}</dd>
    <dt {...stylex.props(styles.factLabel)}>Resets used</dt><dd {...stylex.props(styles.factValue)}>{account.resetCount === undefined ? 'Unreported' : countExact.format(account.resetCount)}</dd>
  </dl>
  <h3 {...stylex.props(styles.subTitle)}>Reset credits</h3>
  <Facts facts={account.resetCredits} label="reset credits" />
  <h3 {...stylex.props(styles.subTitle)}>Account facts</h3>
  <Facts facts={account.facts} label="account facts" />
</div>

const HistoryPanel = ({ account, days, unit }: { readonly account: QuotaAccount; readonly days: readonly QuotaHistoryDay[]; readonly unit: QuotaUnit }) => {
  const format = formatExact(unit)
  return <div tabIndex={0} role="group" aria-label={`${account.label} history table scroll area`} {...stylex.props(styles.tableScroll)}><table aria-label={`${account.label} daily history`} {...stylex.props(styles.table)}>
    <colgroup><col {...stylex.props(styles.colDay)} /><col {...stylex.props(styles.colValue)} /><col {...stylex.props(styles.colCount)} /></colgroup>
    <thead><tr><th scope="col" {...stylex.props(styles.th)}>Day (Berlin)</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>{unit === 'usd' ? 'USD' : 'Tokens'}</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>Requests</th></tr></thead>
    <tbody>{days.map(day => {
      const values = day.values.filter(value => value.accountId === account.id)
      return <tr key={day.id} data-quota-history-day={day.id}>
        <th scope="row" {...stylex.props(styles.td, styles.rowHead)}>{day.label}{day.partial && <span {...stylex.props(styles.partialBadge)}>Partial · Berlin</span>}</th>
        <td {...stylex.props(styles.td, styles.numeric)}>{tallyText(values.reduce((tally, value) => addValue(tally, amountOf(value, unit)), emptyTally), format)}</td>
        <td {...stylex.props(styles.td, styles.numeric)}>{tallyText(values.reduce((tally, value) => addValue(tally, value.requests), emptyTally), value => countExact.format(value))}</td>
      </tr>
    })}</tbody>
  </table></div>
}

const CostOverview = ({ economics, accounts, landmarkContext }: { readonly economics: readonly QuotaEconomics[]; readonly accounts: readonly QuotaAccount[]; readonly landmarkContext: string }) => {
  const titleId = React.useId()
  return <section aria-label={`${landmarkContext} cost overview`} data-quota-cost {...stylex.props(styles.card)}>
  <header {...stylex.props(styles.cardHead)}>
    <h2 id={titleId} {...stylex.props(styles.sectionTitle)}>Cost overview</h2>
    <span {...stylex.props(styles.scopeText)}>Last 30 days, normalized</span>
  </header>
  <div tabIndex={0} role="group" aria-label="Cost table scroll area" {...stylex.props(styles.tableScroll)}>
    <table {...stylex.props(styles.table)}>
      <colgroup><col {...stylex.props(styles.costIdentity)} /><col {...stylex.props(styles.costAmount)} /><col {...stylex.props(styles.costAmount)} /><col {...stylex.props(styles.costAmount)} /><col {...stylex.props(styles.colCoverage)} /><col {...stylex.props(styles.colCoverage)} /></colgroup>
      <thead><tr>
        <th scope="col" {...stylex.props(styles.th)}>Subscription</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>Usage value</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>Price</th>
        <th scope="col" {...stylex.props(styles.th, styles.numeric)}>Ledger</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>Value ÷ price</th><th scope="col" {...stylex.props(styles.th, styles.numeric)}>Coverage</th>
      </tr></thead>
      <tbody>
        {economics.map(item => {
          const account = accounts.find(candidate => candidate.id === item.accountId)
          const limit = account?.windows.find(window => window.id === item.limitId)?.label ?? item.limitId
          return <tr key={item.id} data-quota-economics={item.id}>
            <th scope="row" {...stylex.props(styles.td, styles.rowHead)}><FullValueTooltip full={item.label} style={styles.block}>{item.label}</FullValueTooltip><FullValueTooltip full={`${account?.label ?? item.accountId} · ${limit}`} style={[styles.block, styles.muted]}>{account?.label ?? item.accountId} · {limit}</FullValueTooltip></th>
            <td {...stylex.props(styles.td, styles.numeric)}>{formatUsd(item.usageValue)}</td>
            <td {...stylex.props(styles.td, styles.numeric)}><span {...stylex.props(styles.block)}>{formatUsd(item.price)}</span><span {...stylex.props(styles.block, styles.muted)}>{item.basis === 'invoiced' ? 'Invoice' : 'List price'}</span></td>
            <td {...stylex.props(styles.td, styles.numeric)}>{formatUsd(item.ledgerValue)}</td>
            <td {...stylex.props(styles.td, styles.numeric)}>{item.usageValue === undefined || item.price === undefined || item.price === 0 ? 'Unreported' : `${(item.usageValue / item.price).toFixed(2)}×`}</td>
            <td {...stylex.props(styles.td, styles.numeric)}>{item.coverageDays === undefined ? 'Unreported' : `${countExact.format(item.coverageDays)} of 30 days`}</td>
          </tr>
        })}
        {economics.length === 0 && <tr><td colSpan={6} {...stylex.props(styles.td, styles.muted)}>No subscription economics supplied.</td></tr>}
      </tbody>
    </table>
  </div>
</section>
}

/** Controlled Usage workspace: stacked daily history (overview + brush, small multiples, or calendar heatmap), per-model/scope breakdown, Limits/Account/History detail and 30-day normalized cost. */
export const QuotaUsagePage = (props: QuotaUsageProps): React.ReactElement => {
  const landmarkContext = props.landmarkContext ?? 'Usage'
  const { accounts, days, economics, status: quotaStatus, variant, unit, stackBy, range, selectedDay, selectedAccount, detailTab, onUnit, onStackBy, onRange, onDay, onAccount, onDetailTab, onBack } = props
  const [hoveredDay, setHoveredDay] = React.useState<string | undefined>(undefined)
  const idBase = React.useId()
  const ids = { unitNote: `${idBase}-unit`, history: `${idBase}-history`, detail: `${idBase}-detail` }
  const lastIndex = Math.max(0, days.length - 1)
  const start = clampIndex(range[0], 0, lastIndex)
  const end = clampIndex(range[1], start, lastIndex)
  const rangeDays = days.slice(start, end + 1)
  const rangeValues = rangeDays.flatMap(day => day.values)
  // Owner gate: USD only when every visible record admits USD coverage; unknown USD amounts stay unknown.
  const inadequate = rangeValues.filter(value => value.coverage < usdCoverageGate).length
  const usdAllowed = rangeValues.length > 0 && inadequate === 0
  const effectiveUnit: QuotaUnit = unit === 'usd' && usdAllowed ? 'usd' : 'tokens'
  const toggleUnit = () => onUnit(unit === 'usd' ? 'tokens' : 'usd')
  const series = buildSeries(days, accounts, stackBy)
  const allCells = days.map(day => buildCell(day, effectiveUnit, stackBy))
  const cells = allCells.slice(start, end + 1)
  // Controlled selections can outlive a brush change; never display records outside the admitted range.
  const focusCell = cells.find(cell => cell.day.id === (hoveredDay ?? selectedDay))
  const selected = rangeDays.find(day => day.id === selectedDay)
  const detailAccount = accounts.find(account => account.id === selectedAccount) ?? accounts[0]
  const rangeLabel = `${rangeDays[0]?.label ?? 'No days'}${rangeDays.length > 1 ? ` – ${rangeDays.at(-1)?.label ?? ''}` : ''}`
  const unitNote = unit === 'tokens' ? 'Preferred tokens; showing tokens.'
    : rangeValues.length === 0 ? 'Preferred USD; no records in range, showing tokens.'
      : usdAllowed ? 'Preferred USD; every record in range has at least 95% USD coverage.'
        : `Preferred USD; showing tokens because ${inadequate} of ${rangeValues.length} records in range have under 95% USD coverage.`
  const statusAlert = quotaStatus.kind === 'failed' || quotaStatus.kind === 'incompatible'
  // Drawer parity: plain `u` toggles the unit preference unless typing in a text field.
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'u' || event.altKey || event.ctrlKey || event.metaKey || event.defaultPrevented) return
    const target = event.target
    if (target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable) || (target instanceof HTMLInputElement && !['range', 'radio', 'checkbox', 'button'].includes(target.type))) return
    event.preventDefault()
    toggleUnit()
  }
  return <div data-quota-usage data-quota-variant={variant} data-quota-status={quotaStatus.kind} data-quota-unit={effectiveUnit} data-quota-unit-preference={unit} onKeyDown={onKeyDown} {...stylex.props(styles.root)}>
    <header {...stylex.props(styles.header)}>
      {onBack !== undefined && <Button onPress={onBack} {...stylex.props(styles.button)}><Icon name="chevrons-left" size={12} />Back</Button>}
      <h1 {...stylex.props(styles.pageTitle)}>Usage</h1>
      <p role={statusAlert ? 'alert' : 'status'} data-quota-status-kind={quotaStatus.kind} {...stylex.props(styles.status, (quotaStatus.kind === 'stale' || quotaStatus.kind === 'unpublished' || quotaStatus.kind === 'loading') && styles.statusWarn, statusAlert && styles.statusDanger)}>
        <span aria-hidden="true" {...stylex.props(styles.statusDot)} />
        <FullValueTooltip full={quotaStatus.text} style={styles.truncate}>{quotaStatus.text}</FullValueTooltip>
      </p>
      <div role="toolbar" aria-label="Usage view" {...stylex.props(styles.toolbar)}>
        <RadioGroup value={stackBy} orientation="horizontal" onChange={value => { if (value === 'account' || value === 'model') onStackBy(value) }} {...stylex.props(styles.segmented)}>
          <Label {...stylex.props(styles.controlLabel)}>Stack by</Label>
          <Radio value="account" {...stylex.props(styles.segment)}>Account</Radio>
          <Radio value="model" {...stylex.props(styles.segment)}>Model</Radio>
        </RadioGroup>
        <Button aria-label="Toggle unit: $ / tokens" aria-describedby={ids.unitNote} aria-keyshortcuts="u" onPress={toggleUnit} {...stylex.props(styles.button, styles.unitButton)}>
          <span aria-hidden="true" {...stylex.props(unit === 'usd' ? styles.unitOn : styles.unitOff)}>$</span><span aria-hidden="true" {...stylex.props(styles.muted)}>/</span><span aria-hidden="true" {...stylex.props(unit === 'tokens' ? styles.unitOn : styles.unitOff)}>tokens</span>
        </Button>
        <span id={ids.unitNote} data-quota-unit-fallback={unit !== effectiveUnit || undefined} {...stylex.props(styles.unitNote, unit !== effectiveUnit && styles.unitFallback)}><FullValueTooltip full={unitNote} style={styles.block}>{unitNote}</FullValueTooltip></span>
      </div>
    </header>
    <div {...stylex.props(styles.grid)}>
      <section aria-label={`${landmarkContext} history workspace`} {...stylex.props(styles.mainColumn)}>
        <section aria-label={`${landmarkContext} daily chart`} {...stylex.props(styles.card)}>
          <header {...stylex.props(styles.cardHead)}>
            <h2 id={ids.history} {...stylex.props(styles.sectionTitle)}>{variant === 'overview' ? 'Daily usage' : variant === 'multiples' ? `Daily usage by ${stackBy}` : 'Usage calendar'}</h2>
            <span {...stylex.props(styles.scopeText)}>Berlin calendar days · {effectiveUnit === 'usd' ? 'USD' : 'tokens'} · {rangeLabel}</span>
          </header>
          {days.length === 0 ? <p {...stylex.props(styles.emptyChart)}>No history days supplied.</p> : <>
            {variant === 'overview' && <><Legend series={series} /><StackedChart cells={cells} series={series} unit={effectiveUnit} height="main" label="Days in range" selectedDay={selectedDay} onDay={onDay} onHover={setHoveredDay} /><Axis cells={cells} /></>}
            {variant === 'multiples' && <SmallMultiples cells={cells} series={series} unit={effectiveUnit} selectedDay={selectedDay} onDay={onDay} onHover={setHoveredDay} />}
            {variant === 'heatmap' && <Heatmap cells={cells} series={series} unit={effectiveUnit} selectedDay={selectedDay} onDay={onDay} onHover={setHoveredDay} landmarkContext={landmarkContext} />}
            {variant !== 'heatmap' && <Readout cell={focusCell} series={series} unit={effectiveUnit} landmarkContext={landmarkContext} />}
          </>}
          <RangeControl allCells={allCells} series={series} start={start} end={end} strip={variant === 'overview'} onRange={onRange} />
          {selected !== undefined && <Button onPress={() => onDay(undefined)} {...stylex.props(styles.button, styles.clearDay)}>Clear day {selected.label}</Button>}
        </section>
        <Breakdown days={days} scopeDays={selected === undefined ? rangeDays : [selected]} accounts={accounts} unit={effectiveUnit} selected={selected} rangeLabel={rangeLabel} landmarkContext={landmarkContext} />
      </section>
      <aside aria-label={`${landmarkContext} quota detail`} {...stylex.props(styles.sideColumn)}>
        <section aria-label={`${landmarkContext} account evidence`} {...stylex.props(styles.card)}>
          <header {...stylex.props(styles.cardHead)}><h2 id={ids.detail} {...stylex.props(styles.sectionTitle)}>Detail</h2></header>
          <ListBox aria-label="Accounts" selectionMode="single" disallowEmptySelection selectedKeys={detailAccount === undefined ? [] : [detailAccount.id]} onSelectionChange={keys => {
            if (keys === 'all') return
            const id = keys.values().next().value
            if (typeof id === 'string') onAccount(id)
          }} renderEmptyState={() => <span {...stylex.props(styles.muted)}>No accounts supplied.</span>} {...stylex.props(styles.accountList)}>
            {accounts.map(account => <ListBoxItem key={account.id} id={account.id} textValue={account.label} data-quota-account={account.id} {...stylex.props(styles.accountItem, account.id === detailAccount?.id && styles.accountItemSelected)}>
              <span {...stylex.props(styles.truncate)}>{account.label}</span><span {...stylex.props(styles.accountMeta)}>{account.provider} · {quotaStateLabels[account.state]}</span>
            </ListBoxItem>)}
          </ListBox>
          <Tabs selectedKey={detailTab} onSelectionChange={key => { if (isDetailTab(key)) onDetailTab(key) }}>
            <TabList aria-label="Detail" {...stylex.props(styles.tabList)}>
              {detailTabs.map(tab => <Tab key={tab.id} id={tab.id} {...stylex.props(styles.tab)}>{tab.label}</Tab>)}
            </TabList>
            <TabPanel id="limits" {...stylex.props(styles.tabPanel)}>{detailAccount === undefined ? <p {...stylex.props(styles.muted)}>No account limits published.</p> : <LimitsPanel account={detailAccount} />}</TabPanel>
            <TabPanel id="account" {...stylex.props(styles.tabPanel)}>{detailAccount === undefined ? <p {...stylex.props(styles.muted)}>No account evidence published.</p> : <AccountPanel account={detailAccount} />}</TabPanel>
            <TabPanel id="history" {...stylex.props(styles.tabPanel)}>{detailAccount === undefined ? <p {...stylex.props(styles.muted)}>No account history published.</p> : <HistoryPanel account={detailAccount} days={rangeDays} unit={effectiveUnit} />}</TabPanel>
          </Tabs>
        </section>
        <CostOverview economics={economics} accounts={accounts} landmarkContext={landmarkContext} />
      </aside>
    </div>
  </div>
}

const styles = stylex.create({
  root: { containerType: 'inline-size', display: 'flex', flexDirection: 'column', gap: space.lg, minWidth: 0, padding: space.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading },
  header: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.lg, minBlockSize: quotaGeometry.usageHeaderMinimumHeight },
  pageTitle: { margin: 0, fontSize: type.headingSize, lineHeight: type.headingLeading, fontWeight: type.weightSemibold },
  status: { display: 'flex', alignItems: 'center', gap: space.sm, margin: 0, inlineSize: quotaGeometry.usageStatusWidth, maxInlineSize: '100%', blockSize: quotaGeometry.usageStatusHeight, paddingInline: space.md, borderRadius: radius.full, border: quotaGeometry.hair, color: ink.fgMuted, fontSize: type.metaSize },
  statusWarn: { color: status.attention },
  statusDanger: { color: status.dangerFg },
  statusDot: { flexShrink: 0, inlineSize: quotaGeometry.statusDotSize, blockSize: quotaGeometry.statusDotSize, borderRadius: radius.full, backgroundColor: 'currentColor' },
  toolbar: { display: 'flex', alignItems: 'center', gap: space.lg, marginInlineStart: 'auto', flexWrap: 'wrap' },
  segmented: { display: 'flex', alignItems: 'center', gap: space.xs },
  controlLabel: { color: ink.fgMuted, fontSize: type.metaSize, marginInlineEnd: space.xs },
  segment: { display: 'inline-flex', alignItems: 'center', blockSize: quotaGeometry.usageControlHeight, paddingInline: space.md, borderRadius: radius.control, border: quotaGeometry.strongHair, color: ink.fgMuted, backgroundColor: surface.transparent, cursor: 'pointer', fontSize: type.metaSize, ':hover': { backgroundColor: surface.rowHover }, ':is([data-selected])': { color: ink.fg, backgroundColor: surface.rowActive }, ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.focusOffset } },
  button: { display: 'inline-flex', alignItems: 'center', gap: space.xs, blockSize: quotaGeometry.usageControlHeight, paddingInline: space.md, borderRadius: radius.control, border: quotaGeometry.strongHair, backgroundColor: surface.controlFill, color: ink.fg, cursor: 'pointer', fontFamily: type.fontSans, fontSize: type.metaSize, ':hover': { backgroundColor: surface.rowHover }, ':is([data-disabled])': { cursor: 'not-allowed', opacity: 0.5 }, ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.focusOffset } },
  unitButton: { inlineSize: quotaGeometry.unitControlWidth, justifyContent: 'center', fontVariantNumeric: 'tabular-nums' },
  unitFallback: { color: status.attention },
  unitOn: { color: ink.fg, fontWeight: type.weightSemibold },
  unitOff: { color: ink.fgMuted },
  unitNote: { inlineSize: quotaGeometry.unitNoteWidth, color: ink.fgMuted, fontSize: type.denseSize, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  grid: { display: 'grid', gap: space.lg, gridTemplateColumns: { default: 'minmax(0, 1fr)', '@container (min-width: 72rem)': quotaGeometry.workspaceColumns }, alignItems: 'start' },
  mainColumn: { display: 'flex', flexDirection: 'column', gap: space.lg, minWidth: 0 },
  sideColumn: { display: 'flex', flexDirection: 'column', gap: space.lg, minWidth: 0 },
  card: { display: 'flex', flexDirection: 'column', gap: space.md, minWidth: 0, padding: space.lg, borderRadius: radius.base, border: quotaGeometry.hair, backgroundColor: surface.raised },
  cardHead: { display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: space.md, flexWrap: 'wrap' },
  sectionTitle: { display: 'flex', alignItems: 'center', gap: space.sm, margin: 0, fontSize: type.uiSize, fontWeight: type.weightSemibold },
  subTitle: { margin: `${space.md} 0 0`, fontSize: type.metaSize, fontWeight: type.weightMedium, color: ink.fgSoft },
  scopeText: { color: ink.fgMuted, fontSize: type.metaSize, fontVariantNumeric: 'tabular-nums' },
  muted: { color: ink.fgMuted },
  truncate: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  block: { display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  figure: { marginInlineStart: 'auto', fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', color: ink.fg },
  emptyChart: { display: 'grid', placeItems: 'center', blockSize: quotaGeometry.mainChartHeight, margin: 0, color: ink.fgMuted, border: quotaGeometry.dashedStrongHair, borderRadius: radius.md },
  legend: { display: 'flex', flexWrap: 'wrap', gap: `${space.xs} ${space.lg}`, margin: 0, padding: 0, listStyle: 'none', minBlockSize: quotaGeometry.legendHeight, fontSize: type.metaSize, color: ink.fgSoft },
  legendItem: { display: 'flex', alignItems: 'center', gap: space.xs, maxInlineSize: quotaGeometry.legendItemMaximumWidth },
  swatch: { flexShrink: 0, inlineSize: quotaGeometry.legendSwatchSize, blockSize: quotaGeometry.legendSwatchSize, borderRadius: quotaGeometry.legendSwatchRadius },
  chart: { position: 'relative', inlineSize: '100%' },
  chartMain: { blockSize: quotaGeometry.mainChartHeight },
  chartSmall: { blockSize: quotaGeometry.smallChartHeight },
  svg: { position: 'absolute', inset: 0, inlineSize: '100%', blockSize: '100%', overflow: 'visible' },
  baseline: { stroke: border.borderStrong, strokeWidth: quotaGeometry.chartStrokeWidth },
  hatchInk: { fill: ink.fgMuted },
  unknownBar: { stroke: ink.fgMuted, strokeWidth: quotaGeometry.chartStrokeWidth },
  zeroTick: { fill: ink.fgSoft },
  emptyTick: { fill: border.borderStrong },
  partialBar: { opacity: 0.55 },
  dimmedBar: { opacity: 0.35 },
  partialOutline: { fill: 'none', stroke: ink.fgSoft, strokeWidth: quotaGeometry.chartStrokeWidth, strokeDasharray: '3 2' },
  dayButtons: { position: 'absolute', inset: 0, display: 'flex' },
  dayButton: { flexBasis: 0, flexGrow: 1, minWidth: 0, padding: 0, border: 'none', borderRadius: radius.keyboard, backgroundColor: surface.transparent, cursor: 'pointer', ':hover': { backgroundColor: surface.washSubtle }, ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.insetFocusOffset } },
  dayButtonSelected: { boxShadow: quotaGeometry.selectedUnderline, backgroundColor: surface.washSubtle },
  axis: { display: 'flex', justifyContent: 'space-between', gap: space.md, blockSize: quotaGeometry.chartAxisHeight, color: ink.fgMuted, fontSize: type.denseSize, fontVariantNumeric: 'tabular-nums' },
  partialLegend: { color: ink.fgSoft },
  partialBadge: { display: 'inline-flex', alignItems: 'center', marginInlineStart: space.sm, paddingInline: space.xs, borderRadius: radius.keyboard, border: quotaGeometry.dashedMutedHair, color: ink.fgSoft, fontSize: type.denseSize, fontWeight: type.weightRegular, whiteSpace: 'nowrap' },
  readout: { display: 'flex', flexDirection: 'column', gap: space.xs, blockSize: quotaGeometry.dayReadoutHeight, overflowY: 'auto', padding: space.sm, borderRadius: radius.md, backgroundColor: surface.washSubtle, fontSize: type.metaSize, ':focus-visible': { outline: quotaGeometry.focusOutline, outlineOffset: quotaGeometry.focusOffset } },
  readoutHead: { display: 'flex', alignItems: 'center', gap: space.xs, fontWeight: type.weightMedium },
  readoutList: { display: 'grid', gridTemplateColumns: quotaGeometry.multipleColumns, gap: `0 ${space.lg}`, margin: 0, padding: 0, listStyle: 'none' },
  readoutItem: { display: 'flex', alignItems: 'center', gap: space.xs, minWidth: 0, color: ink.fgSoft },
  range: { display: 'flex', flexDirection: 'column', gap: space.xs },
  rangeHead: { display: 'flex', justifyContent: 'space-between', gap: space.md, fontSize: type.metaSize },
  rangeLabel: { color: ink.fgMuted },
  rangeOutput: { color: ink.fgSoft, fontVariantNumeric: 'tabular-nums' },
  track: { position: 'relative', inlineSize: '100%' },
  trackStrip: { blockSize: quotaGeometry.rangeStripHeight, borderRadius: radius.sm, backgroundColor: surface.washSubtle },
  trackRail: { blockSize: quotaGeometry.rangeRailHeight },
  stripBar: { fill: ink.fgMuted },
  stripUnknown: { fill: border.borderStrong },
  brush: { position: 'absolute', pointerEvents: 'none', backgroundColor: surface.washSubtle, borderInline: quotaGeometry.accentHair },
  brushStrip: { insetBlock: 0 },
  brushRail: { insetBlockStart: quotaGeometry.rangeRailInset, blockSize: quotaGeometry.rangeRailThickness, backgroundColor: accent.primary },
  brushSpan: (left: string, width: string) => ({ insetInlineStart: left, inlineSize: width }),
  thumb: { insetBlockStart: 0, inlineSize: quotaGeometry.rangeThumbWidth, borderRadius: radius.keyboard, border: quotaGeometry.accentHair, backgroundColor: surface.raised, cursor: 'ew-resize', ':is([data-dragging])': { backgroundColor: accent.primary }, ':is([data-focus-visible])': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.focusRingOffset } },
  thumbStrip: { blockSize: quotaGeometry.rangeStripHeight },
  thumbRail: { blockSize: quotaGeometry.rangeRailHeight },
  clearDay: { alignSelf: 'flex-start' },
  multiples: { display: 'grid', gridTemplateColumns: { default: 'repeat(2, minmax(0, 1fr))', '@container (min-width: 72rem)': 'repeat(3, minmax(0, 1fr))' }, gap: space.md, margin: 0, padding: 0, listStyle: 'none' },
  multiple: { display: 'flex', flexDirection: 'column', gap: space.xs, minWidth: 0, padding: space.sm, borderRadius: radius.md, border: quotaGeometry.hair },
  multipleHead: { display: 'flex', alignItems: 'center', gap: space.xs, minWidth: 0, fontSize: type.metaSize },
  scaleNote: { color: ink.fgMuted, fontSize: type.denseSize, fontVariantNumeric: 'tabular-nums' },
  heatmapWrap: { display: 'grid', gap: space.lg, gridTemplateColumns: { default: 'minmax(0, 1fr)', '@container (min-width: 56rem)': 'minmax(0, 1fr) minmax(0, 1fr)' }, alignItems: 'start' },
  heatmap: { display: 'grid', gridTemplateColumns: 'repeat(7, minmax(0, 1fr))', gridAutoRows: quotaGeometry.heatmapRowHeight, gap: space.xs },
  weekday: { alignSelf: 'end', color: ink.fgMuted, fontSize: type.denseSize, blockSize: quotaGeometry.weekdayLabelHeight },
  cellAt: (row: number, column: number) => ({ gridRowStart: String(row), gridColumnStart: String(column) }),
  heatCell: { position: 'relative', padding: 0, overflow: 'hidden', border: quotaGeometry.hair, borderRadius: radius.sm, backgroundColor: surface.canvas, cursor: 'pointer', ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.focusOffset } },
  heatPartial: { borderStyle: 'dashed', borderColor: ink.fgMuted },
  heatFill: { position: 'absolute', inset: 0 },
  heatKnown: { backgroundColor: accent.primary },
  heatZero: { backgroundColor: surface.transparent, boxShadow: quotaGeometry.heatZeroShadow },
  heatUnknown: { backgroundImage: quotaGeometry.unreportedHeatPattern },
  heatLabel: { position: 'absolute', insetInlineStart: space.xs, insetBlockStart: space.xs2, color: ink.fg, backgroundColor: surface.canvas, paddingInline: space.xs2, borderRadius: radius.keyboard, fontSize: type.denseSize, fontVariantNumeric: 'tabular-nums' },
  drilldown: { display: 'flex', flexDirection: 'column', gap: space.sm, minBlockSize: quotaGeometry.drilldownMinimumHeight },
  drillList: { display: 'flex', flexDirection: 'column', gap: space.xs, margin: 0, padding: 0, listStyle: 'none' },
  drillItem: { display: 'grid', gridTemplateColumns: quotaGeometry.drillColumns, alignItems: 'center', gap: space.sm, fontSize: type.metaSize, color: ink.fgSoft },
  drillTrack: { blockSize: quotaGeometry.drillMeterHeight, borderRadius: radius.full, backgroundColor: surface.washSubtle, overflow: 'hidden' },
  drillFill: { display: 'block', blockSize: '100%' },
  share: (width: string) => ({ inlineSize: width }),
  tableScroll: { overflowX: 'auto', scrollbarGutter: 'stable', ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.focusOffset } },
  table: { inlineSize: '100%', minInlineSize: '100%', tableLayout: 'fixed', borderCollapse: 'collapse', fontSize: type.metaSize, fontVariantNumeric: 'tabular-nums' },
  colModel: { inlineSize: quotaGeometry.modelColumnWidth },
  costIdentity: { inlineSize: quotaGeometry.costIdentityWidth },
  costAmount: { inlineSize: quotaGeometry.costAmountWidth },
  colScope: { inlineSize: quotaGeometry.scopeColumnWidth },
  colAccounts: { inlineSize: quotaGeometry.accountsColumnWidth },
  colValue: { inlineSize: quotaGeometry.valueColumnWidth },
  colCoverage: { inlineSize: quotaGeometry.coverageColumnWidth },
  colCount: { inlineSize: quotaGeometry.countColumnWidth },
  colDay: { inlineSize: quotaGeometry.dayColumnWidth },
  th: { blockSize: quotaGeometry.dataHeaderHeight, paddingInline: space.sm, textAlign: 'start', fontWeight: type.weightRegular, color: ink.fgMuted, borderBottom: quotaGeometry.strongHair, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  td: { blockSize: quotaGeometry.dataRowHeight, paddingInline: space.sm, borderBottom: quotaGeometry.hair, color: ink.fgSoft, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  rowHead: { textAlign: 'start', fontWeight: type.weightRegular, color: ink.fg },
  numeric: { textAlign: 'end' },
  accountList: { display: 'flex', flexDirection: 'column', gap: space.xs2, maxBlockSize: quotaGeometry.accountListMaximumHeight, overflowY: 'auto', scrollbarGutter: 'stable' },
  accountItem: { display: 'flex', alignItems: 'center', gap: space.sm, blockSize: quotaGeometry.accountRowHeight, paddingInline: space.sm, borderRadius: radius.sm, cursor: 'pointer', color: ink.fgSoft, backgroundColor: surface.transparent, ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.insetFocusOffset } },
  accountItemSelected: { backgroundColor: surface.rowActive, color: ink.fg },
  accountMeta: { marginInlineStart: 'auto', color: ink.fgSoft, fontSize: type.denseSize, whiteSpace: 'nowrap' },
  tabList: { display: 'flex', gap: space.xs, borderBottom: quotaGeometry.strongHair },
  tab: { paddingBlock: space.sm, paddingInline: space.md, cursor: 'pointer', color: ink.fgMuted, boxShadow: 'none', fontSize: type.metaSize, fontWeight: type.weightMedium, ':hover': { color: ink.fgSoft }, ':is([data-selected])': { color: ink.fg, boxShadow: quotaGeometry.selectedUnderline }, ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.insetFocusOffset } },
  tabPanel: { minBlockSize: quotaGeometry.tabPanelMinimumHeight, paddingBlockStart: space.md, ':focus-visible': { outlineWidth: quotaGeometry.focusRingWidth, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: quotaGeometry.insetFocusOffset } },
  panelStack: { display: 'flex', flexDirection: 'column', gap: space.xs },
  windowList: { display: 'flex', flexDirection: 'column', gap: space.md, margin: 0, padding: 0, listStyle: 'none' },
  window: { display: 'flex', flexDirection: 'column', gap: space.xs, paddingBlockEnd: space.md, borderBottom: quotaGeometry.hair },
  windowHead: { display: 'flex', alignItems: 'center', gap: space.sm, minWidth: 0, fontSize: type.metaSize },
  windowLabel: { fontWeight: type.weightMedium, color: ink.fg },
  tag: { paddingInline: space.xs, borderRadius: radius.keyboard, border: quotaGeometry.strongHair, color: ink.fgMuted, fontSize: type.denseSize },
  warningText: { color: status.attention },
  dangerText: { color: status.dangerFg },
  meter: { display: 'block', blockSize: quotaGeometry.windowMeterHeight, borderRadius: radius.full, backgroundColor: surface.washSubtle, overflow: 'hidden' },
  meterFill: { display: 'block', blockSize: '100%', backgroundColor: accent.primary },
  warningFill: { backgroundColor: status.attention },
  dangerFill: { backgroundColor: status.danger },
  facts: { display: 'grid', gridTemplateColumns: 'minmax(0, 2fr) minmax(0, 3fr)', gap: `${space.xs2} ${space.md}`, margin: 0, fontSize: type.metaSize },
  factLabel: { color: ink.fgMuted, overflowWrap: 'anywhere' },
  factValue: { margin: 0, color: ink.fgSoft, overflowWrap: 'anywhere', fontVariantNumeric: 'tabular-nums' },
})
