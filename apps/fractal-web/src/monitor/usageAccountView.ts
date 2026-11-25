/**
 * What the Usage pane shows for one quota account: every limit on its own line, banked resets and why
 * the reading is as old as it is. Limits read with the sidebar's tiers (`limitReading`); staleness is
 * the producer's `retained` flag, never reclassified here.
 */
import { bankedResets, effectiveUnit, formatUnitValue, type HistoryDay, type HistoryState, type HistoryUnit, type QuotaRow, type QuotaState } from './model.ts'
import { spanText } from './usageModel.ts'
import { ageText, limitReading, type UsageLevel, type UsageSectionView } from './usageSectionView.ts'

export interface LimitLineView {
  readonly id: string
  readonly label: string
  readonly fill: number | undefined
  /** "72%", "100%", "?", or "unreported"; the at-limit `!` is drawn from `level`. */
  readonly percent: string
  readonly level: UsageLevel
  /** Words beside the percent when the number alone would mislead: "at limit", "not reported". */
  readonly note: string | undefined
  readonly reset: string
  readonly words: string
}

export interface AccountDetailView {
  readonly id: string
  readonly plan: string | undefined
  /** Why the producer's summary could not be read; the limits still show. */
  readonly unreadable: string | undefined
  readonly limits: readonly LimitLineView[]
  /** Absent when the producer did not report banked resets: unknown is never "0". */
  readonly banked: string | undefined
  /** "42s old"; `held` when the producer kept an older reading. */
  readonly reading: { readonly age: string; readonly held: boolean }
  /** The latest attempt in the producer's words, only when it did not produce a fresh reading. */
  readonly attempt: string | undefined
}

const OK_OUTCOMES: readonly string[] = ['determinate', 'observed']

