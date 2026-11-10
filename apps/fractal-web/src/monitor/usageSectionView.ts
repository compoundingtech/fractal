/**
 * What the sidebar's Usage section shows, over the monitor's own quota state. The anchor limit comes
 * from `model.ts`. Order and colour tiers follow the terminal monitor's default `gov` column
 * (`compare_pressure`, `meter_style`), so this section's tiers are its own: the shared model's 75/90
 * severity is left as it is.
 */
import { dependencyNote, type QuotaRow, type QuotaState } from './model.ts'
import { spanText } from './usageModel.ts'

/** The terminal monitor's colour tiers, as used percentages. */
export const USAGE_TIERS = { warn: 60, high: 80, limit: 100 } as const

/** At limit is the producer's exhausted state or a full reading; the rest are tiers of the reading. */
export type UsageLevel = 'ok' | 'warn' | 'high' | 'limit' | 'unknown'

export interface UsageRowView {
  readonly id: string
  readonly name: string
  /** The anchor limit's own label: 7d, 1w, Mon. */
  readonly window: string
  /** Meter fill, 0 to 100; undefined when there is no current reading to draw. */
  readonly fill: number | undefined
  /** "72%", "100%", or "?". The at-limit `!` is drawn from `level`, so text carries it beside colour. */
  readonly percent: string
  readonly level: UsageLevel
  /** The producer says so; orders above a reading of the same size. */
  readonly exhausted: boolean
  /** "resets 4d", "reset 17d ago", "reset ?". */
  readonly reset: string
  /** A reading that is not current: held by the producer, or read before its window reset. */
  readonly old: string | undefined
  /** Limits beyond the anchor, as a `+N`; `limit` when one of them is what's actually blocking. */
  readonly more: { readonly count: number; readonly atLimit: boolean } | undefined
  /** The row's whole fact as speech; the meter inside it is decorative. */
  readonly words: string
}

export type UsageSectionView =
  | { readonly _tag: 'loading' }
  /** No source declared, none configured on this daemon, or a daemon older than the relay. */
  | { readonly _tag: 'not-configured' }
  | { readonly _tag: 'failed'; readonly kind: 'unreachable' | 'http' | 'timeout' | 'incompatible' | 'unavailable' }
  | { readonly _tag: 'empty'; readonly old: string | undefined }
  | {
    readonly _tag: 'ready'
    readonly rows: readonly UsageRowView[]
    readonly worst: UsageRowView
    readonly atLimit: number
    readonly running: number
    /** Accounts the producer expected but has no reading for. */
    readonly unreported: number
    /** The list as a whole is old: the latest read failed, or the snapshot is past its horizon. */
    readonly old: string | undefined
  }

const levelOf = (percent: number): UsageLevel =>
  percent >= USAGE_TIERS.limit ? 'limit' : percent >= USAGE_TIERS.high ? 'high' : percent >= USAGE_TIERS.warn ? 'warn' : 'ok'

const STATE_WORDS: Readonly<Record<UsageLevel, string>> = { limit: 'at limit', high: 'nearly at limit', warn: 'running short', ok: '', unknown: 'no reading' }

/** Rounds without letting 99.6% read as the 100% that means at limit. */
const percentText = (percent: number): string =>
  `${percent >= USAGE_TIERS.limit ? Math.round(percent) : Math.min(USAGE_TIERS.limit - 1, Math.round(percent))}%`

/** The anchor's unrounded reading; `cellState` keeps only a display percent. */
const usedPercent = (row: QuotaRow, limitId: string): number | undefined => {
  const limit = row.account.limits.find(candidate => candidate.limitId === limitId)
  const fraction = limit?.observation.state === 'reported' ? limit.observation.usage.usedFraction : undefined
  return fraction === undefined || fraction === null ? undefined : fraction * 100
}

const ageText = (iso: string, now: number): string => `${spanText(now - Date.parse(iso))} old`

