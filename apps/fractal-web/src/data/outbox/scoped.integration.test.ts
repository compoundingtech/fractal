import 'fake-indexeddb/auto'
import { createOutboxRecord, FractalClient, outboxDeploymentId, prepareMessageSendPacket, prepareOutboxRecord, type EnvelopeOf } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { IDBFactory } from 'fake-indexeddb'
import { afterEach, describe, expect, it } from 'vitest'
import fixture from '../subjectReadPort.gateway.fixtures.json'
import { gatewayMessageSend } from '../attachmentPort.ts'
import { createScopedChatOutbox, scopeFromCapabilities, type ScopedChatOutbox } from './scoped.ts'

const base = { ...fixture.capabilities, api_version: 'st3.client.v0' as const, snapshot: Native.decodeUnknownSync(Native.Snapshot)(fixture.capabilities.snapshot), value: { ...Native.decodeUnknownSync(Native.Capabilities)(fixture.capabilities.value), issuer: 'person/example', issuer_key: 'issuer-key/a' } }
const owners: ScopedChatOutbox[] = []
const setup = () => {
  let response: EnvelopeOf<typeof Native.Capabilities.Type> = base
  let fail = false
  const calls: string[] = []
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async (input) => {
    calls.push(new URL(String(input)).pathname)
    if (fail) throw new TypeError('Offline')
    return Response.json(response)
  } })
  const owner = createScopedChatOutbox(client, { indexedDB: new IDBFactory() })
  owners.push(owner)
  return { client, owner, calls, respond: (next: EnvelopeOf<typeof Native.Capabilities.Type>) => { response = next }, offline: () => { fail = true } }
}
afterEach(() => { for (const owner of owners.splice(0)) owner.close() })

describe('verified browser outbox scope', () => {
  it.each(['blocked', 'disabled'] as const)('reports %s opening as unavailable, never a corrupt recovery entry', async failure => {
    const { client } = setup()
    const factory = new IDBFactory()
    const owner = createScopedChatOutbox(client, { indexedDB: {
      open: (name, version) => {
        if (failure === 'disabled') throw new DOMException('Unavailable', 'InvalidStateError')
        const request = factory.open(name, version)
        queueMicrotask(() => request.dispatchEvent(new Event('blocked')))
        return request
      },
    } })
    owners.push(owner)
    owner.observe(base)
    await expect(owner.recovery.list()).rejects.toMatchObject({ _tag: 'OutboxStorageError', code: 'disabled' })
  })

  it('uses the cached generated issuer pair without another discovery and persists a protected record', async () => {
    const { owner, calls } = setup()
    owner.observe(base)
    const repo = await owner.repository()
    expect(calls).toEqual([])
    expect(owner.protection()).toBe('durable')
    expect(repo?.scope).toEqual({ principalId: base.value.session_actor, deploymentId: outboxDeploymentId(base.value.issuer, base.value.issuer_key), memberId: base.snapshot.host_id })
    const record = createOutboxRecord('attempt/scope', { text: 'saved', attachments: [] })
    expect(await repo?.commit(record)).toBe(true)
    expect(await repo?.read(record.id)).toEqual(record)
  })

  it('discovers identity exactly once before first acceptance when no scope was observed', async () => {
    const { owner, calls } = setup()
    const repo = await owner.repository()
    expect(calls).toEqual(['/v1/client/capabilities'])
    await repo?.commit(createOutboxRecord('attempt/first', { text: 'first', attachments: [] }))
    expect(await repo?.list()).toHaveLength(1)
    expect(calls).toHaveLength(1)
  })

  it.each(['absent', 'issuer-only', 'key-only'] as const)('selects explicit unprotected mode for a successful %s issuer answer', async kind => {
    const { owner } = setup()
    const { issuer: _issuer, issuer_key: _key, ...value } = base.value
    owner.observe({ ...base, value: { ...value, ...(kind === 'issuer-only' ? { issuer: base.value.issuer } : {}), ...(kind === 'key-only' ? { issuer_key: base.value.issuer_key } : {}) } })
    expect(await owner.repository()).toBeUndefined()
    expect(owner.protection()).toBe('unprotected')
  })

  it('separates different issuer pairs without making another scope\'s records readable', async () => {
    const { owner } = setup()
    owner.observe(base)
    const first = await owner.repository()
    await first?.commit(createOutboxRecord('attempt/original', { text: 'original', attachments: [] }))
    owner.observe({ ...base, value: { ...base.value, issuer_key: 'issuer-key/b' } })
    const second = await owner.repository()
    expect(second?.scope).not.toEqual(first?.scope)
    expect(await second?.read('attempt/original')).toBeUndefined()
    expect(await second?.list()).toEqual([])
  })

  it('refuses failed initial identity discovery without accepting or doing send work', async () => {
    const { owner, client, calls, offline } = setup()
    offline()
    let snapshots = 0
    const send = gatewayMessageSend(client, async () => { snapshots++; return { _tag: 'Success', value: base.snapshot.id } }, owner)
    expect(await send({ _tag: 'Send', api_version: 'st3.client.v0', type: 'message.send', id: 'action/identity', parameters: { to: 'agent/example', content: 'draft', tags: [], attachments: [] } }))
      .toMatchObject({ _tag: 'Refused', reason: 'identity-unavailable' })
    expect(calls).toEqual(['/v1/client/capabilities'])
    expect(snapshots).toBe(0)
  })

  it.each(['missing', 'partial', 'different'] as const)('keeps an existing record unchanged and visible when current identity is %s', async kind => {
    const { owner, client, respond, calls } = setup()
    owner.observe(base)
    const repo = await owner.repository()
    if (repo === undefined) throw new TypeError('Expected protected repository')
    const initial = createOutboxRecord('attempt/retained', { text: 'retained', attachments: [] })
    const packet = prepareMessageSendPacket({ id: 'action/retained', idempotency_key: 'retained-packet-key', fence: { snapshot_id: base.snapshot.id, subject_revisions: {} }, parameters: { to: 'agent/example', content: 'retained', tags: [], attachments: [] } })
    const ready = prepareOutboxRecord(initial, packet, ['message/retained'])
    if (ready === undefined) throw new TypeError('Expected ready record')
    await repo.commit(initial)
    await repo.compareAndSwap(0, ready)
    const dispatched = await repo.acquireDispatchPermit(initial.id, ready.revision)
    const { issuer: _issuer, issuer_key: _key, ...value } = base.value
    const changed = { ...base, value: { ...value, ...(kind === 'different' ? { issuer: 'person/other', issuer_key: base.value.issuer_key } : kind === 'partial' ? { issuer: base.value.issuer } : {}) } }
    respond(changed)
    const items = await owner.recovery.list()
    expect(items).toContainEqual({ id: initial.id, status: 'Identity changed; delivery unknown. Return to the original account and deployment.', actions: ['discard'] })
    expect(await repo.read(initial.id)).toEqual(dispatched)
    const send = gatewayMessageSend(client, async () => { throw new Error('No snapshot on mismatch') }, { repository: async () => repo })
    expect(await send({ _tag: 'Resend', idempotencyKey: initial.id, api_version: 'st3.client.v0', type: 'message.send', id: packet.id, parameters: { ...packet.parameters, tags: packet.parameters.tags ?? [], attachments: packet.parameters.attachments ?? [] } })).toMatchObject({ _tag: 'Refused', reason: 'ungranted' })
    expect(await repo.read(initial.id)).toEqual(dispatched)
    expect(calls.every(path => path === '/v1/client/capabilities')).toBe(true)
    expect(scopeFromCapabilities(changed)).not.toEqual(repo.scope)
  })
})
