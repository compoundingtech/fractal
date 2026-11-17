import 'fake-indexeddb/auto'
import { createHash } from 'node:crypto'
import { FractalClient, outboxDeploymentId } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { Effect } from 'effect'
import { IDBFactory, IDBObjectStore } from 'fake-indexeddb'
import { afterEach, describe, expect, it, vi } from 'vitest'
import certainty from '../../../../docs/st3/client-v0/fixtures/thread-start-refusals.json'
import fixture from './subjectReadPort.gateway.fixtures.json'
import { gatewayAttachments, gatewayMessageSend, type ChatOutbox } from './attachmentPort.ts'
import { attemptsStore, openIndexedDbOutbox, type IndexedDbOutbox } from './outbox/indexedDb.ts'
import type { AttachmentSendRequest } from './source.ts'

const base = { ...fixture.capabilities, value: { ...fixture.capabilities.value, issuer: 'person/example', issuer_key: 'issuer-key/a' } }
const scope = {
  principalId: base.value.session_actor,
  deploymentId: outboxDeploymentId(base.value.issuer, base.value.issuer_key),
  memberId: base.snapshot.host_id,
}
const key = 'chat-durable-key'
const request: AttachmentSendRequest = {
  _tag: 'Send', api_version: 'st3.client.v0', type: 'message.send', id: 'action/chat-durable',
  parameters: { to: 'agent/atlas', content: 'Exact café message', tags: ['original'], attachments: [] },
}
const snapshot = async () => ({ _tag: 'Success' as const, value: base.snapshot.id })
const connections: IndexedDbOutbox[] = []
const open = async (factory = new IDBFactory()) => {
  const repo = await Effect.runPromise(openIndexedDbOutbox({ scope, indexedDB: factory }))
  connections.push(repo)
  return repo
}
const receipt = (body: string) => {
  const action = Native.decodeUnknownSync(Native.ActionRequest)(JSON.parse(body))
  return { ...base, value: {
    kind: 'action-result', status: 'completed', action_id: action.id,
    operation_id: 'operation/chat', snapshot_id: base.snapshot.id,
    affected_ids: [`message/${createHash('sha256').update(action.idempotency_key).digest('hex').slice(0, 16)}`],
  } }
}
const transport = (post: (body: string) => Response | Promise<Response> = body => Response.json(receipt(body))) => {
  const calls: { path: string; body: RequestInit['body'] }[] = []
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async (input, init) => {
    const path = new URL(String(input)).pathname
    calls.push({ path, body: init?.body })
    if (path.endsWith('/capabilities')) return Response.json(base)
    if (path.endsWith('/blobs')) return Response.json({ ...base, value: {
      blob: `blob/${'b'.repeat(64)}`, sha256: 'b'.repeat(64), size: 3, media_type: 'image/png',
    } })
    return post(String(init?.body))
  } })
  return { client, calls }
}
const bind = (client: FractalClient, repo: IndexedDbOutbox) => gatewayMessageSend(client, snapshot, { repository: async () => repo })
afterEach(() => {
  vi.restoreAllMocks()
  for (const repo of connections.splice(0)) repo.close()
})