const rowView = (row: QuotaRow, now: number): UsageRowView => {
  const name = row.accountId
  const old = row.retained ? ageText(row.account.valueObservedAt, now) : undefined
  if (row.summary._tag === 'incompatible') {
    return { id: row.accountId, name, window: '', fill: undefined, percent: '?', level: 'unknown', exhausted: false, reset: 'unreadable', old,
      more: undefined, words: `${name}, this account's limits could not be read` }
  }
  const { anchor, anchorReset, omitted, second } = row.summary
  const resetAt = anchorReset._tag === 'at' ? Date.parse(anchorReset.iso) : undefined
  const expired = resetAt !== undefined && resetAt <= now
  const reading = expired ? undefined : usedPercent(row, anchor.limitId)
  const exhausted = !expired && (anchor.state._tag === 'exhausted' || (anchor.state._tag === 'fraction' && anchor.state.exhausted))
  const level: UsageLevel = exhausted ? 'limit' : reading === undefined ? 'unknown' : levelOf(reading)
  const percent = reading !== undefined ? percentText(reading) : exhausted ? '100%' : '?'
  const reset = resetAt === undefined ? 'reset ?' : expired ? `reset ${spanText(now - resetAt)} ago` : `resets ${spanText(resetAt - now)}`
  const count = omitted + (second === undefined ? 0 : 1)
  const hiddenLimit = level !== 'limit' && row.severity === 'exhausted'
  const state = expired ? 'no reading since its reset' : reading === undefined && !exhausted ? STATE_WORDS.unknown
    : [`${percent} used`, STATE_WORDS[level]].filter(part => part !== '').join(', ')
  return {
    id: row.accountId,
    name,
    window: anchor.label,
    fill: exhausted ? 100 : reading === undefined ? undefined : Math.min(100, Math.max(0, reading)),
    percent,
    level,
    exhausted,
    reset,
    old,
    more: count === 0 ? undefined : { count, atLimit: hiddenLimit },
    words: [
      name, `${anchor.label} window`, state, reset.replace(/^resets /, 'resets in '),
      ...(count === 0 ? [] : [`${count} more ${count === 1 ? 'limit' : 'limits'}${hiddenLimit ? ', one at limit' : ''}`]),
      ...(old === undefined ? [] : [`reading ${old}`]),
    ].join(', '),
  }
}

/** The terminal's descending `gov` sort: exhausted first, then used share, then no reading; account id breaks ties. */
const worstFirst = (left: UsageRowView, right: UsageRowView): number =>
  Number(right.exhausted) - Number(left.exhausted)
  || (right.fill ?? -1) - (left.fill ?? -1)
  || (left.id < right.id ? -1 : left.id > right.id ? 1 : 0)

export const usageSectionView = ({ state, now }: { readonly state: QuotaState; readonly now: number }): UsageSectionView => {
  const note = dependencyNote({ state, now })
  if (note._tag === 'undeclared' || note._tag === 'unavailable') return { _tag: 'not-configured' }
  if (note._tag === 'connecting') return { _tag: 'loading' }
  if (note._tag === 'failed') return { _tag: 'failed', kind: note.failure.kind }
  if (state._tag !== 'declared' || state.last === undefined) return { _tag: 'loading' }
  const old = note._tag === 'current' ? undefined : ageText(note.generatedAt, now)
  const rows = state.last.rows.map(row => rowView(row, now)).toSorted(worstFirst)
  const worst = rows[0]
  if (worst === undefined) return { _tag: 'empty', old }
  const current = rows.filter(row => row.old === undefined)
  return {
    _tag: 'ready',
    rows,
    worst,
    atLimit: current.filter(row => row.level === 'limit').length,
    running: current.filter(row => row.level === 'warn' || row.level === 'high').length,
    unreported: state.last.envelope.coverage.unreportedAccountIds.length,
    old,
  }
}