const countOf = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`

const bankedText = (row: QuotaRow, now: number): string | undefined => {
  const banked = bankedResets(row.account)
  if (banked === undefined) return undefined
  if (banked.count === 0) return 'No banked resets'
  return [
    countOf(banked.count, 'banked reset', 'banked resets'),
    ...banked.expiries.map(({ iso, status }) => {
      const at = Date.parse(iso)
      return `${at > now ? `expires in ${spanText(at - now)}` : `expired ${spanText(now - at)} ago`}${status === undefined ? '' : ` (${status})`}`
    }),
    ...(banked.undated === 0 ? [] : [`${banked.undated} undated`]),
  ].join(' · ')
}

export const accountDetailView = ({ row, now }: { readonly row: QuotaRow; readonly now: number }): AccountDetailView => {
  const { account } = row
  const { at, outcome, reason } = account.latestAttempt
  const failed = account.retained || (reason !== undefined && reason !== null) || !OK_OUTCOMES.includes(outcome)
  return {
    id: row.accountId,
    plan: account.plan?.displayName,
    unreadable: row.summary._tag === 'incompatible' ? row.summary.reason : undefined,
    limits: account.limits.toSorted((a, b) => a.order.position - b.order.position).map(limit => {
      const label = limit.label === '' ? limit.limitId : limit.label
      const unreported = limit.observation.state === 'unreported'
      const reading = limitReading(limit, now)
      const note = unreported ? 'not reported' : reading.level === 'limit' ? 'at limit' : reading.level === 'unknown' ? reading.state : undefined
      return {
        id: limit.limitId, label, fill: reading.fill, level: reading.level, note, reset: reading.reset,
        percent: unreported ? 'unreported' : reading.percent,
        words: [label, unreported ? 'not reported' : reading.state, reading.reset.replace(/^resets /, 'resets in ')].join(', '),
      }
    }),
    banked: bankedText(row, now),
    reading: { age: ageText(account.valueObservedAt, now), held: account.retained },
    attempt: failed ? [`${outcome} ${spanText(now - Date.parse(at))} ago`, reason].filter(Boolean).join(': ') : undefined,
  }
}

/** The selected account, else the list's first: a refresh never selects on the reader's behalf. */
export const selectedRow = ({ state, view, selected }: {
  readonly state: QuotaState
  readonly view: Extract<UsageSectionView, { _tag: 'ready' }>
  readonly selected: string | undefined
}): QuotaRow | undefined => {
  const rows = state._tag === 'declared' ? state.last?.rows ?? [] : []
  return rows.find(row => row.accountId === selected) ?? rows.find(row => row.accountId === view.rows[0]?.id)
}

/** Days the history chart spans, the open day included. */
export const HISTORY_DAYS = 30

/** `zero` is a reported day with nothing recorded; `unreported` is a day the producer has no row or column for. */
export type HistoryDayState = 'recorded' | 'zero' | 'unrated' | 'unreported'

export interface HistoryBarView {
  readonly day: string
  readonly state: HistoryDayState
  /** 0–100 of the window's largest day; `undefined` when the day has no value in this unit. */
  readonly fill: number | undefined
  /** The open day: still accumulating, drawn as partial. */
  readonly open: boolean
  readonly words: string
  /** The table's value cell: the day's value in the chart's unit, `undefined` when it has none. */
  readonly value: string | undefined
  /** The table's state cell: `partial` for the open day, then `zero`, `unrated` or `unreported` when not plainly recorded. */
  readonly status: string
}

const DAY_MS = 86_400_000
const daysBefore = (day: string, back: number) => new Date(Date.parse(`${day}T00:00:00Z`) - back * DAY_MS).toISOString().slice(0, 10)

export type HistoryView =
  | { readonly _tag: 'not-configured' }
  | { readonly _tag: 'loading' }
  | { readonly _tag: 'unavailable'; readonly reason: string }
  | { readonly _tag: 'incompatible'; readonly reason: string }
  | {
      readonly _tag: 'chart'
      /** Names the unit: "Daily spend, USD · last 30 days" or "Daily tokens · last 30 days". */
      readonly caption: string
      /** The table's unit column: `USD` or `tokens`. */
      readonly unit: string
      readonly bars: readonly HistoryBarView[]
      readonly summary: string
      /** What the bars leave out: tokens below USD coverage, days the producer dropped, a failed refresh. */
      readonly notes: readonly string[]
    }

/** A source with no history for the account (no relay, or none published) reads as not configured. */
const failureView = ({ kind, reason }: Extract<HistoryState['attempt'], { _tag: 'failed' }>): HistoryView =>
  kind === 'unavailable' ? { _tag: 'not-configured' } : kind === 'incompatible' ? { _tag: 'incompatible', reason } : { _tag: 'unavailable', reason }

/**
 * The `HISTORY_DAYS` calendar days ending on the producer's open day, in USD only where the producer's
 * coverage admits it. Days before the series are not reported, never zero; a day without a rate is
 * left out of the USD subtotal and counted, never read as $0.
 */
export const historyView = ({ ledgerAccountId, history, preferred }: {
  readonly ledgerAccountId: string | undefined
  readonly history: HistoryState
  readonly preferred: HistoryUnit
}): HistoryView => {
  if (ledgerAccountId === undefined) return { _tag: 'not-configured' }
  const { last, attempt } = history
  if (last === undefined) return attempt._tag === 'failed' ? failureView(attempt) : { _tag: 'loading' }
  const { unit, belowCoverage } = effectiveUnit({ preferred, series: last.series })
  const { openDay } = last.series
  const byDay = new Map(last.series.days.map(day => [day.day, day]))
  const valueOf = (day: HistoryDay | undefined) => day === undefined ? undefined : unit === 'usd' ? day.usd : day.tokens
  const days = Array.from({ length: HISTORY_DAYS }, (_, index) => {
    const day = daysBefore(openDay, HISTORY_DAYS - 1 - index)
    const reported = byDay.get(day)
    const value = valueOf(reported)
    const state: HistoryDayState = reported === undefined ? 'unreported' : value === undefined ? 'unrated' : value === 0 ? 'zero' : 'recorded'
    return { day, value, state, open: day === openDay }
  })
  const text = ({ value, state }: (typeof days)[number]) =>
    state === 'unreported' ? 'not reported' : value === undefined ? 'no rate' : formatUnitValue({ value, unit })
  const max = Math.max(0, ...days.map(day => day.value ?? 0))
  const peak = max === 0 ? undefined : days.find(day => day.value === max)
  const open = days.at(-1)!
  const full = days.filter(day => !day.open && day.state !== 'unreported')
  const rated = full.filter(day => day.value !== undefined)
  const unrated = full.length - rated.length
  const unreported = days.filter(day => day.state === 'unreported').length
  const total = formatUnitValue({ value: rated.reduce((sum, day) => sum + day.value!, 0), unit })
  return {
    _tag: 'chart',
    caption: `${unit === 'usd' ? 'Daily spend, USD' : 'Daily tokens'} · last ${HISTORY_DAYS} days`,
    unit: unit === 'usd' ? 'USD' : 'tokens',
    bars: days.map(day => ({
      day: day.day, state: day.state, open: day.open,
      fill: day.value === undefined ? undefined : max === 0 ? 0 : day.value / max * 100,
      words: `${day.day}${day.open ? ' so far' : ''}: ${text(day)}`,
      value: day.value === undefined ? undefined : formatUnitValue({ value: day.value, unit }),
      status: [...(day.open ? ['partial'] : []), ...(day.state === 'recorded' ? [] : [day.state])].join(', ') || 'recorded',
    })),
    summary: [
      ...(full.length === 0 ? []
        : rated.length === 0 ? [`no rate for ${countOf(full.length, 'full day', 'full days')}`]
        : unrated > 0 ? [`${total} rated subtotal over ${countOf(rated.length, 'full day', 'full days')}`, `${unrated} without a rate`]
        : [`${total} over ${countOf(rated.length, 'full day', 'full days')}`]),
      ...(unreported === 0 ? [] : [`${countOf(unreported, 'day', 'days')} not reported`]),
      ...(peak === undefined ? [] : [`peak ${formatUnitValue({ value: max, unit })} on ${peak.day}`]),
      `today so far ${text(open)}`,
    ].join(' · '),
    notes: [
      ...(belowCoverage ? ['USD coverage is below the producer’s gate, so this shows tokens'] : []),
      ...(last.series.omittedDays > 0 ? [`The producer dropped ${countOf(last.series.omittedDays, 'older day', 'older days')}`] : []),
      ...(attempt._tag === 'failed' ? [`Refresh failed: ${attempt.reason}; showing the last good reading`] : []),
    ],
  }
}
