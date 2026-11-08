import { TimelineEntry } from '@fractal/client/schema'
import { Schema } from 'effect'
import { describe, expect, it } from 'vitest'
import { LiveTimeline } from './fromTimeline.ts'

// A harness that wakes again re-reads the same Fractal message; `delivers` (conversation-delivery-links.v1)
// names the message each native user turn consumed, so repeats are known by link, never by matching text.
const P = 'timeline-entry/session-redeliver/'
const turn = (n: number, text: string, delivers?: string) => [
  { id: `${P}native-${n}`, sequence: n * 2, revision: 1, timestamp: `2026-11-08T09:0${n}:00.000Z`, role: 'user', final: true, type: 'message', body: { message_id: `native-turn-${n}` } },
  { id: `${P}native-${n}-content`, sequence: n * 2 + 1, revision: 1, timestamp: `2026-11-08T09:0${n}:00.000Z`, role: 'user', final: true, type: 'content', body: { media_type: 'text/plain', text }, ...(delivers === undefined ? {} : { delivers }) },
]
const answer = (n: number, text: string) => ({ id: `${P}answer-${n}`, sequence: n * 2 + 1, revision: 1, timestamp: `2026-11-08T09:0${n}:30.000Z`, role: 'assistant', final: true, type: 'content', body: { media_type: 'text/plain', text } })
const project = (entries: unknown[]) => {
  const timeline = new LiveTimeline()
  timeline.apply({ replace: true, hasMore: false, entries: Schema.decodeUnknownSync(Schema.Array(TimelineEntry))(entries) })
  return timeline.project().items
}
const prompt = 'Can you check why the worker typecheck fails after the rename?'

describe('re-delivered harness prompts', () => {
  it('reads a message the harness consumed three times as one prompt, counting the repeats', () => {
    const items = project([...turn(1, prompt, 'message/ask01'), answer(1, 'Looking.'), ...turn(2, prompt, 'message/ask01'), answer(2, 'Still looking.'), ...turn(3, prompt, 'message/ask01')])
    const prompts = items.filter(item => item._tag === 'Text' && item.role === 'user')
    expect(prompts).toHaveLength(1)
    expect(prompts[0]).toMatchObject({ id: `${P}native-1-content`, text: prompt, redelivered: 2 })
    expect(items.filter(item => item._tag === 'Text' && item.role === 'assistant')).toHaveLength(2)
  })
  it('never collapses identical prompts without a link, or prompts that deliver different messages', () => {
    expect(project([...turn(1, prompt), ...turn(2, prompt)]).filter(item => item._tag === 'Text')).toHaveLength(2)
    expect(project([...turn(1, prompt, 'message/ask01'), ...turn(2, prompt, 'message/ask02')]).filter(item => item._tag === 'Text')).toHaveLength(2)
  })
  it('collapses a repeat that arrives in a later frame into the first prompt', () => {
    const timeline = new LiveTimeline()
    const decode = (entries: unknown[]) => Schema.decodeUnknownSync(Schema.Array(TimelineEntry))(entries)
    timeline.apply({ replace: true, hasMore: false, entries: decode([...turn(1, prompt, 'message/ask01'), answer(1, 'Looking.')]) })
    expect(timeline.project().items.find(item => item.id === `${P}native-1-content`)).not.toHaveProperty('redelivered')
    timeline.apply({ replace: false, hasMore: false, entries: decode(turn(2, prompt, 'message/ask01')) })
    const items = timeline.project().items
    expect(items.map(item => item.id)).not.toContain(`${P}native-2-content`)
    expect(items.find(item => item.id === `${P}native-1-content`)).toMatchObject({ redelivered: 1 })
  })
})
