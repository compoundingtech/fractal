import * as Native from '@fractal/client/schema/read'
import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { gatewayThreads } from './threadSource.ts'

const base = fixture.capabilities
const id = 'thread/thread-one'
const agent = 'agent/workshop/atlas'
const summary = {
  id, agent_id: agent, title: 'PR #42', title_revision: 'claim/title', revision: 'claim/latest', state: 'open',
}

it('issues no thread reads or edits when the daemon has not advertised the feature', async () => {
  const fetchImpl = vi.fn(async (_input: RequestInfo | URL) => Response.json(base))
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const list = source.list({ agent_id: agent })
  const stop = registry.mount(list)
  try {
    await vi.waitFor(() => expect(fetchImpl).toHaveBeenCalledTimes(1))
    expect(registry.get(list)).toEqual({ _tag: 'Waiting' })
    expect(registry.get(source.access)).toMatchObject({ read: false, actions: new Set() })
    expect(String(fetchImpl.mock.calls[0]?.[0])).toContain('/capabilities')
  } finally {
    stop()
    registry.dispose()
  }
})

it.each(['missing', 'ungranted'] as const)('does not infer current metadata when the summary capability is %s', async state => {
  const fetchImpl = vi.fn(async () => Response.json({
    ...base, value: { ...base.value, capabilities: [
      ...base.value.capabilities,
      { id: 'threads-v1', version: 2, state: 'granted' },
      { id: 'thread.start', version: 0, state: 'granted' },
      ...(state === 'missing' ? [] : [{ id: 'thread-summary-feed', version: 1, state: 'ungranted' }]),
    ] },
  }))
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl }), registry)
  const list = source.list({ agent_id: agent })
  const stop = registry.mount(list)
  try {
    await vi.waitFor(() => expect(registry.get(source.access)).toMatchObject({ read: false, upgradeRequired: state === 'missing', actions: new Set() }))
    expect(registry.get(list)).toEqual({ _tag: 'Waiting' })
    expect(fetchImpl).toHaveBeenCalledTimes(1)
  } finally { stop(); registry.dispose() }
})

it('reads the exact agent, paginates by opaque cursor and sends the observed metadata fence', async () => {
  const requests: Array<{ readonly url: URL; readonly body: unknown }> = []
  const fetchImpl = async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input))
    const body: unknown = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined
    requests.push({ url, body })
    if (url.pathname.endsWith('/capabilities')) return Response.json({
      ...base,
      value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.close', version: 0, state: 'granted' },
      ] },
    })
    if (url.pathname.endsWith('/threads')) return Response.json({ ...base, value: { items: [summary], next_cursor: 'older/+chunk' } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [], next_cursor: null } })
    if (url.pathname.endsWith('/actions')) return Response.json({
      ...base,
      value: {
        kind: 'action-result', action_id: (body as { id: string }).id, operation_id: 'operation/one', status: 'completed',
        affected_ids: [id], metadata_claim_id: 'claim/closed', snapshot_id: base.snapshot.id,
      },
    })
    return Response.json({ ...base, value: summary })
  }
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const list = source.list({ agent_id: agent, cursor: 'older/+chunk' })
  const detail = source.detail(id)
  const timeline = source.timeline(id, 'timeline/older')
  const stops = [registry.mount(list), registry.mount(detail), registry.mount(timeline)]
  try {
    await vi.waitFor(() => {
      expect(registry.get(list)._tag).toBe('Observed')
      expect(registry.get(detail)._tag).toBe('Observed')
      expect(registry.get(timeline)._tag).toBe('Observed')
    })
    const observation = registry.get(detail)
    if (observation._tag !== 'Observed') throw new Error('expected thread observation')
    expect(await source.changeState(observation.value, 'closed')).toBeUndefined()
    expect(requests.find(({ url }) => url.pathname.endsWith('/threads'))?.url.searchParams.get('agent_id')).toBe(agent)
    expect(requests.find(({ url }) => url.pathname.endsWith('/threads'))?.url.searchParams.get('cursor')).toBe('older/+chunk')
    expect(requests.find(({ url }) => url.pathname.endsWith('/timeline'))?.url.pathname).toBe('/v1/client/threads/thread-one/timeline')
    expect(requests.find(({ url }) => url.pathname.endsWith('/timeline'))?.url.searchParams.get('cursor')).toBe('timeline/older')
    expect(requests.find(({ url }) => url.pathname.endsWith('/actions'))?.body).toMatchObject({
      type: 'thread.close',
      fence: { snapshot_id: base.snapshot.id, subject_revisions: { [id]: summary.revision } },
      parameters: { target_id: id },
    })
  } finally {
    for (const stop of stops) stop()
    registry.dispose()
  }
})

