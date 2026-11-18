import 'fake-indexeddb/auto'
import { createHash } from 'node:crypto'
import { FractalClient } from '@fractal/client'
import { outboxDeploymentId } from '@fractal/client/outbox'
import * as Native from '@fractal/client/schema'
import { Effect } from 'effect'
import { IDBDatabase, IDBFactory, IDBObjectStore } from 'fake-indexeddb'
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

  it.each(['same tab', 'another tab after recovery'] as const)('settles an in-flight receipt as unconfirmed after discard in %s with bounded work', async mode => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const started = Promise.withResolvers<void>()
    const held = Promise.withResolvers<Response>()
    const { client, calls } = transport(() => { started.resolve(); return held.promise })
    const pending = bind(client, repo)(request, key)
    await started.promise
    const discarder = mode === 'same tab' ? repo : await open(factory)
    if (mode !== 'same tab') expect(await discarder.read(key)).toMatchObject({ state: 'unknown' })
    await discarder.dismiss(key, (await discarder.read(key))?.revision)
    const before = await discarder.read(key)
    const reads = vi.spyOn(repo, 'read')
    const writes = vi.spyOn(repo, 'compareAndSwapWritable')
    const transactions = vi.spyOn(IDBDatabase.prototype, 'transaction')
    const capabilityReads = calls.filter(call => call.path.endsWith('/capabilities')).length
    const body = calls.find(call => call.path.endsWith('/actions'))?.body
    held.resolve(Response.json(receipt(String(body))))
    expect(await pending).toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
    expect(calls.filter(call => call.path.endsWith('/capabilities')).length - capabilityReads).toBe(1)
    expect(transactions).toHaveBeenCalledTimes(1)
    expect(writes).toHaveBeenCalledTimes(1)
    await expect(writes.mock.results[0]?.value).resolves.toEqual({ _tag: 'not-writable' })
    expect(reads).not.toHaveBeenCalled()
    expect(await discarder.read(key)).toEqual(before)
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
    if (state === 'unknown') expect(calls.slice(count).map(call => call.path)).toEqual(['/v1/client/capabilities'])
    else {
      expect(calls.slice(count).filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
      expect(await repo.read(`${key}/retry`)).toMatchObject({ state: 'not-applied' })
    }
  })

  it('reprepares stale-fence nonapplication with a fresh fence and identities while preserving original text and images', async () => {
    const repo = await open()
    let posts = 0
    const { client, calls } = transport(body => ++posts === 1
      ? Response.json({ ...bare, applied: 'none' }, { status: 409 })
      : Response.json(receipt(body)))
    let snapshots = 0
    const prepared: string[][] = []
    const send = gatewayMessageSend(client, async fresh => {
      expect(fresh).toBe(snapshots > 0)
      return { _tag: 'Success', value: `snapshot/retry-${++snapshots}` }
    }, { repository: async () => repo })
    const original = { ...request, images: [{ bytes: new Uint8Array([1, 2, 3]), mediaType: 'image/png' as const, name: 'original.png' }] }
    expect(await send(original, key)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
    const first = Native.decodeUnknownSync(Native.ActionRequest)(JSON.parse(String(calls.find(call => call.path.endsWith('/actions'))?.body)))
    expect(await send({ ...request, _tag: 'Resend', idempotencyKey: key,
      parameters: { ...request.parameters, to: 'agent/changed', content: 'Changed', tags: [] }, images: [],
    }, undefined, ids => { prepared.push([...ids]) })).toMatchObject({ _tag: 'Success' })
    const actions = calls.filter(call => call.path.endsWith('/actions'))
    const second = Native.decodeUnknownSync(Native.ActionRequest)(JSON.parse(String(actions[1]?.body)))
    expect(second.id).not.toBe(first.id)
    expect(second.idempotency_key).toBe(`${key}/retry`)
    expect(second.fence.snapshot_id).toBe('snapshot/retry-2')
    expect(second.parameters).toEqual(first.parameters)
    expect(await repo.read(key)).toMatchObject({ state: 'not-applied' })
    expect(await repo.read(`${key}/retry`)).toMatchObject({ state: 'delivered' })
    const parent = await repo.read(key)
    const hash = parent?.intent.attachments[0]?.contentHash
    expect(hash).toBeDefined()
    expect(await repo.readSpool(`${key}/retry`, hash!)).toEqual(new Uint8Array([1, 2, 3]))
    expect(prepared).toEqual([receipt(String(actions[1]?.body)).value.affected_ids])
  })

  it.each(['capability', 'snapshot', 'actor', 'grant', 'preparation', 'dispatch-scope'] as const)(
    'retains an accepted retry child after a pre-dispatch %s refusal and continues that same child',
    async failure => {
      const repo = await open()
      let posts = 0
      let refusing = false
      const { client, calls } = transport(body => ++posts === 1
        ? Response.json({ ...bare, applied: 'none' }, { status: 409 })
        : Response.json(receipt(body)))
      const capabilities = client.capabilities.bind(client)
      let childCapabilityReads = 0
      vi.spyOn(client, 'capabilities').mockImplementation(async () => {
        if (!refusing || await repo.read(`${key}/retry`) === undefined) return capabilities()
        childCapabilityReads++
        if (failure === 'capability') throw new TypeError('Capability discovery unavailable')
        const response = await capabilities()
        if (failure === 'actor' || failure === 'dispatch-scope' && childCapabilityReads === 2) {
          return { ...response, value: { ...response.value, session_actor: 'person/other' } }
        }
        if (failure === 'grant') return { ...response, value: { ...response.value, capabilities: [] } }
        return response
      })
      const readSpool = repo.readSpool.bind(repo)
      vi.spyOn(repo, 'readSpool').mockImplementation(async (id, hash) =>
        refusing && failure === 'preparation' && id === `${key}/retry` ? undefined : readSpool(id, hash))
      const send = gatewayMessageSend(client, async () =>
        refusing && failure === 'snapshot'
          ? { _tag: 'Refused', reason: 'snapshot-unavailable', detail: 'Snapshot unavailable.' }
          : snapshot(), { repository: async () => repo })
      const original = { ...request, images: [{ bytes: new Uint8Array([1, 2, 3]), mediaType: 'image/png' as const, name: 'original.png' }] }
      expect(await send(original, key)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
      const first = Native.decodeUnknownSync(Native.ActionRequest)(JSON.parse(String(calls.find(call => call.path.endsWith('/actions'))?.body)))
      const resend: AttachmentSendRequest = {
        ...request, _tag: 'Resend', idempotencyKey: key,
        parameters: { ...request.parameters, content: 'Changed draft', to: 'agent/other', tags: [] },
      }
      refusing = true
      expect(await send(resend)).toMatchObject({ _tag: 'Refused', reason: 'retry-refused' })
      const child = await repo.read(`${key}/retry`)
      const intentBytes = await repo.readIntentBytes(`${key}/retry`)
      expect(child).toMatchObject({ state: 'pending', mayHaveDispatched: false, intent: { text: request.parameters.content } })
      expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
      refusing = false
      expect(await send(resend)).toMatchObject({ _tag: 'Success' })
      const actions = calls.filter(call => call.path.endsWith('/actions'))
      const second = Native.decodeUnknownSync(Native.ActionRequest)(JSON.parse(String(actions[1]?.body)))
      expect(actions).toHaveLength(2)
      expect(second.idempotency_key).toBe(child?.id)
      expect(second.id).not.toBe(first.id)
      expect(second.parameters).toEqual(first.parameters)
      expect(await repo.readIntentBytes(`${key}/retry`)).toEqual(intentBytes)
      expect(await repo.read(`${key}/retry`)).toMatchObject({ state: 'delivered' })
      expect(await repo.read(key)).toMatchObject({ state: 'not-applied' })
      expect(await repo.read(`${key}/retry/retry`)).toBeUndefined()
      expect(await repo.list()).toHaveLength(2)
    },
  )

  it.each(['protected scope', 'protected capability', 'unprotected scope'] as const)(
    'pins a slow Resend to its observed generation during %s discovery',
    async mode => {
      const repo = mode.startsWith('protected') ? await open() : undefined
      const { client, calls } = transport(() => Response.json({ ...bare, applied: 'none' }, { status: 409 }))
      const entered = Promise.withResolvers<void>()
      const resume = Promise.withResolvers<void>()
      let delayNext = false
      const delay = async () => {
        if (!delayNext) return
        delayNext = false
        entered.resolve()
        await resume.promise
      }
      const capabilities = client.capabilities.bind(client)
      vi.spyOn(client, 'capabilities').mockImplementation(async () => {
        if (mode === 'protected capability') await delay()
        return capabilities()
      })
      const send = gatewayMessageSend(client, snapshot, { repository: async () => {
        if (mode !== 'protected capability') await delay()
        return repo
      } })
      expect(await send(request, key)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
      const resend: AttachmentSendRequest = { ...request, _tag: 'Resend', idempotencyKey: key }
      delayNext = true
      const slow = send(resend)
      await entered.promise
      expect(await send(resend)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
      resume.resolve()
      expect(await slow).toMatchObject({ _tag: 'Refused', reason: 'retry-refused' })
      expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(2)
      if (repo !== undefined) {
        expect(await repo.read(`${key}/retry`)).toMatchObject({ state: 'not-applied' })
        expect(await repo.read(`${key}/retry/retry`)).toBeUndefined()
        expect(await repo.list()).toHaveLength(2)
      }
      // Only a subsequent deliberate user action may advance the observed child.
      expect(await send(resend)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
      expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(3)
      if (repo !== undefined) {
        expect(await repo.read(`${key}/retry/retry`)).toMatchObject({ state: 'not-applied' })
        expect(await repo.read(`${key}/retry/retry/retry`)).toBeUndefined()
        expect(await repo.list()).toHaveLength(3)
      }
    },
  )

  it('observes an existing unapplied child after reopening without advancing it in the same Resend', async () => {
    const repo = await open()
    const { client, calls } = transport(() => Response.json({ ...bare, applied: 'none' }, { status: 409 }))
    const first = bind(client, repo)
    const resend: AttachmentSendRequest = { ...request, _tag: 'Resend', idempotencyKey: key }
    expect(await first(request, key)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
    expect(await first(resend)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
    const reopened = bind(client, repo)
    expect(await reopened(resend)).toMatchObject({ _tag: 'Refused', reason: 'retry-refused' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(2)
    expect(await repo.read(`${key}/retry/retry`)).toBeUndefined()
    expect(await reopened(resend)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(3)
    expect(await repo.read(`${key}/retry/retry`)).toMatchObject({ state: 'not-applied' })
    expect(await repo.read(`${key}/retry/retry/retry`)).toBeUndefined()
  })

  it('converges concurrent retries on one child and one dispatch permit', async () => {
    const repo = await open()
    let posts = 0
    const started = Promise.withResolvers<void>()
    const held = Promise.withResolvers<Response>()
    const { client, calls } = transport(() => {
      if (++posts === 1) return Response.json({ ...bare, applied: 'none' }, { status: 409 })
      started.resolve()
      return held.promise
    })
    const send = bind(client, repo)
    expect(await send(request, key)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
    const resend: AttachmentSendRequest = { ...request, _tag: 'Resend', idempotencyKey: key }
    const attempts = [send(resend), send(resend)]
    await started.promise
    const actions = calls.filter(call => call.path.endsWith('/actions'))
    held.resolve(Response.json(receipt(String(actions[1]?.body))))
    const outcomes = await Promise.all(attempts)
    expect(outcomes.filter(result => result._tag === 'Success')).toHaveLength(1)
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(2)
    expect(await repo.read(`${key}/retry`)).toMatchObject({ state: 'delivered' })
    expect(await repo.read(`${key}/retry/retry`)).toBeUndefined()
  })

  it('keeps protected retries in their original scope instead of falling back to unprotected mode', async () => {
    const repo = await open()
    const { client, calls } = transport(() => Response.json({ ...bare, applied: 'none' }, { status: 409 }))
    let selected: IndexedDbOutbox | undefined = repo
    const send = gatewayMessageSend(client, snapshot, { repository: async () => selected })
    expect(await send(request, key)).toMatchObject({ _tag: 'Refused', reason: 'stale-fence' })
    selected = undefined
    const count = calls.length
    expect(await send({ ...request, _tag: 'Resend', idempotencyKey: key }))
      .toMatchObject({ _tag: 'Refused', reason: 'retry-refused' })
    expect(calls).toHaveLength(count)
    expect(await repo.read(`${key}/retry`)).toBeUndefined()
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
