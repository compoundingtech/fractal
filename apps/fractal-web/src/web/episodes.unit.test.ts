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
  it('keeps the Working row at the latest episode when an earlier one has an unanswered call', () => {
    const at = (second: number) => `2026-11-07T08:00:${String(second).padStart(2, '0')}.000Z`
    const prompt = (id: string, second: number) => ({ _tag: 'Text' as const, id, role: 'user' as const, text: id, attachments: [], streaming: false, at: at(second) })
    const items = [
      prompt('first', 0),
      { _tag: 'ToolCall' as const, id: 'call', callId: 'call', name: 'bash', input: {}, status: 'running' as const, callSeen: true, at: at(1) },
      prompt('second', 2),
      { _tag: 'Text' as const, id: 'answer', role: 'assistant' as const, text: 'Still on it.', attachments: [], streaming: false, at: at(3) },
    ]
    const prepared = prepareTranscriptTurns(items, { firstTurnComplete: true })
    expect(prepared.map(turn => turn.work.running)).toEqual([false, true])
  })
  it('reads run status and recoverable driver errors as quiet rows, and shows every one on request', () => {
    const at = (second: number) => `2030-01-01T11:59:${String(second).padStart(2, '0')}.000Z`
    const status = (id: string, value: 'queued' | 'running' | 'waiting', second: number) => ({ _tag: 'Status' as const, id, status: value, at: at(second) })
    const error = (id: string, code: string, text: string, second: number) => ({ _tag: 'Notice' as const, id, kind: 'error' as const, text, detail: `Code: ${code}`, at: at(second) })
    const items = [
      status('s1', 'queued', 50), status('s2', 'queued', 50),
      error('e1', 'transcript-not-bound', 'not bound yet (1)', 50),
      status('s3', 'running', 52),
      error('e2', 'claude-channel-unattached', 'channel unattached', 54),
      status('s4', 'waiting', 55),
      error('e3', 'transcript-not-bound', 'not bound yet (2)', 55),
      error('e4', 'disk-full', 'The disk is full, so the agent stopped.', 56),
      error('e5', 'transcript-not-bound', 'not bound yet (3)', 56),
    ]
    const quiet = prepareTranscriptTurns(items, { firstTurnComplete: true })[0]!.items
    expect(quiet.map(item => item.id)).toEqual(['e2', 's4', 'e4', 'e5'])
    expect(quiet.at(-1)).toMatchObject({ text: 'not bound yet (3)', detail: 'Code: transcript-not-bound · 3 times' })
    expect(quiet.find(item => item.id === 'e4')).toMatchObject({ text: 'The disk is full, so the agent stopped.', detail: 'Code: disk-full' })
    const all = prepareTranscriptTurns(items, { firstTurnComplete: true, showSystemEvents: true })[0]!.items
    expect(all.map(item => item.id)).toEqual(items.map(item => item.id))
  })
})
