import type { TimelineEntry } from '@fractal/client'
import { expect, it } from 'vitest'
import { ANCHOR_MS, catalog, loadWorld, type Slice } from '../src/index.ts'

const entries = (slice: Slice<'conversation'>): TimelineEntry[] => [
  ...slice.state.threads.flatMap((thread) => thread.items),
  ...[...slice.timeline, ...(slice.state.recorded_history ?? [])].flatMap((event) =>
    event._tag === 'thread-create' ? event.thread.items
      : event._tag === 'entries' || event._tag === 'replace' ? event.items : []),
]

const checkSenders = (items: readonly TimelineEntry[]): void => {
  for (const item of items) {
    expect(item.id.startsWith('timeline-entry/')).toBe(true)
    if (item.type !== 'message' || !('message_id' in item.body)) continue
    const body = item.body
    expect(typeof body.message_id).toBe('string')
    if (typeof body.message_id !== 'string') continue
    if (body.from == null && body.to == null) {
      expect(body.message_id.startsWith('message/'), `${item.id}: native header must not become graph mail`).toBe(false)
    } else if (body.message_id.startsWith('message/')) {
      expect(typeof body.from).toBe('string')
      expect(typeof body.to).toBe('string')
    }
  }
}

for (const entry of catalog) {
  it(`${entry.id} preserves native versus graph-mail sender identity`, () => {
    const world = loadWorld(entry.id, { now: ANCHOR_MS })
    checkSenders(entries(world.slices.conversation))
    for (const conversation of world.available.conversation.filter((name) => name !== 'default')) {
      checkSenders(entries(world.with({ conversation }).slices.conversation))
    }
  })
}

it('fleet builder native turns distinguish Robin from Builder by their roles, without invented mail senders', () => {
  const world = loadWorld('fleet-mid-refactor', { now: ANCHOR_MS })
  const builder = world.cast.agents.find((agent) => agent.key === 'builder')!
  const items = world.slices.conversation.state.threads.find((thread) => thread.agent === builder.id)!.items
  const headers = items.filter((item) => item.type === 'message' && 'message_id' in item.body && item.body.from == null && item.body.to == null)
  expect(headers.some((item) => item.role === 'user')).toBe(true)
  expect(headers.some((item) => item.role === 'assistant')).toBe(true)
  for (const item of headers) {
    expect(item.body).toHaveProperty('message_id', expect.stringMatching(/^native\//u))
  }
})
