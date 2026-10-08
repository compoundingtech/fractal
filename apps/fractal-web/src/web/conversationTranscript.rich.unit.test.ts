import type { CollectionFrame, TimelineEntry } from '@smalltalk/st3-client'
import { decodeConversationChunk } from '@st3/sdk/effect'
import { describe, expect, it } from 'vitest'
import { LiveTimeline } from '../conversation/fromTimeline.ts'
import { mapConversationFeed, prepareTranscriptTurns } from './conversationTranscript.ts'

// Invented payloads reproduce the captured page's shape without committing private transcripts:
// 92 projected items, 88 before the sole user Text, 12 reasoning labels, 36 tool starts.
const richProfile = (): Extract<CollectionFrame, { kind: 'conversation' }> => {
  const items: TimelineEntry[] = []
  const text = (value: string, role: TimelineEntry['role'] = 'assistant') => {
    const sequence = items.length
    items.push({ id: `timeline-entry/profile-${sequence}`, sequence, revision: 1, final: true,
      timestamp: '2026-10-03T00:00:00Z', type: 'content', role,
      body: { media_type: 'text/plain', text: value } })
  }
  for (let index = 0; index < 12; index++) text(`[reasoning]\nThought ${index}`)
  for (let index = 0; index < 36; index++) {
    const sequence = items.length
    items.push({ id: `timeline-entry/profile-${sequence}`, sequence, revision: 1, final: true,
      timestamp: '2026-10-03T00:00:00Z', type: 'tool_call', role: 'assistant',
      body: { call_id: `call-${index}`, name: 'read', arguments: { path: 'example.ts' } } })
    text(`[unrecognized omp entry \`custom\`]\n${JSON.stringify({ raw: { type: 'custom', customType: 'tool_execution_start', data: { toolCallId: `call-${index}` } } })}`, 'system')
  }
  for (let index = 0; index < 4; index++) text(`Earlier assistant answer ${index}`)
  text('A real human prompt.', 'user')
  text('[unrecognized omp entry `custom`]\n{"raw":{"type":"custom","customType":"session_exit"}}', 'system')
  text('Later answer 1')
  text('Later answer 2')
  return { kind: 'conversation', collection: 'conversation', id: 'follow/profile',
    session_id: 'session/profile', replace: true, has_more: true, items }
}

describe('rich page decode, fold and transcript mapping', () => {
  it('recovers reasoning, omits only known internals, retains Unknown and native history', () => {
    const timeline = new LiveTimeline()
    timeline.apply(decodeConversationChunk(richProfile()))
    const items = timeline.project().items
    expect(items).toHaveLength(92)
    expect(items.findIndex(item => item._tag === 'Text' && item.role === 'user')).toBe(88)
    const turns = prepareTranscriptTurns(items, { firstTurnComplete: !timeline.hasOlder })
    const kept = turns.flatMap(turn => turn.prompt === undefined ? turn.items : [turn.prompt, ...turn.items])
    expect(turns).toHaveLength(2)
    expect(turns[0]!.prompt).toBeUndefined()
    expect(kept.filter(item => item._tag === 'Reasoning')).toHaveLength(12)
    expect(kept.filter(item => item._tag === 'Text').some(item => item.text.includes('[reasoning]'))).toBe(false)
    expect(kept.filter(item => item._tag === 'UnknownEvent')).toHaveLength(1)
    expect(kept.find(item => item._tag === 'UnknownEvent')).toMatchObject({ eventType: 'custom', data: { raw: { customType: 'session_exit' } } })
    expect(JSON.stringify(kept)).not.toContain('tool_execution_start')
    expect(kept.some(item => item._tag === 'Notice' && item.kind === 'truncation')).toBe(false)
    const active = mapConversationFeed({ _tag: 'Observed', freshness: 'live', value: {
      items, hasOlder: timeline.hasOlder, observation: timeline.observation,
    } }, {})
    expect(active).toMatchObject({ _tag: 'Observed', history: { _tag: 'HasOlder' } })
    // Current kit requires a prompt: the prepared leading turn activates on its optional-prompt bump.
    expect(active._tag === 'Observed' && active.turns).toHaveLength(1)
  })
})