describe('chat durable gateway delivery', () => {
  it('retains a validated receipt after another tab recovers its in-flight marker as unknown', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const started = Promise.withResolvers<void>()
    const held = Promise.withResolvers<Response>()
    const { client, calls } = transport(() => { started.resolve(); return held.promise })
    const pending = bind(client, repo)(request, key)
    await started.promise
    const second = await open(factory)
    expect(await second.read(key)).toMatchObject({ state: 'unknown' })
    const body = calls.find(call => call.path.endsWith('/actions'))?.body
    held.resolve(Response.json(receipt(String(body))))
    expect(await pending).toMatchObject({ _tag: 'Success' })
    expect(await second.read(key)).toMatchObject({ state: 'delivered', evidence: { kind: 'receipt' } })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
  })

  it.each(['protected', 'unprotected'] as const)('acquires a text snapshot before capability discovery in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    const { client, calls } = transport()
    const send = gatewayMessageSend(client, async () => {
      expect(calls).toEqual([])
      if (repo !== undefined) expect(await repo.read(key)).toMatchObject({ phase: 'preparing' })
      return { _tag: 'Refused', reason: 'snapshot-unavailable', detail: 'Snapshot unavailable.' }
    }, { repository: async () => repo })
    expect(await send(request, key)).toMatchObject({ _tag: 'Refused', reason: 'snapshot-unavailable' })
    expect(calls).toEqual([])
  })

  it.each(['protected', 'unprotected'] as const)('acquires an image snapshot after capability discovery and upload in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    const { client, calls } = transport()
    const send = gatewayMessageSend(client, async () => {
      expect(calls.map(call => call.path)).toEqual(['/v1/client/capabilities', '/v1/client/blobs'])
      return snapshot()
    }, { repository: async () => repo })
    expect(await send({
      ...request, images: [{ bytes: new Uint8Array([1, 2, 3]), mediaType: 'image/png', name: 'original.png' }],
    }, key)).toMatchObject({ _tag: 'Success' })
    expect(calls.map(call => call.path)).toEqual(mode === 'protected'
      ? ['/v1/client/capabilities', '/v1/client/blobs', '/v1/client/capabilities', '/v1/client/actions', '/v1/client/capabilities']
      : ['/v1/client/capabilities', '/v1/client/blobs', '/v1/client/actions'])
  })

  it.each(['message', 'attachment'] as const)('makes zero requests before the %s acceptance transaction completes', async entry => {
    const repo = await open()
    const { client, calls } = transport()
    let requestSucceeded = false
    let committed = false
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, itemKey) {
      const added = add.call(this, value, itemKey)
      if (this.name === attemptsStore) {
        added.addEventListener('success', () => { requestSucceeded = true; expect(calls).toEqual([]) })
        this.transaction.addEventListener('complete', () => { expect(calls).toEqual([]); committed = true })
      }
      return added
    })
    const outbox: ChatOutbox = { repository: async () => repo }
    const send = entry === 'message' ? gatewayMessageSend(client, snapshot, outbox)
      : gatewayAttachments(client, snapshot, outbox).send
    const input: AttachmentSendRequest = entry === 'message' ? request : {
      ...request, images: [{ bytes: new Uint8Array([1, 2, 3]), mediaType: 'image/png', name: 'original.png' }],
    }
    expect(await send(input)).toMatchObject({ _tag: 'Success' })
    expect(requestSucceeded).toBe(true)
    expect(committed).toBe(true)
    expect(calls.map(call => call.path)).toEqual(entry === 'message'
      ? ['/v1/client/capabilities', '/v1/client/capabilities', '/v1/client/actions', '/v1/client/capabilities']
      : ['/v1/client/capabilities', '/v1/client/blobs', '/v1/client/capabilities', '/v1/client/actions', '/v1/client/capabilities'])
  })

  it('refuses an aborted acceptance transaction with zero capability, upload or action requests', async () => {
    const repo = await open()
    const { client, calls } = transport()
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, itemKey) {
      const added = add.call(this, value, itemKey)
      if (this.name === attemptsStore) added.addEventListener('success', () => this.transaction.abort())
      return added
    })
    const input: AttachmentSendRequest = { ...request, images: [{ bytes: new Uint8Array([1, 2, 3]), mediaType: 'image/png', name: 'original.png' }] }
    expect(await bind(client, repo)(input, key)).toMatchObject({ _tag: 'Refused', reason: 'storage' })
    expect(calls).toEqual([])
    expect(await repo.read(key)).toBeUndefined()
    expect(await repo.readIntentBytes(key)).toBeUndefined()
  })

  it('never turns repository acquisition failure into an unprotected network send', async () => {
    const { client, calls } = transport()
    const send = gatewayMessageSend(client, snapshot, {
      repository: async () => { throw new DOMException('Storage denied', 'SecurityError') },
    })
    expect(await send(request, key)).toMatchObject({ _tag: 'Refused', reason: 'storage' })
    expect(calls).toEqual([])
  })

  it('accepts intent before real-fence preparation rejects an invalid target, retaining preparing without dispatch', async () => {
    const repo = await open()
    const { client, calls } = transport()
    const send = bind(client, repo)
    expect(await send({ ...request, parameters: { ...request.parameters, to: 'invalid' } }, key)).toMatchObject({
      _tag: 'Refused', reason: 'failed', detail: 'The message is saved on this device, but preparation could not be completed.',
    })
    expect(await repo.read(key)).toMatchObject({ state: 'pending', phase: 'preparing', mayHaveDispatched: false })
    expect(await repo.readPacketBytes(key)).toBeUndefined()
    expect(calls.map(call => call.path)).toEqual(['/v1/client/capabilities'])
  })

  it('persists the exact unsigned packet bytes and dispatch permit before POST, then retains validated delivery', async () => {
    const repo = await open()
    let captured = ''
    const { client, calls } = transport(async body => {
      captured = body
      expect(await repo.readPacketBytes(key)).toEqual(new TextEncoder().encode(body))
      expect(await repo.read(key)).toMatchObject({ state: 'pending', phase: 'ready', mayHaveDispatched: true })
      const packet = Native.decodeUnknownSync(Native.ActionRequest)(JSON.parse(body))
      expect(Object.hasOwn(packet.parameters, 'signature')).toBe(false)
      return Response.json(receipt(body))
    })
    const send = bind(client, repo)
    expect(await send(request, key)).toMatchObject({ _tag: 'Success' })
    expect(await repo.read(key)).toMatchObject({ state: 'delivered', evidence: { kind: 'receipt' } })
    expect(await repo.readPacketBytes(key)).toEqual(new TextEncoder().encode(captured))
    const count = calls.length
    expect(await send({ ...request, _tag: 'Resend', idempotencyKey: key })).toMatchObject({ _tag: 'Success' })
    expect(calls.slice(count).map(call => call.path)).toEqual(['/v1/client/capabilities'])
  })

  it('reopens a crashed held POST as unknown and never dispatches its Resend', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const started = Promise.withResolvers<void>()
    const held = Promise.withResolvers<Response>()
    const { client, calls } = transport(() => { started.resolve(); return held.promise })
    const pending = bind(client, repo)(request, key)
    await started.promise
    expect(await repo.read(key)).toMatchObject({ mayHaveDispatched: true })
    repo.close()
    const reopened = await open(factory)
    expect(await reopened.read(key)).toMatchObject({ state: 'unknown', mayHaveDispatched: true })
    const count = calls.length
    expect(await bind(client, reopened)({ ...request, _tag: 'Resend', idempotencyKey: key }))
      .toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
    expect(calls.slice(count).map(call => call.path)).toEqual(['/v1/client/capabilities'])
    held.reject(new TypeError('Connection disappeared'))
    expect(await pending).toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
  })

  it('refuses a missing original Resend without inventing a record or making any request', async () => {
    const repo = await open()
    const { client, calls } = transport()
    expect(await bind(client, repo)({ ...request, _tag: 'Resend', idempotencyKey: key }))
      .toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
    expect(await repo.list()).toEqual([])
    expect(calls).toEqual([])
  })

  const bare = certainty['stale-snapshot']
  const explicit = certainty['recipient-retired']
  it.each([
    { name: 'golden explicit none', body: explicit, state: 'not-applied' },
    { name: 'outer explicit none', body: { ...bare, applied: 'none' }, state: 'not-applied' },
    { name: 'details explicit none', body: { ...bare, details: { applied: 'none' } }, state: 'not-applied' },
    { name: 'bare golden stale fence', body: bare, state: 'unknown' },
    { name: 'bare rate limited', body: { ...bare, code: 'rate-limited' }, state: 'unknown' },
    { name: 'explicit unknown', body: { ...bare, applied: 'unknown' }, state: 'unknown' },
    { name: 'outer unknown conflicts with none', body: { ...explicit, applied: 'unknown' }, state: 'unknown' },
    { name: 'details unknown conflicts with none', body: { ...explicit, details: { applied: 'unknown' } }, state: 'unknown' },
    ...[null, false, 0, '', 'future'].flatMap(applied => [
      { name: `malformed outer ${String(applied)}`, body: { ...explicit, applied }, state: 'unknown' },
      { name: `malformed details ${String(applied)}`, body: { ...explicit, details: { applied } }, state: 'unknown' },
    ]),
  ])('classifies $name by protocol certainty, never message text', async ({ body, state }) => {
    const repo = await open()
    const { client, calls } = transport(() => Response.json(body, { status: 409 }))
    const send = bind(client, repo)
    expect(await send(request, key)).toMatchObject({ _tag: 'Refused' })
    expect(await repo.read(key)).toMatchObject({ state })
    const count = calls.length
    expect(await send({ ...request, _tag: 'Resend', idempotencyKey: key })).toMatchObject({ _tag: 'Refused' })
    expect(calls.slice(count).map(call => call.path)).toEqual(['/v1/client/capabilities'])
  })

  it.each(['rejected', 'accepted', 'wrong-action', 'wrong-message', 'malformed'] as const)(
    'retains %s receipts as unknown and never POSTs their Resend', async kind => {
      const repo = await open()
      const { client, calls } = transport(body => {
        const response = receipt(body)
        return Response.json({ ...response, value: {
          ...response.value,
          status: kind === 'rejected' || kind === 'accepted' ? kind : response.value.status,
          action_id: kind === 'wrong-action' ? 'action/other' : response.value.action_id,
          affected_ids: kind === 'wrong-message' ? ['message/other'] : response.value.affected_ids,
          ...(kind === 'malformed' ? { terminal_attachment: { state: 'malformed' } } : {}),
        } })
      })
      const send = bind(client, repo)
      expect(await send(request, key)).toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
      expect(await repo.read(key)).toMatchObject({ state: 'unknown' })
      const count = calls.length
      expect(await send({ ...request, _tag: 'Resend', idempotencyKey: key }))
        .toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
      expect(calls.slice(count).map(call => call.path)).toEqual(['/v1/client/capabilities'])
    },
  )
})
