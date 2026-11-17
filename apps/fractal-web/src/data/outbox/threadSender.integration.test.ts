import 'fake-indexeddb/auto'
import { ClientError, FractalClient, createOutboxRecord, outboxDeploymentId } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { Effect } from 'effect'
import { IDBFactory, IDBObjectStore } from 'fake-indexeddb'
import { afterEach, describe, expect, it, vi } from 'vitest'
import refusals from '../../../../../docs/st3/client-v0/fixtures/thread-start-refusals.json'
import fixture from '../subjectReadPort.gateway.fixtures.json'
import { attemptsStore, openIndexedDbOutbox, type IndexedDbOutbox } from './indexedDb.ts'
import { createThreadOutboxSender, type ThreadOutboxInput } from './threadSender.ts'
import { createScopedChatOutbox } from './scoped.ts'

const base = { ...fixture.capabilities, value: { ...fixture.capabilities.value,
  issuer: 'person/example', issuer_key: 'issuer-key/a', transport: 'local-web',
  capabilities: [...fixture.capabilities.value.capabilities,
    { id: 'thread.start', version: 1, state: 'granted' }, { id: 'thread.reply', version: 1, state: 'granted' }],
} }
const scope = { principalId: base.value.session_actor, deploymentId: outboxDeploymentId(base.value.issuer, base.value.issuer_key), memberId: base.snapshot.host_id }
const start: ThreadOutboxInput = { key: 'thread-durable-start-key', type: 'thread.start', parameters: {
  target_id: 'thread/original', agent_id: 'agent/atlas', content: 'Original café', title: 'Original title',
  origin: { source_id: 'inbox/source', episode: 'revision/episode' },
} }
const reply: ThreadOutboxInput = { key: 'thread-durable-reply-key', type: 'thread.reply', parameters: {
  target_id: 'thread/original', content: 'Exact reply', thread_quote_id: 'message/quoted',
} }
const connections: IndexedDbOutbox[] = []
const open = async (factory = new IDBFactory()) => {
  const repo = await Effect.runPromise(openIndexedDbOutbox({ scope, indexedDB: factory }))
  connections.push(repo)
  return repo
}
const receipt = (body: string) => {
  const action = Native.decodeUnknownSync(Native.ActionRequest)(JSON.parse(body))
  return { ...base, value: { kind: 'action-result', status: 'completed', action_id: action.id,
    operation_id: 'operation/thread', snapshot_id: base.snapshot.id,
    affected_ids: ['thread/original', 'message/arbitrary-valid-receipt'],
  } }
}
const transport = (post: (body: string) => Response | Promise<Response> = body => Response.json(receipt(body)), capability: () => unknown = () => base) => {
  const calls: { path: string; body: string }[] = []
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async (input, init) => {
    const path = new URL(String(input)).pathname
    calls.push({ path, body: String(init?.body ?? '') })
    return path.endsWith('/capabilities') ? Response.json(capability()) : post(String(init?.body))
  } })
  return { client, calls }
}
const bind = (client: FractalClient, repo?: IndexedDbOutbox) => createThreadOutboxSender(client, { repository: async () => repo })
afterEach(() => { vi.restoreAllMocks(); for (const repo of connections.splice(0)) repo.close() })

