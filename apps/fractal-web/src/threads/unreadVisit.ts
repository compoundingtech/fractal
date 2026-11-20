import type { ThreadUnreadMark, ThreadUnreadReminder } from './unread-action-port.ts'

/** Foreground loss suspends a visit; only deliberate pane departure ends it. */
export type UnreadVisit =
  | { readonly _tag: 'Away' }
  | { readonly _tag: 'Visiting'; readonly captured: readonly ThreadUnreadMark[]; readonly attempted: ReadonlySet<string>; readonly sticky: boolean }

export const awayVisit: UnreadVisit = { _tag: 'Away' }

/** Capture once at entry. Unknown intent is not retroactively captured during this visit. */
export const enterUnreadVisit = (reminder: ThreadUnreadReminder | undefined): UnreadVisit => ({
  _tag: 'Visiting', captured: reminder?.marks.map(mark => ({ ...mark })) ?? [], attempted: new Set(), sticky: false,
})

/** A deliberate mark protects this visit even before its authoritative summary arrives. */
export const holdUnreadVisit = (visit: UnreadVisit): UnreadVisit => visit._tag === 'Away' ? visit : { ...visit, sticky: true }

export interface UnreadView {
  readonly foreground: boolean
  readonly visibleMessageIds: readonly string[]
  readonly currentReminder: ThreadUnreadReminder | undefined
}

/** Observed-remove only: never clear a concurrent mark or a different mark's unseen anchor. */
export const planUnreadView = (visit: UnreadVisit, view: UnreadView): {
  readonly visit: UnreadVisit
  readonly observed_marks: readonly string[]
} => {
  if (visit._tag === 'Away' || visit.sticky || !view.foreground) return { visit, observed_marks: [] }
  const visible = new Set(view.visibleMessageIds)
  const active = new Map(view.currentReminder?.marks.map(mark => [mark.mark_id, mark.anchor_message_id]))
  const observed_marks = visit.captured.flatMap(mark =>
    !visit.attempted.has(mark.mark_id) && active.get(mark.mark_id) === mark.anchor_message_id && visible.has(mark.anchor_message_id)
      ? [mark.mark_id] : [],
  )
  if (observed_marks.length === 0) return { visit, observed_marks }
  // One submission per ID per visit. Unknown outcomes are not silently retried with a new operation.
  return { visit: { ...visit, attempted: new Set([...visit.attempted, ...observed_marks]) }, observed_marks }
}
