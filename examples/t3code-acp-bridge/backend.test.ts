import { describe, expect, it } from 'vitest'

import { FractalClient, type CollectionSocket } from '../../clients/typescript/fractal-client/Client.generated.ts'
import type { Agent, CollectionFrame, Snapshot, TimelineEntry } from '../../clients/typescript/fractal-client/Models.generated.ts'
import { makeBackend, type BridgeUpdate } from './backend.ts'

// Transport adapter tests, not daemon E2E. The existing isolated AppState/axum
// fixtures are Rust cfg(test) helpers, not a subprocess callable from TypeScript.
const snapshot: Snapshot = { id: 'snapshot/fixture', created_at: '2026-10-28T00:00:00Z', host_id: 'host/test', projection_version: 'client-projection.v0', store_index: 1 }
const id = 'agent/test.acp'
const sessionId = `fractal-acp:${encodeURIComponent(id)}`
const entry = (sequence: number, text: string, revision = 1): Extract<TimelineEntry, { type: 'content' }> => ({ id: `entry/${sequence}`, sequence, revision, timestamp: '2026-10-28T00:00:00Z', role: 'assistant', type: 'content', final: true, body: { media_type: 'text/plain', text } })
// Same records as fractal-drivers/src/codex_app_server/tests.rs
// transcript fixtures (task_started/task_complete with exact native turn_id),
// wrapped by fractal-store/src/external_sessions.rs push_unrecognized.
const event = (sequence: number, type: string, turnId: string, error?: string): TimelineEntry => ({
  ...entry(sequence, '[unrecognized codex record `event_msg`]'), role: 'system',
  body: { media_type: 'text/plain', blocks: [{ id: `block/${sequence}`, kind: 'unknown', source_type: 'event_msg', payload: { raw: { type: 'event_msg', payload: { type, turn_id: turnId, ...(error === undefined ? {} : { error }) } } } }] },
})
const fixture = () => {
  const actions: { type: string; parameters: Record<string, unknown>; fence: Record<string, unknown> }[] = []
  let socket!: CollectionSocket
  let closed = false
  let sent!: () => void
  const submitted = new Promise<void>((resolve) => { sent = resolve })
  let subscribed!: () => void
  const startupSubscribed = new Promise<void>((resolve) => { subscribed = resolve })
  let agent: Agent = { id, kind: 'agent', name: 'test.acp', revision: 'revision/agent', updated_at: '2026-10-28T00:00:00Z', state: 'running', driver: 'codex', harness_state: 'idle', observation: 'current', reachability: 'reachable', runtime_ids: ['runtime/test'], incarnation_id: 'incarnation/test', current_session_id: 'session/test', workspace: '/workspace' }
  let history: TimelineEntry[] = [event(1, 'task_started', 'old'), entry(2, 'Old output'), event(3, 'task_complete', 'old')]
  const frame = (value: CollectionFrame) => socket.onmessage?.({ data: JSON.stringify(value) })
  const conversation = (items: TimelineEntry[], replace = false) => frame({ kind: 'conversation', id: 'conversation', collection: 'conversation', session_id: 'session/test', replace, items })
  const client = new FractalClient({ baseUrl: 'http://fixture.invalid', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    let value: unknown
    if (url.pathname.endsWith('/capabilities')) value = { capabilities: [{ id: 'collection.agents.exact-selector', version: 1, state: 'granted' }], limits: { max_page_items: 200, max_event_items: 200, max_wait_ms: 1000 } }
    else if (url.pathname.endsWith('/actions')) {
      const action = JSON.parse(String(init?.body)) as typeof actions[number]
      actions.push(action)
      value = { kind: 'action-result', action_id: 'action/fixture', operation_id: 'operation/fixture', status: 'completed', affected_ids: [action.type === 'agent.create' ? id : 'message/test'], snapshot_id: snapshot.id }
      if (action.type === 'message.send') sent()
    } else if (url.pathname.endsWith('/timeline')) value = { kind: 'timeline-page', items: history, session_id: 'session/test', page: { limit: 200, has_more: false } }
    else if (url.pathname.includes('/agents/')) value = agent
    else throw new Error(`Unexpected generated client request ${url}`)
    return new Response(JSON.stringify({ api_version: 'st3.client.v0', snapshot, value }), { headers: { 'content-type': 'application/json' } })
  }, socket: () => {
    socket = { onopen: null, onmessage: null, onclose: null, onerror: null,
      send: (data) => {
        const command = JSON.parse(data) as { collection: string }
        if (command.collection === 'conversation') conversation(history, true)
        else if (command.collection === 'attention') frame({ kind: 'snapshot', id: 'attention', collection: 'attention', items: [], order: [], has_more: false, snapshot })
        else if (command.collection === 'agents') {
          frame({ kind: 'snapshot', id: 'startup', collection: 'agents', items: [agent], order: [id], has_more: false, snapshot })
          subscribed()
        }
      },
      close: () => { closed = true },
    }
    queueMicrotask(() => socket.onopen?.())
    return socket
  } })
  return { client, actions, submitted, startupSubscribed, conversation, frame, isClosed: () => closed, setAgent: (patch: Partial<Agent>) => { agent = { ...agent, ...patch } }, publishAgent: () => frame({ kind: 'snapshot', id: 'startup', collection: 'agents', items: [agent], order: [id], has_more: false, snapshot }) }
}

