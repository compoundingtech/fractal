import type { ThreadSummary, ThreadSummaryPage } from '@fractal/client/schema/read'
import { Option } from 'effect'
import type { Feed } from '../data/source.ts'

export type MarkedUnreadError =
  | { readonly _tag: 'FetchFailed'; readonly detail: string }
  | { readonly _tag: 'RepeatedCursor'; readonly cursor: string }
  | { readonly _tag: 'PageBoundExceeded'; readonly pages: number }

export type MarkedUnreadRead =
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Ready'; readonly items: readonly ThreadSummary[]; readonly stale: boolean }
  | { readonly _tag: 'Error'; readonly error: MarkedUnreadError }

export const markedUnreadErrorMessage = (error: MarkedUnreadError): string => {
  switch (error._tag) {
    case 'FetchFailed': return `Couldn't load marked-unread threads. ${error.detail}`
    case 'RepeatedCursor': return "Couldn't finish loading marked-unread threads because Fractal repeated a page. Try again."
    case 'PageBoundExceeded': return "Couldn't finish loading marked-unread threads because the page limit was reached. Try again."
  }
}

/** Empty pages are not an empty Inbox: only a terminal cursor establishes completeness. */
export const readMarkedUnreadPages = (
  readPage: (cursor?: string) => Feed<ThreadSummaryPage>,
  maxPages = 100,
): MarkedUnreadRead => {
  const cursors = new Set<string>()
  const items = new Map<string, ThreadSummary>()
  let cursor: string | undefined
  let stale = false
  for (let pages = 0; pages < maxPages; pages++) {
    const page = readPage(cursor)
    if (page._tag === 'Waiting') return { _tag: 'Loading' }
    if (page._tag === 'Unavailable') return { _tag: 'Error', error: { _tag: 'FetchFailed', detail: page.detail } }
    if (page.error !== undefined) return { _tag: 'Error', error: { _tag: 'FetchFailed', detail: page.error.detail } }
    stale ||= page.freshness === 'stale'
    // Exact summary updates can confirm a clear before the collection follow removes its row.
    for (const item of page.value.items) {
      if (Option.isNone(item.unread_reminder)) items.delete(item.id)
      else items.set(item.id, item)
    }
    const next = Option.getOrUndefined(page.value.next_cursor)
    if (next === undefined) return { _tag: 'Ready', items: [...items.values()], stale }
    if (cursors.has(next)) return { _tag: 'Error', error: { _tag: 'RepeatedCursor', cursor: next } }
    cursors.add(next)
    cursor = next
  }
  return { _tag: 'Error', error: { _tag: 'PageBoundExceeded', pages: maxPages } }
}
