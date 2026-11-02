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
  // Scan every entry, but aggregate failures: huge's variants contain hundreds of thousands
  // of entries, and per-entry matcher construction costs far more than the identity checks.
  const violations: string[] = []
  for (const item of items) {
    if (!item.id.startsWith('timeline-entry/')) violations.push(`${item.id}: invalid timeline entry ID`)
    if (item.type !== 'message' || !('message_id' in item.body)) continue
    const body = item.body
    if (typeof body.message_id !== 'string') {
      violations.push(`${item.id}: message_id must be a string`)
      continue
    }
    if (body.from == null && body.to == null) {
      if (body.message_id.startsWith('message/')) violations.push(`${item.id}: native header must not become graph mail`)
    } else if (body.message_id.startsWith('message/')) {
      if (typeof body.from !== 'string') violations.push(`${item.id}: graph mail must name its sender`)
      if (typeof body.to !== 'string') violations.push(`${item.id}: graph mail must name its recipient`)
    }
  }
  expect(violations).toEqual([])
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