it('starts with the exact ask episode and retries the same first message after an uncertain response', async () => {
  const bodies: Array<Record<string, unknown>> = []
  const fetchImpl = async (input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input).endsWith('/capabilities')) return Response.json({
      ...base,
      value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.start', version: 0, state: 'granted' },
      ] },
    })
    const body = JSON.parse(String(init?.body))
    bodies.push(body)
    if (bodies.length === 1) throw new TypeError('connection interrupted')
    if (bodies.length === 3) return Response.json({
      api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
      code: 'forbidden', message: 'Thread key is temporarily unavailable', retryable: false, details: {},
    }, { status: 403 })
    return Response.json({ ...base, value: {
      kind: 'action-result', action_id: bodies.length === 2 ? 'action/unrelated' : body.id, operation_id: 'operation/start', status: 'completed',
      affected_ids: [body.parameters.target_id, 'message/first'], snapshot_id: base.snapshot.id,
    } })
  }
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const stop = registry.mount(source.access)
  const input = { agent_id: agent, origin: { source_id: 'step-run/review', episode: 'claim/ask' }, title: 'PR #42', content: 'Can we run the worker test?' }
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions.has('thread.start')).toBe(true))
    expect((await source.start('ask/review', input))._tag).toBe('Refused')
    expect((await source.start('ask/review', { ...input, content: 'Different message' }))._tag).toBe('Refused')
    expect((await source.start('ask/review', input))._tag).toBe('Refused')
    expect((await source.start('ask/review', input))._tag).toBe('Refused')
    expect((await source.start('ask/review', input))._tag).toBe('Sent')
    expect(bodies).toHaveLength(4)
    for (const retry of bodies.slice(1)) expect(retry).toEqual(bodies[0])
    expect(bodies[1]).toMatchObject({
      type: 'thread.start',
      parameters: { ...input, target_id: expect.stringMatching(/^thread\//) },
    })
    expect(bodies[1]?.parameters).not.toHaveProperty('in_reply_to')
  } finally {
    stop()
    registry.dispose()
  }
})

it('offers metadata edits but omits the unsigned composer on paired web transport', async () => {
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async () => Response.json({
    ...base,
    value: { ...base.value, capabilities: [
      ...base.value.capabilities,
      { id: 'threads-v1', version: 2, state: 'granted' },
      { id: 'thread-summary-feed', version: 1, state: 'granted' },
      ...['thread.start', 'thread.reply', 'thread.close'].map(id => ({ id, version: 0, state: 'granted' })),
    ] },
  }) })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const stop = registry.mount(source.access)
  try {
    await vi.waitFor(() => expect(registry.get(source.access)).toMatchObject({
      read: true,
      actions: new Set(['thread.close']),
    }))
  } finally {
    stop()
    registry.dispose()
  }
})