describe('Fractal ACP generated-client backend', () => {
  it('creates daemon-owned Codex identity and loads it in a fresh backend instance', async () => {
    const f = fixture()
    expect(await makeBackend({ client: f.client, model: 'fixture-model' }).create('/workspace')).toBe(sessionId)
    expect(f.actions[0]).toMatchObject({ type: 'agent.create', parameters: { harness: 'codex', workspace: '/workspace', model: 'fixture-model' }, fence: { snapshot_id: snapshot.id } })
    const loaded = await makeBackend({ client: f.client }).load(sessionId, '/workspace')
    expect(f.isClosed()).toBe(true)
    expect(loaded).toEqual([{ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Old output' } }])
  })

  it('waits for live daemon startup readiness before returning the session', async () => {
    const f = fixture()
    f.setAgent({ state: 'starting', harness_state: 'indeterminate' })
    let created = false
    const result = makeBackend({ client: f.client }).create('/workspace').then((session) => { created = true; return session })
    await f.startupSubscribed
    expect(created).toBe(false)
    f.setAgent({ state: 'running', harness_state: 'idle' })
    f.publishAgent()
    expect(await result).toBe(sessionId)
    expect(f.isClosed()).toBe(true)
    expect(f.actions).toHaveLength(1)
  })

  it('requires the same workspace, an idle current agent, and the supported normalizer', async () => {
    const f = fixture()
    const backend = makeBackend({ client: f.client })
    await expect(backend.load(sessionId, '/elsewhere')).rejects.toThrow('cwd does not match')
    f.setAgent({ harness_state: 'working' })
    await expect(backend.load(sessionId, '/workspace')).rejects.toThrow('idle')
    f.setAgent({ harness_state: 'idle', driver: 'omp' })
    await expect(backend.load(sessionId, '/workspace')).rejects.toThrow('Codex native transcript normalizer')
  })

  it('streams suffixes/tools once and completes only the exact new native turn', async () => {
    const f = fixture()
    const updates: BridgeUpdate[] = []
    const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', (update) => updates.push(update), async () => 'allow', new AbortController().signal)
    await f.submitted
    f.conversation([event(4, 'task_started', 'turn-live'), entry(5, 'Hello')])
    // Replacement snapshots replay the baseline and unchanged entries.
    f.conversation([event(1, 'task_started', 'old'), entry(2, 'Old output'), event(4, 'task_started', 'turn-live'), entry(5, 'Hello'), entry(5, 'Hello world', 2)], true)
    f.conversation([{ ...entry(6, ''), type: 'tool_call', body: { call_id: 'call/1', name: 'shell', arguments: { command: 'pwd' } } }])
    f.conversation([{ ...entry(7, ''), role: 'tool', type: 'tool_result', body: { call_id: 'call/1', content: '/workspace', media_type: 'text/plain', status: 'success' } }, event(8, 'task_complete', 'foreign')])
    // final=true on assistant content and another turn's completion are not end_turn.
    expect(f.isClosed()).toBe(false)
    f.conversation([event(9, 'task_complete', 'turn-live')])
    expect(await result).toBe('end_turn')
    expect(updates).toEqual([
      { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Hello' } },
      { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: ' world' } },
      { sessionUpdate: 'tool_call', toolCallId: 'call/1', title: 'shell', status: 'in_progress', rawInput: { command: 'pwd' } },
      { sessionUpdate: 'tool_call_update', toolCallId: 'call/1', status: 'completed', rawOutput: '/workspace' },
    ])
    expect(f.isClosed()).toBe(true)
    expect(f.actions.filter((action) => action.type === 'message.send')).toHaveLength(1)
  })

  it.each(['allow', 'reject'] as const)('round-trips %s through prompt.respond with the real episode/fence', async (answer) => {
    const f = fixture()
    let permissions = 0
    let responded!: () => void
    const response = new Promise<void>((resolve) => { responded = resolve })
    const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', () => {}, async (request) => {
      permissions++
      expect(request.toolCall.toolCallId).toBe('call/permission')
      expect(request.options.map((option) => option.kind)).toEqual(['allow_once', 'reject_once'])
      queueMicrotask(responded)
      return answer
    }, new AbortController().signal)
    await f.submitted
    f.conversation([event(4, 'task_started', 'turn-live')])
    const attention = { id: 'attention/permission', revision: 'revision/prompt', kind: 'attention' as const, updated_at: '2026-10-28T00:00:00Z', attention_kind: 'harness-prompt', actions: ['prompt.respond' as const], detail: '', person_id: 'person/test', priority: 'normal' as const, requested_at: '2026-10-28T00:00:00Z', source_id: id, state: 'open' as const, title: 'Run shell', action_parameters: { 'prompt.respond': { target_id: id, episode: 'episode/1', call_id: 'call/permission', answers: ['allow', 'deny'] } } }
    const change: CollectionFrame = { kind: 'changes', id: 'attention', collection: 'attention', upserts: [attention], removes: [], order: [attention.id], has_more: false, snapshot }
    f.frame(change)
    f.frame(change)
    await response
    f.conversation([event(5, 'task_complete', 'turn-live')])
    expect(await result).toBe('end_turn')
    expect(permissions).toBe(1)
    expect(f.actions.find((action) => action.type === 'prompt.respond')).toMatchObject({ parameters: { target_id: id, episode: 'episode/1', answer: answer === 'allow' ? 'allow' : 'deny' }, fence: { snapshot_id: snapshot.id, subject_revisions: { 'attention/permission': 'revision/prompt' } } })
  })

  it('interrupts only on explicit cancel and uses the current runtime incarnation', async () => {
    const f = fixture()
    const backend = makeBackend({ client: f.client })
    const result = backend.prompt(sessionId, 'Work', () => {}, async () => 'reject', new AbortController().signal)
    await f.submitted
    await backend.cancel(sessionId)
    expect(await result).toBe('cancelled')
    expect(f.actions.find((action) => action.type === 'runtime.signal')).toMatchObject({ parameters: { target_id: 'session/test', signal: 'interrupt' }, fence: { runtime_incarnation: 'incarnation/test', subject_revisions: { [id]: 'revision/agent' } } })
    expect(f.isClosed()).toBe(true)
  })

  it('detaches an aborted subscription without interrupting the daemon turn', async () => {
    const f = fixture()
    const controller = new AbortController()
    const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', () => {}, async () => 'reject', controller.signal)
    await f.submitted
    controller.abort()
    expect(await result).toBe('cancelled')
    expect(f.actions.some((action) => action.type === 'runtime.signal')).toBe(false)
    expect(f.isClosed()).toBe(true)
  })

  it.each(['competing', 'failure', 'rewrite', 'resync'] as const)('fails explicitly on %s without fabricating completion', async (scenario) => {
    const f = fixture()
    const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', () => {}, async () => 'reject', new AbortController().signal)
    await f.submitted
    f.conversation([event(4, 'task_started', 'turn-live'), entry(5, 'Hello')])
    if (scenario === 'competing') f.conversation([event(6, 'task_started', 'turn-other')])
    else if (scenario === 'failure') f.conversation([event(6, 'task_complete', 'turn-live', 'provider failed')])
    else if (scenario === 'rewrite') f.conversation([entry(5, 'Replacement', 2)])
    else f.frame({ kind: 'resync', id: 'conversation', message: 'history changed' })
    await expect(result).rejects.toThrow()
    expect(f.isClosed()).toBe(true)
  })
})

it('rejects a native completion without its post-baseline task_started identity', async () => {
  const f = fixture()
  const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', () => {}, async () => 'reject', new AbortController().signal)
  await f.submitted
  f.conversation([event(4, 'task_complete', 'unbound-turn')])
  await expect(result).rejects.toThrow('matching native task_started/turn_id')
  expect(f.isClosed()).toBe(true)
})

it('refuses owner-continuation content rather than silently truncating output', async () => {
  const f = fixture()
  const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', () => {}, async () => 'reject', new AbortController().signal)
  await f.submitted
  f.conversation([{ ...entry(4, 'truncated'), body: { media_type: 'text/plain', text: 'truncated', blocks: [{ id: 'block/large', kind: 'text', source_type: 'content', payload: {}, continuation: { ref: 'owner/content', media_type: 'text/plain', size: 100000 } }] } }])
  await expect(result).rejects.toThrow('without truncation')
  expect(f.isClosed()).toBe(true)
})

it('refuses rich message blocks instead of silently omitting output', async () => {
  const f = fixture()
  const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', () => {}, async () => 'reject', new AbortController().signal)
  await f.submitted
  f.conversation([{ ...entry(4, 'image fallback'), body: { media_type: 'text/plain', text: 'image fallback', blocks: [{ id: 'block/image', kind: 'image', source_type: 'content', payload: {} }] } }])
  await expect(result).rejects.toThrow('non-text blocks')
  expect(f.isClosed()).toBe(true)
})

it('refuses an explicit history redaction marker', async () => {
  const f = fixture()
  const result = makeBackend({ client: f.client }).prompt(sessionId, 'Work', () => {}, async () => 'reject', new AbortController().signal)
  await f.submitted
  f.conversation([{ ...entry(4, ''), type: 'redaction', body: { reason: 'hidden', withheld_bytes: 10 } }])
  await expect(result).rejects.toThrow('truncated or redacted history')
  expect(f.isClosed()).toBe(true)
})
