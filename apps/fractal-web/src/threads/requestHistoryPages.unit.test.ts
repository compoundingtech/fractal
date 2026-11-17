import { Option } from 'effect'
import * as Native from '@fractal/client/schema/read'
import { describe, expect, it } from 'vitest'
import { readRequestHistoryPages, RequestHistoryPagingError } from './requestHistoryPages.ts'

const key = (episode: string) => ({ source_id: 'step-run/history/review', episode })
const item = (episode: string, status = 'completed'): Native.RequestHistoryItem => Native.decodeUnknownSync(Native.RequestHistoryItem)({
  key: key(episode), complete: true,
  payload: { availability: 'available', title: `Original ${episode}`, text: `Approve ${episode}`, request: null, references: [], provenance: { kind: 'runtime-ask', asking_claim_id: episode } },
  state: { availability: 'available', status, answers: [] },
})
const page = (items: readonly Native.RequestHistoryItem[] = [], next?: string): Native.RequestHistoryPage => ({
  items, boundary_items: [], relations: [], source_episodes: [], complete: true,
  next_cursor: Option.fromUndefinedOr(next), source_support: Option.some('person-step'),
})

describe('authorized request-history pagination', () => {
  it('continues through an empty page with a cursor and preserves episode identity and status', async () => {
    const requests: (string | undefined)[] = []
    const result = await readRequestHistoryPages(async cursor => {
      requests.push(cursor)
      return cursor === undefined ? page([], 'more') : page([item('old'), item('new', 'ready')])
    })
    expect(requests).toEqual([undefined, 'more'])
    expect(result.items.map(value => [value.key.episode, value.state.availability === 'available' ? value.state.status : undefined])).toEqual([['old', 'completed'], ['new', 'ready']])
    expect(Option.isNone(result.next_cursor)).toBe(true)
  })

  it('retains authorized boundary relations and source groups without duplicating episodes', async () => {
    const relation = { predecessor: key('old'), request: key('new'), reason: 'Independent revision' }
    const result = await readRequestHistoryPages(async cursor => cursor === undefined
      ? { ...page([item('old')], 'next'), source_episodes: [{ source_id: key('old').source_id, episodes: [key('old')] }] }
      : { ...page([item('new', 'ready')]), boundary_items: [item('old')], relations: [relation], source_episodes: [{ source_id: key('old').source_id, episodes: [key('old'), key('new')] }] })
    expect(result.items).toHaveLength(2)
    expect(result.boundary_items).toEqual([])
    expect(result.relations).toEqual([relation])
    expect(result.source_episodes[0]?.episodes).toEqual([key('old'), key('new')])
  })

  it('keeps unavailable original payload and evidence completeness instead of borrowing current text', async () => {
    const unavailable: Native.RequestHistoryItem = { ...item('old'), complete: false, payload: { availability: 'unavailable', reason: 'opening-unavailable' } }
    const result = await readRequestHistoryPages(async () => ({ ...page([unavailable]), complete: false }))
    expect(result.complete).toBe(false)
    expect(result.items[0]?.payload).toEqual({ availability: 'unavailable', reason: 'opening-unavailable' })
  })

  it('returns no hidden rows, identifiers, relations or counts for an authorized empty projection', async () => {
    const result = await readRequestHistoryPages(async () => page())
    expect(result.items).toEqual([])
    expect(result.boundary_items).toEqual([])
    expect(result.relations).toEqual([])
    expect(result.source_episodes).toEqual([])
  })

  it('rejects repeated cursors with a typed failure instead of returning partial history', async () => {
    await expect(readRequestHistoryPages(async () => page([], 'same'))).rejects.toMatchObject({ _tag: 'RequestHistoryPagingError', reason: 'repeated-cursor', pages: 2 })
  })

  it('bounds work even when every page is empty and has a new cursor', async () => {
    let count = 0
    await expect(readRequestHistoryPages(async () => page([], String(++count)), 3)).rejects.toBeInstanceOf(RequestHistoryPagingError)
    expect(count).toBe(3)
  })

  it('never publishes a partial page when a later authorization read fails', async () => {
    const denied = { code: 'forbidden' }
    await expect(readRequestHistoryPages(async cursor => {
      if (cursor !== undefined) throw denied
      return page([item('old')], 'next')
    })).rejects.toBe(denied)
  })
})
