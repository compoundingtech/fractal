import { describe, expect, it } from 'vitest'
import { decodeConversationChunk } from '@st3/sdk/effect'
import { LiveTimeline } from './fromTimeline.ts'

const entry = (type: string, sequence: number, body: unknown) => ({ id: `timeline-entry/example/${sequence}`, sequence, revision: 1, final: true, role: type === 'content' ? 'assistant' : 'tool', timestamp: '2026-01-15T12:00:00Z', type, body })
const block = (kind: string, fields: object) => ({ id: `block/${kind}`, kind, source_type: 'omp', payload: { body_ref: true }, ...fields })
const frame = (items: unknown[]) => decodeConversationChunk({ kind: 'conversation', collection: 'conversation', id: 'follow/example', session_id: 'session/example', replace: true, has_more: false, items: items as never })

describe('optional native block facts', () => {
  it('reads deployed result metadata and retains negotiated views through SDK decoding', () => {
    const timeline = new LiveTimeline()
    timeline.apply(frame([
      entry('tool_call', 1, { call_id: 'call/example', name: 'bash', arguments: { command: 'printf ready' }, blocks: [block('tool_call', { view: { type: 'bash', tool: 'bash', command: 'printf ready' } })] }),
      entry('tool_result', 2, { call_id: 'call/example', content: 'ready', media_type: 'text/plain', status: 'success', blocks: [block('tool_output', { metadata: { exitCode: 0, wallTimeMs: 0, future: { unit: 'ticks', value: 4 } }, view: { type: 'bash', tool: 'bash', exit_code: 0, wall_ms: 0 } })] }),
    ]))
    const call = timeline.project().items[0]
    expect(call).toMatchObject({ _tag: 'ToolCall', view: { type: 'bash' }, result: { metadata: { exitCode: 0, wallTimeMs: 0, future: { unit: 'ticks', value: 4 } }, view: { exit_code: 0, wall_ms: 0 } } })
  })
  it('keeps old fallback-only pages readable without inventing facts', () => {
    const timeline = new LiveTimeline()
    timeline.apply(frame([entry('content', 1, { text: 'Ready.', media_type: 'text/plain' })]))
    expect(timeline.project().items[0]).toMatchObject({ _tag: 'Text', text: 'Ready.' })
    expect(timeline.project().items[0]).not.toHaveProperty('blocks')
    expect(timeline.project().items[0]).not.toHaveProperty('assistantMetadata')
  })
})