describe('durable thread delivery', () => {
  it.each([start, reply])('commits full $type intent and exact prepared packet before network dispatch', async input => {
    const repo = await open()
    let committed = false
    const add = IDBObjectStore.prototype.add
    const { client, calls } = transport(async body => {
      expect(committed).toBe(true)
      expect(await repo.readPacketBytes(input.key)).toEqual(new TextEncoder().encode(body))
      expect(await repo.read(input.key)).toMatchObject({ mayHaveDispatched: true })
      expect(JSON.parse(body).parameters).toEqual(input.parameters)
      return Response.json(receipt(body))
    })
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      const request = add.call(this, value, key)
      if (this.name === attemptsStore) this.transaction.addEventListener('complete', () => { expect(calls).toEqual([]); committed = true })
      return request
    })
    expect(await bind(client, repo).send(input)).toEqual({ how: 'sent' })
    expect(JSON.parse(new TextDecoder().decode(await repo.readIntentBytes(input.key)))).toEqual(input)
    expect(await repo.read(input.key)).toMatchObject({ state: 'delivered', expectedIdentities: [expect.stringMatching(/^message\//)] })
  })

  it('does not send when intent acceptance aborts', async () => {
    const repo = await open()
    const { client, calls } = transport()
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      const request = add.call(this, value, key)
      if (this.name === attemptsStore) request.addEventListener('success', () => this.transaction.abort())
      return request
    })
    expect(await bind(client, repo).send(start)).toMatchObject({ how: 'failed' })
    expect(calls).toEqual([])
  })

  it('recovers a crash after permit commit but before POST as unknown and never replays', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const { client, calls } = transport()
    const acquire = repo.acquireDispatchPermit
    vi.spyOn(repo, 'acquireDispatchPermit').mockImplementation(async (id, revision) => {
      await acquire(id, revision)
      throw new Error('Process stopped before socket call')
    })
    expect(await bind(client, repo).send(start)).toEqual({ how: 'unconfirmed' })
    const reopened = await open(factory)
    expect(await reopened.read(start.key)).toMatchObject({ state: 'unknown', ambiguous: true })
    expect(await bind(client, reopened).send(start)).toEqual({ how: 'unconfirmed' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toEqual([])
  })

  it.each(Object.entries(refusals))('uses golden refusal %s exactly as decoded', async (_name, envelope) => {
    const repo = await open()
    const { client } = transport(() => Response.json(envelope, { status: 409 }))
    const result = await bind(client, repo).send(start)
    const certainty = 'applied' in envelope && envelope.applied === 'none' ? 'not-applied' : 'unconfirmed'
    expect(result.how).toBe(certainty)
    if (result.how === 'not-applied') {
      expect(result.error).toBeInstanceOf(ClientError)
      if (result.error instanceof ClientError) expect(result.error.response).toEqual(Native.decodeUnknownSync(Native.ErrorEnvelope)(envelope))
    }
    expect(await repo.read(start.key)).toMatchObject({ state: certainty === 'not-applied' ? 'not-applied' : 'unknown' })
  })

  it.each(['protected', 'unprotected'] as const)('reprepares explicit none with immutable parameters and a fresh fence in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    const bodies: string[] = []
    const freshSnapshot = 'snapshot/retry'
    let validated = false
    const { client } = transport(body => {
      bodies.push(body)
      return bodies.length === 1 ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body))
    }, () => !validated ? base : { ...base, snapshot: { ...base.snapshot, id: freshSnapshot } })
    const sender = bind(client, repo)
    const revalidate = vi.fn(async () => { validated = true })
    expect(await sender.send(start, { revalidate })).toMatchObject({ how: 'not-applied' })
    const changedInput = { ...start, parameters: { ...start.parameters, content: 'Do not replace the original intent' } }
    expect(await sender.send(changedInput, { revalidate })).toEqual({ how: 'sent' })
    expect(bodies).toHaveLength(2)
    const original = JSON.parse(bodies[0]!)
    const retried = JSON.parse(bodies[1]!)
    expect(retried.parameters).toEqual(original.parameters)
    expect(retried.id).toBe(`action/${start.key}/retry`)
    expect(retried.idempotency_key).toBe(`${start.key}/retry`)
    expect(retried.fence.snapshot_id).toBe(freshSnapshot)
    expect(revalidate).toHaveBeenCalledTimes(1)
    if (repo !== undefined) expect(await repo.read(`${start.key}/retry`)).toMatchObject({ state: 'delivered' })
  })

  it.each(['protected', 'unprotected'] as const)('accepts a fresh retry after an explicit-none stale fence in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    const bodies: string[] = []
    const { client } = transport(body => {
      bodies.push(body)
      if (bodies.length === 1) return Response.json({ ...refusals['stale-snapshot'], applied: 'none', details: { applied: 'none' } }, { status: 409 })
      expect(JSON.parse(body).fence.snapshot_id).toBe('snapshot/current')
      return Response.json(receipt(body))
    }, () => bodies.length === 0 ? base : { ...base, snapshot: { ...base.snapshot, id: 'snapshot/current' } })
    const sender = bind(client, repo)
    expect(await sender.send(reply)).toMatchObject({ how: 'not-applied' })
    expect(await sender.send(reply, { revalidate: async () => {} })).toEqual({ how: 'sent' })
    expect(bodies).toHaveLength(2)
  })

  it.each(['protected', 'unprotected'] as const)('keeps failed retry revalidation retryable without POST in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    let sends = 0
    const { client, calls } = transport(body => ++sends === 1
      ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body)))
    const sender = bind(client, repo)
    expect(await sender.send(reply)).toMatchObject({ how: 'not-applied' })
    expect(await sender.send(reply)).toMatchObject({ how: 'retry-refused', error: expect.any(Error) })
    const reason = new Error('The original reply thread is closed.')
    const revalidate = vi.fn(async () => { throw reason })
    expect(await sender.send(reply, { revalidate })).toEqual({ how: 'retry-refused', error: reason })
    expect(await sender.send(reply, { revalidate })).toEqual({ how: 'retry-refused', error: reason })
    expect(revalidate).toHaveBeenCalledTimes(2)
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
    expect(await sender.send(reply, { revalidate: async () => {} })).toEqual({ how: 'sent' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(2)
  })

  it('revalidates a prepared never-dispatched retry child on every explicit retry', async () => {
    const repo = await open()
    let sends = 0
    const { client } = transport(body => ++sends === 1
      ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body)))
    const sender = bind(client, repo)
    expect(await sender.send(start)).toMatchObject({ how: 'not-applied' })
    const permit = vi.spyOn(repo, 'acquireDispatchPermit').mockRejectedValueOnce(new Error('Stopped before permit'))
    expect(await sender.send(start, { revalidate: async () => {} })).toEqual({ how: 'unconfirmed' })
    expect(await repo.read(`${start.key}/retry`)).toMatchObject({ state: 'pending', phase: 'ready', mayHaveDispatched: false })
    permit.mockRestore()
    const reason = new Error('The original Start recipient retired.')
    const revalidate = vi.fn(async () => { throw reason })
    expect(await sender.send(start, { revalidate })).toEqual({ how: 'retry-refused', error: reason })
    expect(await sender.send(start)).toMatchObject({ how: 'retry-refused' })
    expect(sends).toBe(1)
    expect(await sender.send(start, { revalidate: async () => {} })).toEqual({ how: 'sent' })
    expect(sends).toBe(2)
    expect(await repo.read(`${start.key}/retry/retry`)).toBeUndefined()
  })

  it.each(['protected', 'unprotected'] as const)('never replays unknown in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    const { client, calls } = transport(() => { throw new TypeError('Lost response') })
    const sender = bind(client, repo)
    expect(await sender.send(reply)).toEqual({ how: 'unconfirmed' })
    expect(await sender.send(reply)).toEqual({ how: 'unconfirmed' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
  })

  it('requires visible recipient revalidation only on restored never-dispatched Start', async () => {
    const repo = await open()
    await repo.accept(createOutboxRecord(start.key, { text: start.parameters.content, attachments: [] }), [], new TextEncoder().encode(JSON.stringify(start)))
    const { client, calls } = transport()
    const sender = bind(client, repo)
    expect(await sender.send(start)).toMatchObject({ how: 'failed' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toEqual([])
    const hook = vi.fn(async () => { expect(await repo.read(start.key)).toMatchObject({ mayHaveDispatched: false }) })
    expect(await sender.send(start, { revalidate: hook })).toEqual({ how: 'sent' })
    expect(hook).toHaveBeenCalledTimes(1)
  })

  it('refuses a switched scope before dispatch and does not retain switched-scope outcomes', async () => {
    const repo = await open()
    let switched = false
    const capabilities = () => switched ? { ...base, value: { ...base.value, session_actor: 'person/other' } } : base
    const { client } = transport(body => { switched = true; return Response.json(receipt(body)) }, capabilities)
    expect(await bind(client, repo).send(reply)).toEqual({ how: 'unconfirmed' })
    expect(await repo.read(reply.key)).toMatchObject({ mayHaveDispatched: true, state: 'pending' })
    expect(await bind(client, repo).send(reply)).toMatchObject({ how: 'failed' })
  })

  it('rejects a mismatched action receipt as unknown', async () => {
    const repo = await open()
    const { client } = transport(body => { const response = receipt(body); return Response.json({ ...response, value: { ...response.value, action_id: 'action/other' } }) })
    expect(await bind(client, repo).send(reply)).toEqual({ how: 'unconfirmed' })
    expect(await repo.read(reply.key)).toMatchObject({ state: 'unknown' })
  })

  it('stops retention immediately after an in-flight discard', async () => {
    const repo = await open()
    const started = Promise.withResolvers<void>()
    const held = Promise.withResolvers<Response>()
    let body = ''
    const { client } = transport(value => { body = value; started.resolve(); return held.promise })
    const pending = bind(client, repo).send(reply)
    await started.promise
    await repo.dismiss(reply.key)
    const writes = vi.spyOn(repo, 'compareAndSwapWritable')
    const reads = vi.spyOn(repo, 'read')
    held.resolve(Response.json(receipt(body)))
    expect(await pending).toEqual({ how: 'unconfirmed' })
    expect(writes).toHaveBeenCalledTimes(1)
    await expect(writes.mock.results[0]?.value).resolves.toEqual({ _tag: 'not-writable' })
    expect(reads).not.toHaveBeenCalled()
  })
})

describe('thread sender account and retry ownership', () => {
  it('rejects the source actor changing before preparation without a send', async () => {
    const repo = await open()
    const { client, calls } = transport()
    expect(await bind(client, repo).send(reply, { expectedActor: 'person/other' })).toMatchObject({ how: 'failed' })
    expect(calls).toEqual([])
    expect(await repo.read(reply.key)).toBeUndefined()
  })

  it('binds an unprotected fresh retry to its original actor', async () => {
    let switched = false
    const { client, calls } = transport(() => Response.json(refusals['recipient-retired'], { status: 409 }),
      () => switched ? { ...base, value: { ...base.value, session_actor: 'person/other' } } : base)
    const sender = bind(client)
    expect(await sender.send(reply)).toMatchObject({ how: 'not-applied' })
    switched = true
    expect(await sender.send(reply)).toMatchObject({ how: 'retry-refused' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
  })

  it('uses declared retry intent before repository discovery can fail', async () => {
    const reason = new Error('Repository discovery failed')
    const { client, calls } = transport()
    const sender = createThreadOutboxSender(client, { repository: async () => { throw reason } })
    expect(await sender.send(reply, { retry: true })).toEqual({ how: 'retry-refused', error: reason })
    expect(calls).toEqual([])
  })

  it.each(['protected', 'unprotected'] as const)('refuses retry actor and grant changes without losing ownership in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    let sends = 0
    let revokeGrant = false
    const { client, calls } = transport(body => ++sends === 1
      ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body)),
    () => revokeGrant ? { ...base, value: { ...base.value, capabilities: base.value.capabilities.filter(grant => grant.id !== reply.type) } } : base)
    const sender = bind(client, repo)
    expect(await sender.send(reply)).toMatchObject({ how: 'not-applied' })
    expect(await sender.send(reply, { expectedActor: 'person/other' })).toMatchObject({ how: 'retry-refused' })
    revokeGrant = true
    expect(await sender.send(reply, { revalidate: async () => {} })).toMatchObject({ how: 'retry-refused' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
    revokeGrant = false
    expect(await sender.send(reply, { revalidate: async () => {} })).toEqual({ how: 'sent' })
    expect(sends).toBe(2)
  })

  it.each(['protected', 'unprotected'] as const)('retains retry ownership through capability failures in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    let sends = 0
    let failCapabilities = false
    const reason = new Error('Capabilities unavailable')
    const { client, calls } = transport(body => ++sends === 1
      ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body)),
    () => { if (failCapabilities) throw reason; return base })
    const sender = bind(client, repo)
    expect(await sender.send(reply)).toMatchObject({ how: 'not-applied' })
    failCapabilities = true
    expect(await sender.send(reply, { revalidate: async () => {} })).toMatchObject({ how: 'retry-refused' })
    failCapabilities = false
    expect(await sender.send(reply, { revalidate: async () => { failCapabilities = true } })).toMatchObject({ how: 'retry-refused' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
    failCapabilities = false
    expect(await sender.send(reply, { revalidate: async () => {} })).toEqual({ how: 'sent' })
    const actions = calls.filter(call => call.path.endsWith('/actions'))
    expect(actions).toHaveLength(2)
    expect(JSON.parse(actions[1]!.body).idempotency_key).toBe(`${reply.key}/retry`)
    if (repo !== undefined) expect(await repo.read(`${reply.key}/retry/retry`)).toBeUndefined()
  })

  it('infers a restored explicit-none retry before its initial scope check', async () => {
    const repo = await open()
    let switched = false
    let sends = 0
    const { client } = transport(body => ++sends === 1
      ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body)),
    () => switched ? { ...base, value: { ...base.value, session_actor: 'person/other' } } : base)
    expect(await bind(client, repo).send(reply)).toMatchObject({ how: 'not-applied' })
    const sender = bind(client, repo)
    switched = true
    expect(await sender.send(reply)).toMatchObject({ how: 'retry-refused' })
    expect(await repo.read(`${reply.key}/retry`)).toBeUndefined()
    switched = false
    expect(await sender.send(reply, { revalidate: async () => {} })).toEqual({ how: 'sent' })
    expect(sends).toBe(2)
  })

  it('retains the same pending child when preparation or shared dispatch scope refuses a retry', async () => {
    const repo = await open()
    let sends = 0
    let scopeChecks = 0
    let refuseDispatch = false
    const { client } = transport(body => ++sends === 1
      ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body)),
    () => {
      if (refuseDispatch && ++scopeChecks === 3) return { ...base, value: { ...base.value, session_actor: 'person/other' } }
      return base
    })
    const sender = bind(client, repo)
    expect(await sender.send(reply)).toMatchObject({ how: 'not-applied' })
    const reason = new Error('Packet identity preparation failed')
    const digest = vi.spyOn(crypto.subtle, 'digest').mockRejectedValueOnce(reason)
    expect(await sender.send(reply, { revalidate: async () => {} })).toEqual({ how: 'retry-refused', error: reason })
    expect(await repo.read(`${reply.key}/retry`)).toMatchObject({ state: 'pending', phase: 'preparing', mayHaveDispatched: false })
    digest.mockRestore()
    refuseDispatch = true
    expect(await sender.send(reply, { revalidate: async () => {} })).toMatchObject({ how: 'retry-refused' })
    expect(await repo.read(`${reply.key}/retry`)).toMatchObject({ state: 'pending', phase: 'ready', mayHaveDispatched: false })
    expect(sends).toBe(1)
    refuseDispatch = false
    expect(await sender.send(reply, { revalidate: async () => {} })).toEqual({ how: 'sent' })
    expect(sends).toBe(2)
    expect(await repo.read(`${reply.key}/retry/retry`)).toBeUndefined()
  })

  it('converges concurrent retry owners on one fresh-packet child permit', async () => {
    const factory = new IDBFactory()
    const first = await open(factory)
    let sends = 0
    const bodies: string[] = []
    const { client } = transport(body => {
      bodies.push(body)
      sends++
      return sends === 1 ? Response.json(refusals['recipient-retired'], { status: 409 }) : Response.json(receipt(body))
    })
    expect(await bind(client, first).send(start)).toMatchObject({ how: 'not-applied' })
    const second = await open(factory)
    const options = { revalidate: async () => {} }
    const outcomes = await Promise.all([bind(client, first).send(start, options), bind(client, second).send(start, options)])
    expect(outcomes.some(result => result.how === 'sent')).toBe(true)
    expect(bodies).toHaveLength(2)
    expect(JSON.parse(bodies[1]!).parameters).toEqual(JSON.parse(bodies[0]!).parameters)
    expect(JSON.parse(bodies[1]!).idempotency_key).toBe(`${start.key}/retry`)
    expect(await first.read(`${start.key}/retry`)).toMatchObject({ state: 'delivered' })
  })

  it.each(['protected', 'unprotected'] as const)('pins an invoked retry while another owner quickly refuses its child in %s mode', async mode => {
    const repo = mode === 'protected' ? await open() : undefined
    const bodies: string[] = []
    const { client } = transport(body => {
      bodies.push(body)
      return Response.json(refusals['recipient-retired'], { status: 409 })
    })
    const delayedOwner = bind(client, repo)
    expect(await delayedOwner.send(reply)).toMatchObject({ how: 'not-applied' })
    const competingOwner = repo === undefined ? delayedOwner : bind(client, repo)
    const entered = Promise.withResolvers<void>()
    const release = Promise.withResolvers<void>()
    const capabilities = client.capabilities.bind(client)
    vi.spyOn(client, 'capabilities').mockImplementationOnce(async () => {
      entered.resolve()
      await release.promise
      return capabilities()
    })
    const options = { retry: true, revalidate: async () => {} }
    const delayed = delayedOwner.send(reply, options)
    await entered.promise
    expect(await competingOwner.send(reply, options)).toMatchObject({ how: 'not-applied' })
    expect(bodies).toHaveLength(2)
    release.resolve()
    expect(await delayed).toMatchObject({ how: 'not-applied' })
    expect(bodies.map(body => JSON.parse(body).idempotency_key)).toEqual([reply.key, `${reply.key}/retry`])
    if (repo !== undefined) expect(await repo.read(`${reply.key}/retry/retry`)).toBeUndefined()
    // Only a subsequent invocation may consume the child's newly observed refusal.
    expect(await delayedOwner.send(reply, options)).toMatchObject({ how: 'not-applied' })
    expect(bodies.map(body => JSON.parse(body).idempotency_key)).toEqual([reply.key, `${reply.key}/retry`, `${reply.key}/retry/retry`])
  })

  it('limits a fresh sender to the refused root and its single existing child', async () => {
    const repo = await open()
    const { client, calls } = transport(() => Response.json(refusals['recipient-retired'], { status: 409 }))
    const original = bind(client, repo)
    const options = { retry: true, revalidate: async () => {} }
    expect(await original.send(reply)).toMatchObject({ how: 'not-applied' })
    expect(await original.send(reply, options)).toMatchObject({ how: 'not-applied' })
    const restored = bind(client, repo)
    expect(await restored.send(reply, options)).toMatchObject({ how: 'not-applied' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(2)
    expect(await repo.read(`${reply.key}/retry/retry`)).toBeUndefined()
    expect(await restored.send(reply, options)).toMatchObject({ how: 'not-applied' })
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(3)
    expect(await repo.read(`${reply.key}/retry/retry`)).toMatchObject({ state: 'not-applied' })
  })
})