it('observes exact summary receipt updates without a write grant or a follow-up HTTP read', async () => {
  const fetchImpl = vi.fn(async (input: RequestInfo | URL) => {
    if (String(input).endsWith('/capabilities')) return Response.json({
      ...base,
      value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
      ] },
    })
    return Response.json({ ...base, value: { ...summary, unread: true } })
  })
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const unsubscribe = vi.fn()
  const detail = vi.fn<NonNullable<Parameters<typeof gatewayThreads>[2]>['detail']>()
  let observe: Parameters<ReturnType<typeof detail>>[0] | undefined
  detail.mockImplementation(() => receive => {
    observe = receive
    return unsubscribe
  })
  const source = gatewayThreads(client, registry, { detail, summaries: () => undefined, timeline: () => () => () => {} })
  const view = source.detail(id)
  const stop = registry.mount(view)
  try {
    await vi.waitFor(() => expect(registry.get(view)).toMatchObject({ _tag: 'Observed', value: { summary: { unread: true } } }))
    const before = registry.get(view)
    if (before._tag !== 'Observed') throw new Error('expected exact thread summary')
    expect(detail).toHaveBeenCalledWith(id)
    expect(registry.get(source.access).actions.has('thread.read')).toBe(false)
    observe?.({ _tag: 'Observed', value: { items: [{ ...before.value.summary, unread: false }], hasMore: false, snapshotId: 'snapshot/socket' } })
    expect(registry.get(view)).toMatchObject({
      _tag: 'Observed', value: { summary: { unread: false }, snapshotId: 'snapshot/socket' },
    })
    expect(fetchImpl).toHaveBeenCalledTimes(2)
  } finally {
    stop()
    registry.dispose()
    expect(unsubscribe).toHaveBeenCalledOnce()
  }
})


it('sends the same typed actions for approved browsers and blocks writes if the fresh grant changes', async () => {
  const writes: Array<{ readonly type: string; readonly id: string; readonly parameters: Record<string, unknown> }> = []
  let browserGrant = true
  let replyGrant = true
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    if (String(input).endsWith('/capabilities')) return Response.json({
      ...base, value: { ...base.value, capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread-browser-send', version: 1, state: browserGrant ? 'granted' : 'ungranted' },
        { id: 'thread.start', version: 0, state: 'granted' },
        { id: 'thread.reply', version: 0, state: replyGrant ? 'granted' : 'ungranted' },
      ] },
    })
    const action = JSON.parse(String(init?.body)) as typeof writes[number]
    writes.push(action)
    return Response.json({ ...base, value: {
      kind: 'action-result', status: 'completed', action_id: action.id, operation_id: 'operation/browser-send',
      affected_ids: [action.parameters.target_id, 'message/browser-send'], snapshot_id: base.snapshot.id,
    } })
  } })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const stop = registry.mount(source.access)
  const start = { agent_id: agent, origin: { source_id: 'step-run/review', episode: 'claim/ask' }, title: 'PR #42', content: 'Can we run the worker test?' }
  const reply = { target_id: id, content: 'Please include cancellation.', thread_quote_id: 'message/worker-tests' }
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions).toEqual(new Set(['thread.start', 'thread.reply'])))
    expect((await source.start('ask/review', start))._tag).toBe('Sent')
    expect((await source.reply(reply))._tag).toBe('Sent')
    expect(writes).toMatchObject([
      { type: 'thread.start', parameters: { ...start, target_id: expect.stringMatching(/^thread\//) } },
      { type: 'thread.reply', parameters: reply },
    ])
    for (const write of writes) expect(write.parameters).not.toHaveProperty('signature')
    replyGrant = false
    expect((await source.reply(reply))._tag).toBe('Refused')
    replyGrant = true
    browserGrant = false
    expect((await source.reply(reply))._tag).toBe('Refused')
    expect(writes).toHaveLength(2)
  } finally {
    stop()
    registry.dispose()
  }
})

