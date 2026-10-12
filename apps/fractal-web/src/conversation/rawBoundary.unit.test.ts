import { describe, expect, it } from 'vitest'
import { Effect, Metric } from 'effect'
import { decodeConversationChunk } from '@st3/sdk/effect'
import { LiveTimeline } from './fromTimeline.ts'
import { inspectPreview } from './inspect.ts'
import { degradedCounter, unsupportedCounter } from './telemetry.ts'
import { prepareTranscriptTurns } from '../web/conversationTranscript.ts'

const block = (kind: string, fields: object = {}) => ({ id: `block/${kind}`, kind, source_type: 'omp', payload: {}, ...fields })
const entry = (blocks: readonly unknown[], fields: object = {}) => ({ id: 'timeline-entry/raw/1', sequence: 1, revision: 1, final: true, role: 'system', timestamp: '2026-10-12T12:00:00Z', type: 'content', body: { text: '[omp raw]\n{"raw":{}}', media_type: 'text/plain', blocks }, ...fields })
const frame = (items: readonly unknown[], replace = true) => decodeConversationChunk({ kind: 'conversation', collection: 'conversation', id: 'follow/raw', session_id: 'session/raw', replace, has_more: false, items })
const project = (raw: unknown) => { const timeline = new LiveTimeline(); timeline.apply(frame([raw])); return timeline.project().items }

