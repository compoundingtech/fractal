// @vitest-environment node
import { describe, expect, it } from 'vitest'

import type { ConversationItem } from './model.ts'
import { countsAsNew, latestAtMs, unreadAfter, unreadMark } from './unread.ts'

const at = (ms: number) => new Date(ms).toISOString()
const text = (id: string, role: 'user' | 'assistant', ms: number): ConversationItem =>
  ({ _tag: 'Text', id, role, text: id, attachments: [], streaming: false, at: at(ms) }) as ConversationItem
const tool = (id: string, ms: number): ConversationItem =>
  ({ _tag: 'ToolCall', id, callId: id, name: 'bash', input: {}, status: 'completed', callSeen: true, at: at(ms) }) as unknown as ConversationItem
const message = (id: string, ms: number, from: string): ConversationItem =>
  ({ _tag: 'Message', id, messageId: id, from, author: from, text: id, at: at(ms) }) as ConversationItem

describe('what counts as new', () => {
  it('counts the other side\'s prose, never tools or the person\'s own words', () => {
    expect(countsAsNew(text('a', 'assistant', 1))).toBe(true)
    expect(countsAsNew(text('u', 'user', 1))).toBe(false)
    expect(countsAsNew(tool('t', 1))).toBe(false)
    expect(countsAsNew(message('m', 1, 'agent/scout'))).toBe(true)
    expect(countsAsNew(message('mine', 1, 'person/ada'))).toBe(false)
  })
})

describe('unread after a position', () => {
  const items = [text('u1', 'user', 10), text('a1', 'assistant', 20), tool('t1', 30), text('a2', 'assistant', 40), message('m1', 50, 'agent/scout')]

  it('places the divider at the first new item after the position and counts prose only', () => {
    expect(unreadAfter(items, 20)).toEqual({ firstId: 'a2', count: 2 })
    expect(unreadAfter(items, 5)).toEqual({ firstId: 'a1', count: 3 })
  })

  it('shows nothing when the person has seen the tail, or has never looked', () => {
    expect(unreadAfter(items, 50)).toEqual({ count: 0 })
    expect(unreadAfter(items, undefined)).toEqual({ count: 0 })
  })

  it('knows the newest time of any item', () => {
    expect(latestAtMs(items)).toBe(50)
    expect(latestAtMs([])).toBeUndefined()
  })
})

describe('the mark on a tab or row', () => {
  it('is an exact count from what this window saw', () => {
    expect(unreadMark({ seenAtMs: 20, observed: [20, 40, 50], latestProseAt: 50 })).toBe(2)
  })

  it('is a dot, not a stale count, when the roster knows newer prose than this window saw', () => {
    expect(unreadMark({ seenAtMs: 20, observed: [20, 40], latestProseAt: 90_000 })).toBe('some')
    expect(unreadMark({ seenAtMs: 20, latestProseAt: 90_000 })).toBe('some')
  })

  it('is nothing once seen, or before the person ever looked', () => {
    expect(unreadMark({ seenAtMs: 90, observed: [40], latestProseAt: 90 })).toBeUndefined()
    expect(unreadMark({ seenAtMs: undefined, latestProseAt: 90 })).toBeUndefined()
  })
})

describe('clock skew between items and the roster', () => {
  it('reads roster prose within a moment of what was seen as already seen', () => {
    expect(unreadMark({ seenAtMs: 20_000, observed: [20_000], latestProseAt: 20_050 })).toBeUndefined()
    expect(unreadMark({ seenAtMs: 20_000, latestProseAt: 20_050 })).toBeUndefined()
    expect(unreadMark({ seenAtMs: 20_000, latestProseAt: 30_000 })).toBe('some')
  })
})