it('retains the exact rename intent across malformed receipt, denial and a changed draft', async () => {
  const writes: Array<Record<string, unknown>> = []
  let release!: () => void
  const held = new Promise<void>(resolve => { release = resolve })
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    if (String(input).endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, capabilities: [
      ...base.value.capabilities,
      ...['threads-v1', 'thread-summary-feed', 'thread.rename'].map(id => ({ id, version: id === 'threads-v1' ? 2 : 1, state: 'granted' })),
    ] } })
    const action = JSON.parse(String(init?.body)) as Record<string, unknown>
    writes.push(action)
    if (writes.length === 1) await held
    if (writes.length === 2) return Response.json({ api_version: base.api_version, error_version: 'st3.client.error.v0', request_id: action.id,
      code: 'forbidden', message: 'grant expired', retryable: false, details: {} }, { status: 403 })
    return Response.json({ ...base, value: { kind: 'action-result', action_id: action.id, status: 'completed',
      operation_id: 'operation/rename', snapshot_id: base.snapshot.id, affected_ids: [id], metadata_claim_id: writes.length === 1 ? '' : 'claim/new-title' } })
  } })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const stop = registry.mount(source.access)
  // Keep the observed old title revision: a retry cannot invent a new old/new rename pair.
  const observation = { summary: Native.decodeUnknownSync(Native.ThreadSummary)(summary), snapshotId: base.snapshot.id }
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions.has('thread.rename')).toBe(true))
    const initial = source.rename(observation, 'New title', 'claim/title')
    await vi.waitFor(() => expect(writes).toHaveLength(1))
    expect(await source.rename(observation, 'Different title', 'claim/title')).toBeDefined()
    release()
    expect(await initial).toBeDefined()
    expect(await source.rename(observation, 'Different title', 'claim/title')).toBeDefined()
    expect(writes).toHaveLength(1)
    if (!source.pendingChange || !source.retryChange) throw new Error('Expected retained metadata actions')
    expect(registry.get(source.pendingChange(id))).toBe('rename')
    expect(await source.retryChange(id)).toBeDefined()
    expect(await source.retryChange(id)).toBeUndefined()
    expect(registry.get(source.pendingChange(id))).toBeUndefined()
    expect(writes).toHaveLength(3)
    expect(writes[1]).toEqual(writes[0])
    expect(writes[2]).toEqual(writes[0])
  } finally { stop(); registry.dispose() }
})


it('requires the current Inbox origin version before reading or sending', async () => {
  const fetchImpl = vi.fn(async () => Response.json({
    ...base, value: { ...base.value, transport: 'local-web', capabilities: [
      ...base.value.capabilities,
      { id: 'threads-v1', version: 1, state: 'granted' },
      { id: 'thread-summary-feed', version: 1, state: 'granted' },
      { id: 'thread.start', version: 0, state: 'granted' },
    ] },
  }))
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl }), registry)
  const stop = registry.mount(source.list({ agent_id: agent }))
  try {
    await vi.waitFor(() => expect(registry.get(source.access)).toMatchObject({ read: false, inboxOrigins: false, upgradeRequired: true }))
    expect(await source.start('notice-draft', { agent_id: agent, origin: { source_id: 'person-work/notice', episode: 'opaque-episode' }, title: 'Notice', content: 'Please clarify' })).toMatchObject({ _tag: 'Refused' })
    expect(fetchImpl).toHaveBeenCalledTimes(1)
  } finally { stop(); registry.dispose() }
})