describe('native closed transcript boundary', () => {
  it('hides internal tool starts', () => {
    expect(project(entry([block('status', { visibility: 'internal', view: { type: 'tool_start', call_id: 'call/a', tool: 'bash', started_at: '2026-10-12T12:00:00Z' } })]))).toEqual([])
  })
  it.each(['internal', 'hidden-by-harness'])('hides %s native tool invocations', visibility => {
    expect(project(entry([], { type: 'tool_call', role: 'assistant', body: {
      call_id: 'call/hidden', name: 'bash', arguments: { command: 'INTERNAL_ONLY_COMMAND' },
      blocks: [block('tool_invocation', { visibility })],
    } }))).toEqual([])
  })
  it.each(['internal', 'hidden-by-harness'])('hides %s orphan native tool results', visibility => {
    expect(project(entry([], { type: 'tool_result', role: 'tool', body: {
      call_id: 'call/hidden', content: 'INTERNAL_ONLY_OUTPUT', media_type: 'text/plain', status: 'success',
      blocks: [block('tool_output', { visibility })],
    } }))).toEqual([])
  })
  it.each(['internal', 'hidden-by-harness'])('does not join %s result payloads into visible calls', visibility => {
    const timeline = new LiveTimeline()
    timeline.apply(frame([
      entry([], { type: 'tool_call', role: 'assistant', body: { call_id: 'call/joined', name: 'bash', arguments: { command: 'visible command' }, blocks: [block('tool_invocation')] } }),
      entry([], { id: 'result', sequence: 2, type: 'tool_result', role: 'tool', body: { call_id: 'call/joined', content: 'INTERNAL_ONLY_OUTPUT', media_type: 'text/plain', status: 'success', blocks: [block('tool_output', { visibility })] } }),
    ]))
    const items = timeline.project().items
    expect(items).toMatchObject([{ _tag: 'ToolCall', input: { command: 'visible command' } }])
    expect(items[0]).not.toHaveProperty('result')
    expect(JSON.stringify(items)).not.toContain('INTERNAL_ONLY_OUTPUT')
  })
  it.each(['internal', 'hidden-by-harness'])('hides %s non-content message and status bodies', visibility => {
    for (const type of ['message', 'status']) {
      expect(project(entry([], { type, body: { message_id: 'message/hidden', title: 'INTERNAL_ONLY_MESSAGE', status: 'completed', detail: 'INTERNAL_ONLY_STATUS', blocks: [block(type === 'message' ? 'text' : 'status', { visibility })] } }))).toEqual([])
    }
  })
  it('retains visible tool fallback and mixed unsupported markers', () => {
    const items = project(entry([], { type: 'tool_call', role: 'assistant', body: {
      call_id: 'call/mixed', name: 'bash', arguments: { command: 'visible command' },
      blocks: [block('tool_invocation', { visibility: 'internal' }), block('future-tool'), block('tool_invocation')],
    } }))
    expect(items).toMatchObject([{ _tag: 'Unsupported', recordKind: 'future-tool' }, { _tag: 'ToolCall', input: { command: 'visible command' } }])
  })
  it('retains visible results as orphans when their invocation is hidden', () => {
    const timeline = new LiveTimeline()
    timeline.apply(frame([
      entry([], { type: 'tool_call', role: 'assistant', body: { call_id: 'call/mixed', name: 'bash', arguments: { command: 'INTERNAL_ONLY_COMMAND' }, blocks: [block('tool_invocation', { visibility: 'internal' })] } }),
      entry([], { id: 'visible-result', sequence: 2, type: 'tool_result', role: 'tool', body: { call_id: 'call/mixed', content: 'Visible output', media_type: 'text/plain', status: 'success', blocks: [block('tool_output')] } }),
    ]))
    expect(timeline.project().items).toMatchObject([{ _tag: 'ToolCall', callSeen: false, result: { content: 'Visible output' } }])
  })
  it.each([
    [{ id: 'Impl2239-2', state: 'completed', duration_ms: 15 }, 'completed'],
    [{ id: 'process/1', name: 'Supervised process', state: 'failed', exit_code: 1 }, 'failed'],
  ])('uses typed single-job facts without a matching envelope', (job, state) => {
    expect(project(entry([block('job', { view: { type: 'job', jobs: [job] } })]))).toMatchObject([{ _tag: 'Event', kind: 'background-job', notice: { _tag: 'BackgroundJob', jobId: job.id, state, payload: '', raw: '' } }])
  })
  it('lists multiple jobs without copying the envelope', () => {
    expect(project(entry([block('job', { view: { type: 'job', jobs: [{ id: 'a', name: 'Alpha', state: 'completed' }, { id: 'b', name: 'Beta', state: 'completed' }] } })]))).toMatchObject([{ _tag: 'Event', title: '2 background jobs completed', text: 'Alpha\nBeta' }])
  })
  it('marks unsupported native and injected kinds, even with system events hidden', () => {
    for (const [kind, source, reason] of [['unknown', 'model_usage', 'unrecognized-record'], ['future-kind', 'future', 'unknown-block-kind']]) {
      const items = project(entry([block('source_record', { visibility: 'internal' }), block(kind!, { source_type: source })]))
      expect(items).toMatchObject([{ _tag: 'Unsupported', harness: 'omp', recordKind: kind === 'unknown' ? source : kind, reason }])
      expect(prepareTranscriptTurns(items, { showSystemEvents: false, firstTurnComplete: true })[0]?.items).toEqual(items)
      expect(items[0]).not.toHaveProperty('blocks')
    }
  })
  it('keeps a good block alongside a dropped block marker', () => {
    const items = project(entry([{ id: 'bad', kind: 23, source_type: 'omp', payload: {} }, block('text')], { role: 'assistant', body: { text: 'Good prose', media_type: 'text/plain', blocks: [{ id: 'bad', kind: 23, source_type: 'omp', payload: {} }, block('text')] } }))
    expect(items).toMatchObject([{ _tag: 'Unsupported', reason: 'undecodable-block' }, { _tag: 'Text', text: 'Good prose' }])
  })
  it('turns system prose into a harness notice', () => {
    const items = project(entry([block('text')], { body: { text: '<system-reminder>Remember to save</system-reminder>', media_type: 'text/plain', blocks: [block('text')] } }))
    expect(items).toMatchObject([{ _tag: 'Event', kind: 'harness-message', title: 'Harness message', text: 'Full result available in details.' }])
  })
  it('keeps native reasoning metadata', () => {
    const blocks = [block('reasoning', { metadata: { model: 'test-model' } })]
    expect(project(entry(blocks, { role: 'assistant', body: { text: '[reasoning]\nA thought', media_type: 'text/plain', blocks } }))).toMatchObject([
      { _tag: 'Reasoning', text: 'A thought', assistantMetadata: { model: 'test-model' } },
    ])
  })
  it('joins degraded results while retaining result-position markers and counting revisions once', () => {
    const counter = degradedCounter.pipe(Metric.withAttributes({ kind: 'tool_output', view_type: 'bash' }))
    const before = Effect.runSync(Metric.value(counter)).count
    const call = entry([], { type: 'tool_call', role: 'assistant', body: { call_id: 'call/test', name: 'bash', arguments: {} } })
    const result = entry([], { id: 'timeline-entry/raw/2', sequence: 2, type: 'tool_result', role: 'tool', body: {
      call_id: 'call/test', content: 'Good result', media_type: 'text/plain', status: 'success',
      blocks: [block('tool_output', { view: { type: 'bash', tool: 'bash', env_keys: [], wall_ms: 1.5 } }), { id: 'bad-result-block', kind: 2, source_type: 'omp', payload: {} }],
    } })
    const timeline = new LiveTimeline()
    timeline.apply(frame([call, result]))
    const projected = timeline.project().items
    expect(projected).toMatchObject([
      { _tag: 'ToolCall', result: { content: 'Good result', blocks: [{ kind: 'tool_output' }] } },
      { _tag: 'Unsupported', id: 'timeline-entry/raw/2#bad-result-block', reason: 'undecodable-block' },
    ])
    timeline.project()
    timeline.apply(frame([call, result])); timeline.project()
    expect(Effect.runSync(Metric.value(counter)).count - before).toBe(1)
    timeline.apply(frame([{ ...result, revision: 2 }], false)); timeline.project()
    expect(Effect.runSync(Metric.value(counter)).count - before).toBe(2)
  })
  it('updates telemetry once per entry revision, not repeated projection or activity rebuild', () => {
    const counter = unsupportedCounter.pipe(Metric.withAttributes({ harness: 'st', record_kind: 'telemetry-kind', reason: 'unknown-block-kind' }))
    const value = () => Effect.runSync(Metric.value(counter)).count
    const before = value()
    const raw = entry([block('telemetry-kind', { source_type: 'test' })])
    const timeline = new LiveTimeline()
    timeline.apply(frame([raw])); timeline.project(); timeline.project()
    timeline.apply(frame([raw])); timeline.project()
    expect(value() - before).toBe(1)
    timeline.apply(frame([{ ...raw, revision: 2 }], false)); timeline.project()
    expect(value() - before).toBe(2)
  })
})

describe('inspect preview', () => {
  it('redacts adversarial keys and token strings before UTF-8 capping', () => {
    const raw = { authorization: 'Bearer never-show', nested: { api_key: 'secret', password: 'hunter2', safe: 'ghp_abcdef sk-abcdef xoxb-abcdef eyJabc.def.ghi' }, content: Array.from({ length: 100 }, () => '界'.repeat(600)) }
    const preview = inspectPreview(raw)
    expect(preview.bytes).toBe(new TextEncoder().encode(JSON.stringify(raw)).length)
    expect(new TextEncoder().encode(preview.text).length).toBeLessThanOrEqual(4096)
    expect(preview.truncated).toBe(true)
    expect(preview.text).not.toMatch(/never-show|hunter2|abcdef|eyJabc|"secret"/)
    expect(preview.text).toContain('[redacted]')
  })
})
