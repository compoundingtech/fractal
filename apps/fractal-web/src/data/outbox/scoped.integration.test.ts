import 'fake-indexeddb/auto'
import { FractalClient, type Capabilities, type EnvelopeOf } from '@fractal/client'
import { createOutboxRecord, observeOutbox, outboxDeploymentId, prepareOutboxRecord, restoreOutboxRecord } from '@fractal/client/outbox'
import { prepareMessageSendPacket } from '@fractal/client/prepared-packet'
import * as Native from '@fractal/client/schema/read'
import { IDBFactory } from 'fake-indexeddb'
import { afterEach, describe, expect, it } from 'vitest'
import fixture from '../subjectReadPort.gateway.fixtures.json'
import { gatewayMessageSend } from '../attachmentPort.ts'
import { createScopedChatOutbox, scopeFromCapabilities, type ScopedChatOutbox } from './scoped.ts'
import { attemptsStore, defaultDatabaseName, spoolStore } from './indexedDb.ts'

const decodedCapabilities = Native.decodeUnknownSync(Native.Capabilities)(fixture.capabilities.value)
const base = {
  ...fixture.capabilities,
  api_version: 'st3.client.v0',
  snapshot: { ...fixture.capabilities.snapshot, projection_version: 'client-projection.v0' },
  value: { ...fixture.capabilities.value, kind: 'capabilities', transport: decodedCapabilities.transport, capabilities: [...decodedCapabilities.capabilities], issuer: 'person/example', issuer_key: 'issuer-key/a' },
} satisfies EnvelopeOf<Capabilities>
const owners: ScopedChatOutbox[] = []
const setup = (indexedDB = new IDBFactory()) => {
  let response: EnvelopeOf<Capabilities> = base
  let fail = false
  const calls: string[] = []
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async (input) => {
    calls.push(new URL(String(input)).pathname)
    if (fail) throw new TypeError('Offline')
    return Response.json(response)
  } })
  const owner = createScopedChatOutbox(client, { indexedDB })
  owners.push(owner)
  return { client, owner, calls, respond: (next: EnvelopeOf<Capabilities>) => { response = next }, offline: () => { fail = true } }
}
afterEach(() => { for (const owner of owners.splice(0)) owner.close() })

const openStoredDatabase = (factory: IDBFactory): Promise<IDBDatabase> => {
  const result = Promise.withResolvers<IDBDatabase>()
  const request = factory.open(defaultDatabaseName, 1)
  request.onsuccess = () => result.resolve(request.result)
  request.onerror = () => result.reject(request.error)
  return result.promise
}
const storedRows = async (factory: IDBFactory, store = attemptsStore): Promise<unknown[]> => {
  const db = await openStoredDatabase(factory)
  try {
    const result = Promise.withResolvers<unknown[]>()
    const tx = db.transaction(store, 'readonly')
    const request = tx.objectStore(store).getAll()
    tx.oncomplete = () => result.resolve(request.result)
    tx.onabort = () => result.reject(tx.error)
    return await result.promise
  } finally { db.close() }
}

it('does not open storage when disposed during startup identity discovery', async () => {
  let release!: () => void
  const gate = new Promise<void>(resolve => { release = resolve })
  const factory = new IDBFactory()
  let opens = 0
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async () => {
    await gate
    return Response.json(base)
  } })
  const owner = createScopedChatOutbox(client, { indexedDB: { open: (name, version) => {
    opens++
    return factory.open(name, version)
  } } })
  owners.push(owner)
  const startup = owner.initialize()
  owner.close()
  release()
  await expect(startup).rejects.toMatchObject({ _tag: 'OutboxIdentityUnavailable' })
  expect(opens).toBe(0)
})