it('never creates another retry after the fresh retry child becomes unknown', async () => {
  const repo = await open()
  let sends = 0
  const { client } = transport(() => {
    sends++
    if (sends === 1) return Response.json(refusals['recipient-retired'], { status: 409 })
    throw new TypeError('Lost fresh retry response')
  })
  const sender = bind(client, repo)
  expect(await sender.send(start)).toMatchObject({ how: 'not-applied' })
  expect(await sender.send(start, { retry: true, revalidate: async () => {} })).toEqual({ how: 'unconfirmed' })
  expect(await sender.send(start, { retry: true, revalidate: async () => {} })).toEqual({ how: 'unconfirmed' })
  expect(sends).toBe(2)
  expect(await repo.read(`${start.key}/retry`)).toMatchObject({ state: 'unknown' })
  expect(await repo.read(`${start.key}/retry/retry`)).toBeUndefined()
})

it.each(['before retry', 'during retry acceptance'] as const)('does not accept a retry of a discarded explicit-none attempt %s', async mode => {
  const repo = await open()
  const { client, calls } = transport(() => Response.json(refusals['recipient-retired'], { status: 409 }))
  const sender = bind(client, repo)
  expect(await sender.send(start)).toMatchObject({ how: 'not-applied' })
  if (mode === 'before retry') await repo.dismiss(start.key)
  else {
    const accept = repo.accept
    vi.spyOn(repo, 'accept').mockImplementation(async (record, spool, bytes, predecessor) => {
      if (predecessor !== undefined) await repo.dismiss(predecessor.id)
      return accept(record, spool, bytes, predecessor)
    })
  }
  expect(await sender.send(start)).toEqual({ how: 'unconfirmed' })
  expect(await repo.read(`${start.key}/retry`)).toBeUndefined()
  expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
})

