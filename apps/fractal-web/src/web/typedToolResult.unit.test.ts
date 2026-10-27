import { describe, expect, it } from 'vitest'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { projectTypedToolResult, recordedWebHref } from './typedToolResult.ts'
import { semanticToolFor } from '../conversation/semanticToolModel.ts'

const call = (fields: Partial<WorkLogCall>): WorkLogCall => ({ id: 'recorded-call', title: 'Recorded tool', kind: 'unknown', status: 'success', startedAt: '2026-01-01T00:00:00Z', ...fields })
describe('typed result projection', () => {
  it('retains exact search query/path and recorded matches', () => {
    const result = projectTypedToolResult(call({ kind: 'search', input: { pattern: '  agent.* ', path: 'src/example.ts' }, output: { matches: [{ path: 'src/example.ts', line: 0, text: 'agent()' }], files: ['src/example.ts'] } }))
    expect(result).toMatchObject({ kind: 'search', query: '  agent.* ', path: 'src/example.ts', files: ['src/example.ts'], matches: [{ path: 'src/example.ts', line: 0, text: 'agent()' }] })
    expect(result.body).toBeUndefined()
  })
  it('shows actual web text rather than its JSON envelope', () => {
    const result = projectTypedToolResult(call({ kind: 'web', input: { query: 'observed web query' }, output: JSON.stringify({ content: [{ type: 'text', text: '## Findings\n[Source](https://example.org/source)' }], metadata: { sources: [{ title: 'Example source', url: 'https://example.org/source', snippet: 'Recorded snippet' }] } }) }))
    expect(result).toMatchObject({ kind: 'web', query: 'observed web query', body: '## Findings\n[Source](https://example.org/source)', sources: [{ title: 'Example source', url: 'https://example.org/source', snippet: 'Recorded snippet' }] })
  })
  it('reads negotiated views before legacy fields without inventing task states', () => {
    expect(projectTypedToolResult(call({ kind: 'subagent', view: { tasks: [{ name: 'Index', ref: 'agent://example-index' }] }, input: { tasks: [{ name: 'Old' }] } }))).toMatchObject({ tasks: [{ name: 'Index', ref: 'agent://example-index', state: undefined }] })
  })
  it('keeps wait arrivals distinct from awaited refs and lifecycle', () => {
    expect(projectTypedToolResult(call({ kind: 'wait', status: 'interrupted', input: { handles: ['agent://example-one', 'agent://example-two'] }, metadata: { arrived: [{ id: 'agent://example-one' }] } }))).toMatchObject({ kind: 'wait', awaited: ['agent://example-one', 'agent://example-two'], arrived: ['agent://example-one'], running: false, interrupted: true })
  })
  it('keeps skipped results neutral and omitted facts absent', () => {
    expect(projectTypedToolResult(call({ kind: 'web', skipped: true, output: { arbitrary: 'not prose' } }))).toEqual({ kind: 'web', skipped: true, query: undefined, url: undefined, sources: [] })
  })
  it('does not classify names or malformed envelopes', () => {
    expect(projectTypedToolResult(call({ toolName: 'my_agent_search', output: '{broken' })).kind).toBe('unknown')
    expect(projectTypedToolResult(call({ kind: 'search', output: { matches: [null, 1, 'invalid'], query: 42 } }))).toMatchObject({ query: undefined, matches: [] })
  })
  it('retains native task result state and conversation references over invocation tasks', () => {
    expect(projectTypedToolResult(call({ kind: 'subagent', input: { tasks: [{ name: 'Requested task' }] }, metadata: { details: { results: [{ agent: 'Example worker', status: 'completed', id: 'example-task', conversation: { session_id: 'example-session' } }] } } }))).toMatchObject({ tasks: [{ name: 'example-task', agent: 'Example worker', state: 'completed', ref: 'example-task', sessionId: 'example-session' }] })
    expect(projectTypedToolResult(call({ kind: 'subagent', output: { details: { progress: [{ agent: 'Example worker', status: 'running' }] } } }))).toMatchObject({ tasks: [{ name: 'Example worker', state: 'running' }] })
  })
  it('reports completed waits and native jobId arrivals', () => {
    expect(projectTypedToolResult(call({ kind: 'wait', metadata: { jobs: [{ jobId: 'example-job', status: 'completed' }] } }))).toMatchObject({ arrived: ['example-job'], completed: true })
    expect(projectTypedToolResult(call({ kind: 'wait', skipped: true }))).toMatchObject({ completed: false })
  })
  it('uses the delivered sender, never the waiting seat, in both wait surfaces', () => {
    const metadata = { from: 'WaitingSeat', waited: { from: 'SenderSeat', to: 'WaitingSeat', body: 'Ready.' } }
    const row = semanticToolFor({ _tag: 'ToolCall', id: 'example/wait', callId: 'call/wait', name: 'wait', input: { i: 'Waiting for a reply' }, status: 'success', callSeen: true, at: '2026-01-01T00:00:00Z', result: { content: 'Ready.', metadata, isError: false, at: '2026-01-01T00:00:00Z' } })
    expect(row).toMatchObject({ arrived: ['SenderSeat'], senders: ['SenderSeat'], stillRunning: [] })
    expect(projectTypedToolResult(call({ kind: 'wait', metadata }))).toMatchObject({ arrived: ['SenderSeat'], senders: ['SenderSeat'], stillRunning: [] })
    expect(projectTypedToolResult(call({ kind: 'wait', metadata: { from: 'WaitingSeat' } }))).toMatchObject({ arrived: [], senders: [] })
  })
  it('separates completed deliveries from running and unknown jobs in both wait surfaces', () => {
    const metadata = { jobs: [{ jobId: 'bg/ready', status: 'completed' }, { jobId: 'bg/busy', status: 'running' }, { jobId: 'bg/unknown' }, { jobId: 'bg/failed', status: 'failed' }] }
    const row = semanticToolFor({ _tag: 'ToolCall', id: 'example/wait', callId: 'call/wait', name: 'wait', input: { jobs: ['bg/ready', 'bg/busy'] }, status: 'success', callSeen: true, at: '2026-01-01T00:00:00Z', result: { content: 'One job completed.', metadata, isError: false, at: '2026-01-01T00:00:00Z' } })
    expect(row).toMatchObject({ arrived: ['bg/ready'], stillRunning: ['bg/busy'] })
    expect(projectTypedToolResult(call({ kind: 'wait', metadata }))).toMatchObject({ arrived: ['bg/ready'], stillRunning: ['bg/busy'], completed: true, running: false })
  })
  it('keeps explicit native arrivals authoritative over legacy snapshots', () => {
    expect(projectTypedToolResult(call({ kind: 'wait', view: { arrived: ['bg/native'], awaited: ['bg/native'] }, metadata: { jobs: [{ jobId: 'bg/legacy', status: 'completed' }] } }))).toMatchObject({ arrived: ['bg/native'], awaited: ['bg/native'] })
  })

  it('decodes nested task text arrays before escaped detail JSON', () => {
    const spawn = 'Spawned agent `ExampleWorker` (job `ExampleWorker`).\n\nResults auto-deliver.'
    const output = JSON.stringify([{ type: 'text', text: spawn }])
    const result = projectTypedToolResult(call({ kind: 'subagent', input: { tasks: [{ name: 'Requested task' }] }, output, detail: JSON.stringify(output) }))
    expect(result).toMatchObject({ kind: 'subagent', body: spawn, tasks: [{ name: 'Requested task', requested: true }] })
    expect(result.body).not.toContain('[{"type"')
    expect(result.body).not.toContain('\\n')
  })
  it('shows explicit nested task reports without inferring unknown fields', () => {
    const result = projectTypedToolResult(call({ kind: 'subagent', input: { tasks: [{ name: 'Requested task' }] }, output: { results: [{ agent: 'Example worker', status: 'completed', id: 'example-task', report: '## Outcome\nDone.', schema: { data: 'Nested schema report' }, unknownField: { raw: true } }] } }))
    expect(result).toMatchObject({ tasks: [{ name: 'example-task', agent: 'Example worker', state: 'completed', ref: 'example-task', requested: false, report: '## Outcome\nDone.' }] })
    expect(JSON.stringify(result)).not.toContain('unknownField')
  })
  it('reads schema data only when no explicit task report exists', () => {
    expect(projectTypedToolResult(call({ kind: 'subagent', output: { results: [{ name: 'Example worker', schema: { data: { content: [{ type: 'text', text: 'Schema report' }] } } }] } }))).toMatchObject({ tasks: [{ report: 'Schema report', requested: false }] })
  })
  it('keeps unrecognized JSON arrays raw while preserving literal non-JSON prose', () => {
    expect(projectTypedToolResult(call({ kind: 'subagent', output: '[{"type":"future"}]', detail: '{"content":[{"type":"text","text":"Recovered prose"}]}' }))).toMatchObject({ body: 'Recovered prose' })
    expect(projectTypedToolResult(call({ kind: 'subagent', output: '[{"type":"future"}]' })).body).toBeUndefined()
    expect(projectTypedToolResult(call({ kind: 'subagent', output: '[broken' })).body).toBe('[broken')
  })
  it('keeps requested tasks distinct when no outcome is recorded', () => {
    expect(projectTypedToolResult(call({ kind: 'subagent', input: { tasks: [{ name: 'Requested task' }] } }))).toMatchObject({ tasks: [{ name: 'Requested task', requested: true, state: undefined }] })
  })
  it('links only observed HTTP destinations', () => {
    expect(recordedWebHref('https://example.org/a')).toBe('https://example.org/a')
    expect(recordedWebHref('javascript:alert(1)')).toBeUndefined()
    expect(recordedWebHref('/guessed-relative')).toBeUndefined()
  })
})
