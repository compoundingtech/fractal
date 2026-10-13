import { expect, it } from 'vitest'
import { conversationHistory } from './conversationHistory.ts'
import { observed, type ConversationPage } from '../data/source.ts'

it('keeps unknown and bounded non-pageable history pending, never fabricated complete', () => {
  expect(conversationHistory({ _tag: 'Waiting' })).toEqual({ _tag: 'Pending' })
  expect(conversationHistory(observed({ value: { items: [], hasOlder: true } }))).toEqual({ _tag: 'Pending' })
  expect(conversationHistory(observed({ value: { items: [], hasOlder: false } }))).toEqual({ _tag: 'Complete' })
})
it('maps source states without hiding failures or adding retry policy', async () => {
  let requests = 0
  const loadOlder = async () => { requests += 1 }
  const page: ConversationPage = { items: [], hasOlder: true }
  const more = conversationHistory(observed({ value: page }), loadOlder)
  expect(more).toEqual({ _tag: 'More', loadOlder })
  if (more._tag === 'More') await more.loadOlder()
  expect(conversationHistory(observed({ value: { ...page, historyLoad: { _tag: 'Loading' } } }), loadOlder)).toEqual({ _tag: 'Loading' })
  const failed = conversationHistory(observed({ value: { ...page, historyLoad: { _tag: 'Failed', detail: 'Cursor expired.' } } }), loadOlder)
  expect(failed).toEqual({ _tag: 'Failed', retry: loadOlder, detail: 'Cursor expired.' })
  expect(requests).toBe(1)
  if (failed._tag === 'Failed') await failed.retry()
  expect(requests).toBe(2)
})
