import * as Native from '@fractal/client/schema/read'
import { expect, it } from 'vitest'
import type { Feed } from '../data/source.ts'
import { readMarkedUnreadPages } from './markedUnreadPages.ts'

const page = (items: readonly object[] = [], next: string | null = null): Feed<Native.ThreadSummaryPage> => ({
  _tag: 'Observed', freshness: 'live', value: Native.decodeUnknownSync(Native.ThreadSummaryPage)({ items, next_cursor: next }),
})
const row = { id: 'thread/closed', agent_id: 'agent/other', title: 'Private review', title_revision: 'revision/title', revision: 'revision/thread', state: 'closed', unread: false,
  unread_reminder: { anchor_message_id: 'message/anchor', marks: [{ mark_id: 'mark/private', anchor_message_id: 'message/anchor' }] } }

it('traverses empty cursor pages and retains closed reminders from other agents', () => {
  const requested: Array<string | undefined> = []
  expect(readMarkedUnreadPages(cursor => {
    requested.push(cursor)
    return cursor === undefined ? page([], 'second') : cursor === 'second' ? page([], 'third') : page([row])
  })).toMatchObject({ _tag: 'Ready', items: [{ id: row.id, state: 'closed', unread: false }], stale: false })
  expect(requested).toEqual([undefined, 'second', 'third'])
})
it('reports empty only after a terminal page', () => {
  expect(readMarkedUnreadPages(cursor => cursor === undefined ? page([], 'end') : page())).toEqual({ _tag: 'Ready', items: [], stale: false })
})
it('reports a repeated cursor instead of an empty list', () => {
  expect(readMarkedUnreadPages(() => page([], 'loop'))).toEqual({ _tag: 'Error', error: { _tag: 'RepeatedCursor', cursor: 'loop' } })
})
it('reports page-bound exhaustion instead of an empty list', () => {
  expect(readMarkedUnreadPages(cursor => page([], cursor === undefined ? 'a' : `${cursor}a`), 3)).toEqual({ _tag: 'Error', error: { _tag: 'PageBoundExceeded', pages: 3 } })
})
it('reports a fetch failure after an empty page instead of empty', () => {
  expect(readMarkedUnreadPages(cursor => cursor === undefined ? page([], 'next') : { _tag: 'Unavailable', reason: 'failed', detail: 'Disconnected' }))
    .toEqual({ _tag: 'Error', error: { _tag: 'FetchFailed', detail: 'Disconnected' } })
})
it('reports a failed refresh rather than treating its retained empty page as complete', () => {
  const observed = page()
  expect(readMarkedUnreadPages(() => observed._tag === 'Observed' ? { ...observed, error: { reason: 'failed', detail: 'Refresh failed' } } : observed))
    .toEqual({ _tag: 'Error', error: { _tag: 'FetchFailed', detail: 'Refresh failed' } })
})
it('keeps a pending page loading, not empty', () => {
  expect(readMarkedUnreadPages(() => ({ _tag: 'Waiting' }))).toEqual({ _tag: 'Loading' })
})
it('omits confirmed clears while still traversing subsequent pages', () => {
  const cursors: Array<string | undefined> = []
  expect(readMarkedUnreadPages(cursor => {
    cursors.push(cursor)
    return cursor === undefined ? page([{ ...row, unread_reminder: null }], 'later') : page()
  })).toEqual({ _tag: 'Ready', items: [], stale: false })
  expect(cursors).toEqual([undefined, 'later'])
})
