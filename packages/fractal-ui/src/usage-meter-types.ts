/** Presentation facts only: source decoding, ranking and clock policy belong to the caller. */
export type UsageMeterTone = 'ok' | 'warning' | 'critical' | 'exhausted' | 'unknown'
export interface UsageMeterCell {
  readonly label: string
  readonly text: string
  /** Absent is unknown, never a healthy zero. Exhaustion need not have a fraction. */
  readonly fraction?: number
  readonly tone: UsageMeterTone
}
export interface UsageMeterFact { readonly label: string; readonly value: string }
export interface UsageMeterWindow {
  readonly id: string
  readonly label: string
  readonly meter: UsageMeterCell
  readonly facts: readonly UsageMeterFact[]
}
export interface UsageMeterDetail {
  readonly facts: readonly UsageMeterFact[]
  readonly windows: readonly UsageMeterWindow[]
  readonly resets: readonly UsageMeterFact[]
}
export interface UsageMeterRow {
  readonly id: string
  readonly label: string
  readonly identity: string
  /** Caller-owned caption parts; shrink the provider prefix before the account name. Canonical identity stays accessible and in detail. */
  readonly compactIdentity?: { readonly prefix: string; readonly account: string }
  readonly governing: UsageMeterCell
  readonly comparison?: UsageMeterCell
  readonly reset: string
  /** Absolute reset evidence; omitted is unknown, including for ordering. */
  readonly resetAt?: string
  /** Present zero is a reported zero; absent is unreported. */
  readonly resetCount?: number
  readonly omitted: number
  readonly stale: boolean
  readonly note?: string
  readonly detail: UsageMeterDetail
}
export interface UsageMeterHistoryPoint {
  readonly id: string
  /** Period label, for example a calendar day. */
  readonly label: string
  /** Absent is unmeasured and drawn as a gap, never as a zero bar. */
  readonly value?: number
  readonly text: string
  /** The period is still open, so it is drawn distinct from closed periods. */
  readonly partial: boolean
}
/** Usage history of the selected row, in display facts; `text` states the unit and span. */
export type UsageMeterHistory =
  | { readonly kind: 'loading' | 'unavailable'; readonly label: string; readonly text: string }
  | { readonly kind: 'ready'; readonly label: string; readonly text: string; readonly points: readonly UsageMeterHistoryPoint[]; readonly facts: readonly UsageMeterFact[] }
export type UsageMeterSortKey = 'account' | 'governing' | 'reset' | 'comparison' | 'more'
export interface UsageMeterSort { readonly key: UsageMeterSortKey; readonly direction: 'ascending' | 'descending' }
export interface UsageMeterTableProps {
  readonly rows: readonly UsageMeterRow[]
  readonly status: { readonly kind: 'loading' | 'unreachable' | 'incompatible' | 'current' | 'stale' | 'unknown'; readonly text: string }
  readonly sort: UsageMeterSort
  readonly onSort: (sort: UsageMeterSort) => void
  readonly onCycleSort: () => void
  readonly selectedId?: string
  readonly onSelect: (id: string) => void
  readonly onCloseDetail: () => void
  /** History for the selected row; omitted when the source has none. */
  readonly history?: UsageMeterHistory
}
