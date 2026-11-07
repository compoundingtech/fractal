import { readFileSync } from 'node:fs'
import { TimelineEntry } from '@fractal/client/schema'
import { Schema } from 'effect'
import { describe, expect, it } from 'vitest'
import { LiveTimeline } from '../conversation/fromTimeline.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'

// Episodes over the shared `wakes` timeline: two wakes, agent mail, and the agent's answers.
const raw = () => JSON.parse(readFileSync(new URL('../../../../fixtures/clients/transcripts/wakes.json', import.meta.url), 'utf8')) as unknown[]
const project = (entries: unknown) => {
  const timeline = new LiveTimeline()
  timeline.apply({ replace: true, hasMore: false, entries: Schema.decodeUnknownSync(Schema.Array(TimelineEntry))(entries) })
  return timeline.project().items
}
const P = 'timeline-entry/session-wake/'

describe('episodes', () => {
  const turns = (agentRef?: string) => prepareTranscriptTurns(project(raw()), { firstTurnComplete: true, agentRef, names: ref => ref === 'agent/example/harbor' ? 'Harbor' : undefined })
  it('opens an episode at each wake and each incoming message, the trigger leading it', () => {
    const prepared = turns('agent/example/quay')
    expect(prepared.map(turn => turn.triggerId)).toEqual([`${P}0a0a0a0a0a0a0a0a-content`, `${P}1b1b1b1b1b1b1b1b-content`, `${P}2c2c2c2c2c2c2c2c-content`])
    expect(prepared[0]!.items.map(item => item.id)).toEqual([`${P}0a0a0a0a0a0a0a0a-content`, `${P}native-3`])
  })
  it('names mail participants: roster name, else the last segment with the full ref', () => {
    const mail = turns('agent/example/quay')[2]!.participants?.[`${P}2c2c2c2c2c2c2c2c-content`]
    expect(mail).toEqual({ from: { label: 'Harbor', ref: 'agent/example/harbor' }, to: { label: 'quay', ref: 'agent/example/quay' }, involvesPerson: false })
  })
  it('keeps mail the agent sends inside the episode it was sent from', () => {
    const items = project(raw()).map(item => item._tag === 'Message' ? { ...item, author: 'agent/example/quay', from: 'agent/example/quay', to: 'agent/example/harbor' } : item)
    const prepared = prepareTranscriptTurns(items, { firstTurnComplete: true, agentRef: 'agent/example/quay' })
    expect(prepared).toHaveLength(2)
    expect(prepared[1]!.items.map(item => item._tag)).toEqual(['Event', 'Message', 'Text'])
  })
  it('gives a settled episode with no settling status the time from its first entry to its last', () => {
    const first = turns('agent/example/quay')[0]!
    expect(first.work.durationMs).toBe(5000)
  })
})