it('keeps a protected attempt bound to its original repository when cached identity loses its issuer', async () => {
  let missingIssuer = false
  const { client, calls } = transport(() => Response.json(refusals['recipient-retired'], { status: 409 }),
    () => missingIssuer ? { ...base, value: { ...base.value, issuer: undefined, issuer_key: undefined } } : base)
  const owner = createScopedChatOutbox(client, { indexedDB: new IDBFactory() })
  try {
    owner.observe(await client.capabilities())
    const repo = await owner.repository()
    expect(repo).toBeDefined()
    const sender = createThreadOutboxSender(client, owner)
    expect(await sender.send(start)).toMatchObject({ how: 'not-applied' })
    const before = await repo?.read(start.key)
    missingIssuer = true
    owner.observe(await client.capabilities())
    expect(await sender.send(start)).toMatchObject({ how: 'retry-refused' })
    expect(await repo?.read(start.key)).toEqual(before)
    expect(await repo?.read(`${start.key}/retry`)).toBeUndefined()
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
  } finally { owner.close() }
})

it('does not rebuild an unprotected retry when the scoped owner gains an issuer', async () => {
  let hasIssuer = false
  const { client, calls } = transport(() => Response.json(refusals['recipient-retired'], { status: 409 }),
    () => hasIssuer ? base : { ...base, value: { ...base.value, issuer: undefined, issuer_key: undefined } })
  const owner = createScopedChatOutbox(client, { indexedDB: new IDBFactory() })
  try {
    owner.observe(await client.capabilities())
    const sender = createThreadOutboxSender(client, owner)
    expect(await sender.send(start)).toMatchObject({ how: 'not-applied' })
    hasIssuer = true
    owner.observe(await client.capabilities())
    const repo = await owner.repository()
    expect(repo).toBeDefined()
    expect(await sender.send(start)).toMatchObject({ how: 'retry-refused' })
    expect(await repo?.read(start.key)).toBeUndefined()
    expect(calls.filter(call => call.path.endsWith('/actions'))).toHaveLength(1)
  } finally { owner.close() }
})