it('discovers original first sends by source across changed episodes and retries only their bound intent', async () => {
  const writes: Array<{ id: string; idempotency_key: string; parameters: { target_id: string; origin: { source_id: string; episode: string } } }> = []
  let release!: () => void
  const held = new Promise<void>(resolve => { release = resolve })
  const attempts = new Map<string, number>()
  let actor = base.value.session_actor
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    if (String(input).endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, session_actor: actor, transport: 'local-web', capabilities: [
      ...base.value.capabilities,
      ...['threads-v1', 'thread-summary-feed', 'thread.start'].map(id => ({ id, version: 2, state: 'granted' })),
    ] } })
    const action = JSON.parse(String(init?.body)) as typeof writes[number]
    writes.push(action)
    const count = (attempts.get(action.idempotency_key) ?? 0) + 1
    attempts.set(action.idempotency_key, count)
    if (writes.length === 1) await held
    if (count === 1) throw new TypeError('The thread committed but its receipt was lost')
    return Response.json({ ...base, value: { kind: 'action-result', status: 'completed', action_id: action.id,
      operation_id: `operation/${action.idempotency_key}`, snapshot_id: base.snapshot.id,
      affected_ids: [action.parameters.target_id, 'message/first'],
    } })
  } })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const pendingStarts = source.pendingStarts
  const retryStart = source.retryStart
  if (!pendingStarts || !retryStart) throw new Error('Expected retained first-send support')
  const sourceId = 'custom/garden/review/origin'
  const retained = pendingStarts(sourceId)
  const stopAccess = registry.mount(source.access)
  let stop = registry.mount(retained)
  const original = { agent_id: agent, origin: { source_id: sourceId, episode: 'claim/old' }, title: 'Original title', content: 'Original question' }
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions.has('thread.start')).toBe(true))
    expect(registry.get(retained)).toEqual([])
    registry.set(source.draft('draft/old'), { content: original.content })
    const first = source.start('draft/old', original)
    await vi.waitFor(() => expect(writes).toHaveLength(1))
    expect(registry.get(retained)).toMatchObject([{ draftKey: 'draft/old', state: 'sending', parameters: original }])
    expect((await source.start('draft/old', { ...original, agent_id: 'agent/other', title: 'Changed title' }))._tag).toBe('Refused')
    expect(writes).toHaveLength(1)
    release()
    expect((await first)._tag).toBe('Refused')
    stop()
    stop = registry.mount(pendingStarts(sourceId))
    expect(registry.get(retained)).toMatchObject([{ draftKey: 'draft/old', state: 'unconfirmed', parameters: original }])
    // Another signed-in actor cannot discover or rebind this person's pending content.
    actor = 'person/blair'
    registry.refresh(source.access)
    await vi.waitFor(() => expect(registry.get(source.access).actor).toBe(actor))
    expect(registry.get(retained)).toEqual([])
    expect(registry.get(pendingStarts())).toEqual([])
    expect((await retryStart('draft/old'))._tag).toBe('Refused')
    expect(writes).toHaveLength(1)
    actor = base.value.session_actor
    registry.refresh(source.access)
    await vi.waitFor(() => expect(registry.get(source.access).actor).toBe(actor))
    expect(registry.get(pendingStarts())).toMatchObject([{ draftKey: 'draft/old', state: 'unconfirmed' }])
    // A later episode has its own attempt; neither needs the current row's episode to be found.
    registry.set(source.draft('draft/new'), { content: original.content })
    expect((await source.start('draft/new', { ...original, origin: { source_id: sourceId, episode: 'claim/new' } }))._tag).toBe('Refused')
    expect(registry.get(retained).map(start => start.draftKey)).toEqual(['draft/old', 'draft/new'])
    expect(registry.get(pendingStarts()).map(start => start.draftKey)).toEqual(['draft/old', 'draft/new'])
    expect(registry.get(pendingStarts('custom/another/source'))).toEqual([])
    const old = registry.get(retained)[0]!
    // The preview is detached from the private prepared request, including its nested origin.
    Reflect.set(old.parameters.origin, 'episode', 'changed-preview')
    const newerDraft = { content: original.content }
    registry.set(source.draft(old.draftKey), newerDraft)
    const retry = await retryStart(old.draftKey)
    expect(retry).toEqual({ _tag: 'Sent', threadId: old.targetId })
    expect(writes[2]).toEqual(writes[0])
    expect(registry.get(source.draft(old.draftKey))).toBe(newerDraft)
    expect(registry.get(retained).map(start => start.draftKey)).toEqual(['draft/new'])
    expect((await retryStart('draft/new'))._tag).toBe('Sent')
    expect(writes[3]).toEqual(writes[1])
    expect(registry.get(source.draft('draft/new'))).toEqual({ content: '' })
    expect(registry.get(pendingStarts())).toEqual([])
  } finally { release(); stop(); stopAccess(); registry.dispose() }
})
