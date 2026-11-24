/**
 * What the Usage pane shows for one quota account: every limit on its own line, banked resets and why
 * the reading is as old as it is. Limits read with the sidebar's tiers (`limitReading`); staleness is
 * the producer's `retained` flag, never reclassified here.
 */
import { bankedResets, type QuotaRow, type QuotaState } from './model.ts'
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
