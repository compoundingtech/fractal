/** The few facts unread needs: app items and the kit's runtime items both carry them. */
export interface UnreadItem {
  readonly _tag: string
  readonly id: string
  readonly at?: string
  readonly role?: string
  readonly sender?: { readonly kind: string }
  readonly author?: string
  readonly from?: string
}
type ConversationItem = UnreadItem

/**
 * What "new since you last looked" counts: the other side's prose. Assistant text and Fractal
 * messages not written by a person; never tool calls, status, usage or the person's own prompts.
 */
export const countsAsNew = (item: ConversationItem): boolean => {
  if (item._tag === 'Text') return item.role === 'assistant'
  if (item._tag === 'Message') return item.sender?.kind !== 'human' && item.author?.startsWith('person/') !== true && item.from?.startsWith('person/') !== true
  return false
}

/** An item's time in ms, or NaN when it has none. */
export const itemAtMs = (item: ConversationItem): number => item.at === undefined ? NaN : Date.parse(item.at)

/** The newest time among the items, or undefined when none has one. */
export const latestAtMs = (items: Iterable<ConversationItem>): number | undefined => {
  let latest: number | undefined
  for (const item of items) {
    const at = itemAtMs(item)
    if (Number.isFinite(at) && (latest === undefined || at > latest)) latest = at
  }
  return latest
}

export interface Unread {
  /** The first new item after the position, where the divider goes. */
  readonly firstId?: string
  readonly count: number
}
const none: Unread = { count: 0 }

/** The new items strictly after `seenAtMs`, in the items' order. Unknown position: nothing is new. */
export const unreadAfter = (items: readonly ConversationItem[], seenAtMs: number | undefined): Unread => {
  if (seenAtMs === undefined) return none
  let firstId: string | undefined
  let count = 0
  for (const item of items) {
    if (!countsAsNew(item)) continue
    const at = itemAtMs(item)
    if (!Number.isFinite(at) || at <= seenAtMs) continue
    firstId ??= item.id
    count++
  }
  return count === 0 ? none : { firstId, count }
}

/** Unread state for a row or tab: an exact count, `some` when only a newer time is known, or nothing. */
export type UnreadMark = number | 'some'

/**
 * A conversation's mark without its transcript: `observed` are the times of the new items this
 * window last saw (exact while nothing newer is known), `latestProseAt` the roster's newest prose.
 */
export const unreadMark = ({ seenAtMs, observed, latestProseAt }: {
  readonly seenAtMs: number | undefined
  readonly observed?: readonly number[]
  readonly latestProseAt?: number
}): UnreadMark | undefined => {
  if (seenAtMs === undefined) return undefined
  const observedLatest = observed === undefined || observed.length === 0 ? undefined : Math.max(...observed)
  const behind = latestProseAt !== undefined && (observedLatest === undefined || latestProseAt > observedLatest)
  // Newer prose than this window saw makes any count a lower bound: say only that something is new.
  if (behind && latestProseAt! > seenAtMs) return 'some'
  const count = observed?.filter(at => at > seenAtMs).length ?? 0
  return count > 0 ? count : undefined
}

/** A tab's or row's short form: the count, or a dot when only "something new" is known. */
export const unreadLabel = (mark: UnreadMark): string => mark === 'some' ? '•' : mark > 99 ? '99+' : String(mark)