it('cross-checks a cached identity before first recovery without changing foreign dispatched storage', async () => {
  const factory = new IDBFactory()
  const original = setup(factory)
  await original.owner.initialize(base)
  const repo = await original.owner.repository()
  if (repo === undefined) throw new TypeError('Expected protected repository')
  const intent = createOutboxRecord('attempt/stale-startup', { text: 'Original content', attachments: [] })
  const ready = prepareOutboxRecord(intent, prepareMessageSendPacket({
    id: 'action/stale-startup', idempotency_key: intent.id, fence: { snapshot_id: base.snapshot.id, subject_revisions: {} },
    parameters: { to: 'agent/example', content: intent.intent.text, tags: [], attachments: [] },
  }))
  if (ready === undefined) throw new TypeError('Expected ready packet')
  await repo.commit(intent)
  await repo.compareAndSwap(intent.revision, ready)
  const dispatched = await repo.acquireDispatchPermit(intent.id, ready.revision)
  const reopened = setup(factory)
  reopened.owner.observe(base)
  reopened.respond({ ...base, value: { ...base.value, issuer_key: 'issuer-key/changed' } })
  await expect(reopened.owner.repository()).rejects.toMatchObject({ _tag: 'OutboxIdentityUnavailable' })
  expect(await repo.read(intent.id)).toEqual(dispatched)
  expect(await reopened.owner.recovery.list()).toContainEqual({
    id: intent.id, revision: dispatched.revision, kind: 'identity-mismatch', subject: 'message', status: 'Identity changed; delivery unknown. Return to the original account and deployment.', actions: ['discard'],
  })
  expect(await repo.read(intent.id)).toEqual(dispatched)
  expect(reopened.calls.every(path => path === '/v1/client/capabilities')).toBe(true)
})
describe('verified browser outbox scope', () => {
  it.each(['preparing', 'ready', 'dispatched'] as const)('opens and recovers %s storage before startup resolves without posting', async phase => {
    const factory = new IDBFactory()
    const original = setup(factory)
    await original.owner.initialize(base)
    const repo = await original.owner.repository()
    if (repo === undefined) throw new TypeError('Expected protected repository')
    const initial = createOutboxRecord('attempt/startup', { text: 'saved before restart', attachments: [] })
    await repo.commit(initial)
    const packet = prepareMessageSendPacket({ id: 'action/startup', idempotency_key: 'startup-packet-key', fence: { snapshot_id: base.snapshot.id, subject_revisions: {} }, parameters: { to: 'agent/example', content: initial.intent.text, tags: [], attachments: [] } })
    const ready = prepareOutboxRecord(initial, packet, ['message/startup'])
    if (ready === undefined) throw new TypeError('Expected ready record')
    if (phase !== 'preparing') await repo.compareAndSwap(initial.revision, ready)
    const saved = phase === 'dispatched' ? await repo.acquireDispatchPermit(initial.id, ready.revision) : phase === 'ready' ? ready : initial
    original.owner.close()

    const reloaded = setup(factory)
    let opened = false
    const owner = createScopedChatOutbox(reloaded.client, {
      indexedDB: { open: (name, version) => {
        const request = factory.open(name, version)
        request.addEventListener('success', () => { opened = true }, { once: true })
        return request
      } },
    })
    owners.push(owner)
    await owner.initialize()
    expect(opened).toBe(true)
    const restored = await owner.repository()
    expect(await restored?.read(initial.id)).toEqual(restoreOutboxRecord(saved))
    expect(reloaded.calls).toEqual(['/v1/client/capabilities'])
  })

  it('shares startup with repository access and never recovers its own live dispatch again', async () => {
    const factory = new IDBFactory()
    const { client, calls } = setup(factory)
    const persistence = Promise.withResolvers<boolean>()
    const persisting = Promise.withResolvers<void>()
    let opens = 0
    let persists = 0
    const owner = createScopedChatOutbox(client, {
      indexedDB: { open: (name, version) => { opens++; return factory.open(name, version) } },
      storage: { persist: () => { persists++; persisting.resolve(); return persistence.promise } },
    })
    owners.push(owner)
    const first = owner.initialize(base)
    const second = owner.initialize(base)
    let exposed = false
    const repository = owner.repository().then(repo => { exposed = true; return repo })
    await persisting.promise
    expect(exposed).toBe(false)
    expect(opens).toBe(0)
    persistence.resolve(true)
    await Promise.all([first, second])
    const repo = await repository
    if (repo === undefined) throw new TypeError('Expected protected repository')
    expect(opens).toBe(1)
    expect(persists).toBe(1)
    const initial = createOutboxRecord('attempt/live', { text: 'live dispatch', attachments: [] })
    const packet = prepareMessageSendPacket({ id: 'action/live', idempotency_key: 'live-packet-key-long', fence: { snapshot_id: base.snapshot.id, subject_revisions: {} }, parameters: { to: 'agent/example', content: initial.intent.text, tags: [], attachments: [] } })
    const ready = prepareOutboxRecord(initial, packet, ['message/live'])
    if (ready === undefined) throw new TypeError('Expected ready record')
    await repo.commit(initial)
    await repo.compareAndSwap(initial.revision, ready)
    const dispatched = await repo.acquireDispatchPermit(initial.id, ready.revision)
    await Promise.all([owner.initialize(base), owner.initialize()])
    await owner.recovery.list()
    expect(await repo.read(initial.id)).toEqual(dispatched)
    expect(opens).toBe(2) // The recovery catalog is separate from the scoped connection.
    expect(persists).toBe(1)

    const other = setup(factory)
    await other.owner.initialize(base)
    const unknown = await repo.read(initial.id)
    expect(unknown).toMatchObject({ state: 'unknown', ambiguous: true, revision: dispatched.revision + 1 })
    const observation = { kind: 'observed', identity: 'message/live', source: 'timeline' } as const
    expect(await repo.compareAndSwapWritable(dispatched.revision, observeOutbox(dispatched, observation)))
      .toMatchObject({ _tag: 'revision-conflict' })
    if (unknown === undefined) throw new TypeError('Expected retained unknown record')
    expect(await repo.compareAndSwapWritable(unknown.revision, observeOutbox(unknown, observation)))
      .toMatchObject({ _tag: 'written', record: { state: 'delivered', ambiguous: true } })
    expect(calls.every(path => path === '/v1/client/capabilities')).toBe(true)
    expect(other.calls).toEqual([])
  })

  it('retains corrupt startup entries as discard-only rather than failing initialization', async () => {
    const factory = new IDBFactory()
    const original = setup(factory)
    await original.owner.initialize(base)
    const repo = await original.owner.repository()
    if (repo === undefined) throw new TypeError('Expected protected repository')
    const db = await openStoredDatabase(factory)
    try {
      const finished = Promise.withResolvers<void>()
      const tx = db.transaction(attemptsStore, 'readwrite')
      tx.objectStore(attemptsStore).put({ localAttemptId: 'attempt/corrupt-startup', scope: repo.scope, schemaVersion: 999 })
      tx.oncomplete = () => finished.resolve()
      tx.onabort = () => finished.reject(tx.error)
      await finished.promise
    } finally { db.close() }
    original.owner.close()
    const before = await storedRows(factory)
    const reloaded = setup(factory)
    await reloaded.owner.initialize(base)
    expect(await reloaded.owner.recovery.list()).toEqual([
      { id: 'attempt/corrupt-startup', revision: undefined, kind: 'corrupt', subject: 'message', status: 'storage damaged; delivery unknown', actions: ['discard'] },
    ])
    expect(await storedRows(factory)).toEqual(before)
    await expect((await reloaded.owner.repository())?.read('attempt/corrupt-startup')).rejects.toMatchObject({ code: 'quarantined' })
    expect(reloaded.calls).toEqual(['/v1/client/capabilities'])
  })

  it('reports a real blocked database upgrade as unavailable, never a corrupt recovery entry', async () => {
    const { client } = setup()
    const factory = new IDBFactory()
    const name = 'blocked-outbox'
    const heldResult = Promise.withResolvers<IDBDatabase>()
    const heldRequest = factory.open(name, 1)
    heldRequest.onsuccess = () => heldResult.resolve(heldRequest.result)
    heldRequest.onerror = () => heldResult.reject(heldRequest.error)
    const held = await heldResult.promise
    let opening: IDBOpenDBRequest | undefined
    const owner = createScopedChatOutbox(client, {
      databaseName: name,
      indexedDB: { open: databaseName => { opening = factory.open(databaseName, 2); return opening } },
    })
    owners.push(owner)
    owner.observe(base)
    try {
      await expect(owner.recovery.list()).rejects.toMatchObject({
        _tag: 'OutboxStorageError', code: 'disabled', message: 'Outbox database upgrade is blocked.',
      })
      // The old database remains untouched while the upgrade cannot acquire it.
      expect([...held.objectStoreNames]).toEqual([])
    } finally {
      const finished = Promise.withResolvers<void>()
      if (opening === undefined) finished.resolve()
      else {
        opening.addEventListener('success', () => finished.resolve(), { once: true })
        opening.addEventListener('error', () => finished.reject(opening?.error), { once: true })
      }
      held.close()
      await finished.promise
    }
    const reopenedResult = Promise.withResolvers<IDBDatabase>()
    const reopenedRequest = factory.open(name, 2)
    reopenedRequest.onsuccess = () => reopenedResult.resolve(reopenedRequest.result)
    reopenedRequest.onerror = () => reopenedResult.reject(reopenedRequest.error)
    const reopened = await reopenedResult.promise
    try {
      const rowsResult = Promise.withResolvers<unknown[]>()
      const request = reopened.transaction('attempts-v1', 'readonly').objectStore('attempts-v1').getAll()
      request.onsuccess = () => rowsResult.resolve(request.result)
      request.onerror = () => rowsResult.reject(request.error)
      const rows = await rowsResult.promise
      expect(rows).toEqual([])
    } finally { reopened.close() }
  })

  it('reports disabled opening as unavailable, never a corrupt recovery entry', async () => {
    const { client } = setup()
    const owner = createScopedChatOutbox(client, {
      indexedDB: { open: () => { throw new DOMException('Unavailable', 'InvalidStateError') } },
    })
    owners.push(owner)
    owner.observe(base)
    await expect(owner.initialize(base)).rejects.toMatchObject({ _tag: 'OutboxStorageError', code: 'disabled' })
    await expect(owner.recovery.list()).rejects.toMatchObject({ _tag: 'OutboxStorageError', code: 'disabled' })
  })

  it.each(['only A', 'A then B', 'B then delayed A', 'unprotected B then delayed A'])('keeps scope observations ordered by read start: %s', async order => {
    const { owner, respond } = setup()
    const a = owner.beginObservation()
    if (order === 'only A' || order === 'A then B') await owner.initialize(base, a)
    let expected: EnvelopeOf<Capabilities> = base
    if (order !== 'only A') {
      const b = owner.beginObservation()
      const { issuer: _issuer, issuer_key: _key, ...withoutIssuer } = base.value
      expected = { ...base, value: order.startsWith('unprotected') ? withoutIssuer : { ...base.value, issuer_key: 'issuer-key/b' } }
      respond(expected)
      await owner.initialize(expected, b)
      if (order.includes('delayed A')) await owner.initialize(base, a)
      expect(owner.isCurrentObservation(base)).toBe(false)
    }
    expect(owner.isCurrentObservation(expected)).toBe(true)
    expect(owner.protection()).toBe(order.startsWith('unprotected') ? 'unprotected' : 'durable')
    expect((await owner.repository())?.scope).toEqual(scopeFromCapabilities(expected))
  })

  it('uses the cached generated issuer pair without another discovery and persists a protected record', async () => {
    const { owner, calls } = setup()
    owner.observe(base)
    await owner.initialize(base)
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
    const { client } = setup()
    const owner = createScopedChatOutbox(client, {
      indexedDB: { open: () => { throw new TypeError('Unprotected initialization must not open IndexedDB') } },
    })
    owners.push(owner)
    const { issuer: _issuer, issuer_key: _key, ...value } = base.value
    await owner.initialize({ ...base, value: { ...value, ...(kind === 'issuer-only' ? { issuer: base.value.issuer } : {}), ...(kind === 'key-only' ? { issuer_key: base.value.issuer_key } : {}) } })
    expect(await owner.repository()).toBeUndefined()
    expect(owner.protection()).toBe('unprotected')
  })

  it('separates different issuer pairs without making another scope\'s records readable', async () => {
    const { owner, respond } = setup()
    owner.observe(base)
    const first = await owner.repository()
    await first?.commit(createOutboxRecord('attempt/original', { text: 'original', attachments: [] }))
    respond({ ...base, value: { ...base.value, issuer_key: 'issuer-key/b' } })
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
    expect(items).toContainEqual({ id: initial.id, revision: dispatched.revision, kind: 'identity-mismatch', subject: 'message', status: 'Identity changed; delivery unknown. Return to the original account and deployment.', actions: ['discard'] })
    expect(await repo.read(initial.id)).toEqual(dispatched)
    let snapshots = 0
    const send = gatewayMessageSend(client, async () => { snapshots++; throw new Error('No snapshot on mismatch') }, { repository: async () => repo })
    expect(await send({ _tag: 'Resend', idempotencyKey: initial.id, api_version: 'st3.client.v0', type: 'message.send', id: packet.id, parameters: { ...packet.parameters, tags: packet.parameters.tags ?? [], attachments: packet.parameters.attachments ?? [] } })).toMatchObject({
      _tag: 'Refused', reason: 'retry-refused',
      detail: 'Return to the original account and deployment to inspect or send this message.',
    })
    expect(snapshots).toBe(0)
    expect(await repo.read(initial.id)).toEqual(dispatched)
    expect(calls.every(path => path === '/v1/client/capabilities')).toBe(true)
    expect(scopeFromCapabilities(changed)).not.toEqual(repo.scope)
  })

  it.each([
    ['different', false], ['different', true], ['missing', false], ['missing', true],
  ] as const)('discovers persisted foreign attempts after reload with %s issuer identity (dispatched: %s), without changing or sending them', async (kind, didDispatch) => {
    const factory = new IDBFactory()
    const original = setup(factory)
    original.owner.observe(base)
    const first = await original.owner.repository()
    if (first === undefined) throw new TypeError('Expected protected repository')
    const initial = createOutboxRecord('attempt/reloaded', { text: 'retained across reload', attachments: [] })
    const packet = prepareMessageSendPacket({ id: 'action/reloaded', idempotency_key: 'reloaded-packet-key', fence: { snapshot_id: base.snapshot.id, subject_revisions: {} }, parameters: { to: 'agent/example', content: initial.intent.text, tags: [], attachments: [] } })
    const ready = prepareOutboxRecord(initial, packet, ['message/reloaded'])
    if (ready === undefined) throw new TypeError('Expected ready record')
    await first.commit(initial)
    await first.compareAndSwap(initial.revision, ready)
    const saved = didDispatch ? await first.acquireDispatchPermit(initial.id, ready.revision) : ready
    const before = await storedRows(factory)
    original.owner.close()

    const reloaded = setup(factory)
    const { issuer: _issuer, issuer_key: _key, ...value } = base.value
    const changed = { ...base, value: { ...value, ...(kind === 'different' ? { issuer: 'person/other', issuer_key: 'issuer-key/b' } : {}) } }
    reloaded.respond(changed)
    reloaded.owner.observe(changed)
    const expected = [{ id: initial.id, revision: saved.revision, kind: 'identity-mismatch', subject: 'message', status: 'Identity changed; delivery unknown. Return to the original account and deployment.', actions: ['discard'] }]
    expect(await reloaded.owner.recovery.list()).toEqual(expected)
    expect(await reloaded.owner.recovery.list()).toEqual(expected)
    const current = await reloaded.owner.repository()
    if (kind === 'different') {
      expect(current?.scope).not.toEqual(first.scope)
      expect(await current?.read(initial.id)).toBeUndefined()
      expect(await current?.list()).toEqual([])
    } else expect(current).toBeUndefined()
    const send = gatewayMessageSend(reloaded.client, async () => { throw new TypeError('Foreign attempt must not acquire a snapshot') }, reloaded.owner)
    expect(await send({ _tag: 'Resend', idempotencyKey: initial.id, api_version: 'st3.client.v0', type: 'message.send', id: packet.id, parameters: { ...packet.parameters, tags: [], attachments: [] } }))
      .toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
    expect(reloaded.calls.every(path => path === '/v1/client/capabilities')).toBe(true)
    expect(await storedRows(factory)).toEqual(before)

    // Discard also resolves persisted scopes without relying on a previous list/connection map.
    reloaded.owner.close()
    const discarder = setup(factory)
    discarder.respond(changed)
    discarder.owner.observe(changed)
    await discarder.owner.recovery.discard(initial.id, saved?.revision)
    expect(await discarder.owner.recovery.list()).toEqual([])
    discarder.respond(base)
    discarder.owner.observe(base)
    const retained = await discarder.owner.repository()
    expect(await retained?.read(initial.id)).toEqual(saved)
    expect(await retained?.list()).toEqual([])
  })

  it('keeps scoped and unscoped damaged persisted rows visible and explicitly discardable without an issuer', async () => {
    const factory = new IDBFactory()
    const original = setup(factory)
    original.owner.observe(base)
    const repo = await original.owner.repository()
    if (repo === undefined) throw new TypeError('Expected protected repository')
    const db = await openStoredDatabase(factory)
    try {
      const result = Promise.withResolvers<void>()
      const tx = db.transaction([attemptsStore, spoolStore], 'readwrite')
      tx.objectStore(attemptsStore).put({ localAttemptId: 'attempt/damaged-scoped', scope: repo.scope, schemaVersion: 999, record: { revision: 4 } })
      tx.objectStore(attemptsStore).put({ localAttemptId: 'attempt/damaged-unscoped', scope: { invalid: true }, record: { revision: 2 } })
      for (const id of ['attempt/damaged-scoped', 'attempt/damaged-unscoped']) {
        tx.objectStore(spoolStore).put({ localAttemptId: id, hash: 'damaged', bytes: new Uint8Array([1, 2]) })
      }
      tx.oncomplete = () => result.resolve()
      tx.onabort = () => result.reject(tx.error)
      await result.promise
    } finally { db.close() }
    original.owner.close()
    const reloaded = setup(factory)
    const { issuer: _issuer, issuer_key: _key, ...value } = base.value
    reloaded.respond({ ...base, value })
    const before = await storedRows(factory)
    const items = await reloaded.owner.recovery.list()
    expect(items).toHaveLength(2)
    for (const id of ['attempt/damaged-scoped', 'attempt/damaged-unscoped']) {
      expect(items).toContainEqual({ id, revision: id.endsWith('unscoped') ? 2 : 4, kind: 'corrupt', subject: 'message', status: 'storage damaged; delivery unknown', actions: ['discard'] })
    }
    expect(await storedRows(factory)).toEqual(before)
    for (const item of items) await reloaded.owner.recovery.discard(item.id, item.revision)
    expect(await reloaded.owner.recovery.list()).toEqual([])
    expect(await storedRows(factory)).toEqual([])
    expect(await storedRows(factory, spoolStore)).toEqual([])
  })
})
