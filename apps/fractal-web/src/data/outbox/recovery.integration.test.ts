import 'fake-indexeddb/auto'
import { createHash } from 'node:crypto'
import { FractalClient } from '@fractal/client'
import { createOutboxRecord, observeOutbox, prepareOutboxRecord } from '@fractal/client/outbox'
import { prepareMessageSendPacket, type PreparedPacket } from '@fractal/client/prepared-packet'
import { IDBFactory } from 'fake-indexeddb'
import { Effect } from 'effect'
import { afterEach, describe, expect, it, vi } from 'vitest'
import fixture from '../subjectReadPort.gateway.fixtures.json'
import refusals from '../../../../../docs/st3/client-v0/fixtures/thread-start-refusals.json'
import type { ThreadOutboxInput } from './threadSender.ts'
import type { AttachmentSendRequest } from '../source.ts'
import { spoolAttachment } from './indexedDb.ts'
import { createScopedChatOutbox, type ScopedChatOutbox } from './scoped.ts'

const base = { ...fixture.capabilities, value: { ...fixture.capabilities.value, issuer: 'person/example', issuer_key: 'issuer-key/recovery', transport: 'local-web',
  capabilities: [...fixture.capabilities.value.capabilities, { id: 'thread.start', version: 1, state: 'granted' }, { id: 'thread.reply', version: 1, state: 'granted' }],
} }
const request: AttachmentSendRequest = { _tag: 'Send', api_version: 'st3.client.v0', type: 'message.send', id: 'action/recovery-original',
  parameters: { to: fixture.cases.agent.ref, content: 'Original café', tags: ['original'], attachments: [], title: 'Original title' } }
const owners: ScopedChatOutbox[] = []
afterEach(() => { vi.restoreAllMocks(); for (const owner of owners.splice(0)) owner.close() })
const setup = (indexedDB = new IDBFactory()) => {
  let identity = base
  let offline = false
  let recipientValid = true
  let postOutcome: 'unknown' | 'none' | undefined
  const calls: { path: string; body?: string }[] = []
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async (input, init) => {
    const path = new URL(String(input)).pathname
    const body = init?.body === undefined ? undefined : String(init.body)
    calls.push({ path, ...(body === undefined ? {} : { body }) })
    if (offline) throw new TypeError('Offline')
    if (path.endsWith('/capabilities')) return Response.json(identity)
    if (path.includes('/agents/')) return recipientValid ? Response.json(fixture.cases.agent.payload)
      : Response.json({ ...fixture.cases.agent.payload, value: { ...fixture.cases.agent.payload.value, id: 'agent/different' } })
    if (path.includes('/start-context')) return Response.json({ ...base, value: { kind: 'thread-start-context', source_id: 'source/recovery',
      episode: 'episode/original', start_recipient: { agent_id: recipientValid ? request.parameters.to : 'agent/different', basis: { _tag: 'original' } } } })
    if (path.includes('/threads/')) return Response.json({ ...base, value: { id: 'thread/original', agent_id: request.parameters.to,
      title: 'Original thread', title_revision: 'claim/title', revision: 'claim/latest', state: recipientValid ? 'open' : 'closed' } })
    if (path.endsWith('/blobs')) return Response.json({ ...base, value: {
      blob: `blob/${'b'.repeat(64)}`, sha256: 'b'.repeat(64), size: 3, media_type: 'image/png',
    } })
    if (path.endsWith('/actions') && body !== undefined) {
      if (postOutcome === 'unknown') throw new TypeError('Lost action response')
      if (postOutcome === 'none') return Response.json({ ...refusals['stale-snapshot'], applied: 'none' }, { status: 409 })
      const action: { id: string; idempotency_key: string; parameters: { target_id?: string } } = JSON.parse(body)
      return Response.json({ ...base, value: { kind: 'action-result', status: 'completed', action_id: action.id,
        operation_id: 'operation/recovery', snapshot_id: base.snapshot.id,
        affected_ids: [...(action.parameters.target_id === undefined ? [] : [action.parameters.target_id]),
          `message/${createHash('sha256').update(action.idempotency_key).digest('hex').slice(0, 16)}`] } })
    }
    throw new Error(`Unexpected recovery request ${path}`)
  } })
  const owner = createScopedChatOutbox(client, { indexedDB })
  owners.push(owner)
  return { owner, client, calls, setOffline: () => { offline = true }, invalidateRecipient: () => { recipientValid = false },
    setPostOutcome: (next: 'unknown' | 'none') => { postOutcome = next },
    validateRecipient: () => { recipientValid = true },
    changeIdentity: () => { identity = { ...base, value: { ...base.value, issuer_key: 'issuer-key/other' } } } }
}
const seed = async (owner: ScopedChatOutbox, state: 'pending' | 'unknown' | 'not-applied' = 'pending', withImage = false, key = 'attempt/recovery-original') => {
  const repo = await owner.repository()
  if (repo === undefined) throw new Error('Expected protected repository')
  const spool = withImage ? [await Effect.runPromise(spoolAttachment(new Uint8Array([1, 2, 3])))] : []
  const initial = createOutboxRecord(key, { text: request.parameters.content, attachments: spool.map(image => ({
    contentHash: image.hash, reference: `${key}/${image.hash}`, name: 'Original.png', mediaType: 'image/png',
  })) })
  const { _tag: _, ...intent } = request
  const bytes = new TextEncoder().encode(JSON.stringify(intent))
  expect(await repo.accept(initial, spool, bytes)).toBe(true)
  if (state !== 'pending') {
    const ready = prepareOutboxRecord<PreparedPacket>(initial, prepareMessageSendPacket({ ...intent, parameters: {
      ...intent.parameters, tags: [...intent.parameters.tags], attachments: intent.parameters.attachments.map(image => ({ ...image, name: image.name ?? undefined })),
    }, idempotency_key: key, fence: { snapshot_id: base.snapshot.id, subject_revisions: {} } }))
    if (ready === undefined) throw new Error('Expected prepared record')
    expect(await repo.compareAndSwap(initial.revision, ready)).toBe(true)
    const dispatched = await repo.acquireDispatchPermit(key, ready.revision)
    const next = observeOutbox(dispatched, { kind: 'failure', applied: state === 'not-applied' ? 'none' : 'unknown' })
    expect(await repo.compareAndSwap(dispatched.revision, next)).toBe(true)
  }
  return { repo, key, bytes }
}
const actions = <TCall extends { path: string }>(calls: readonly TCall[]) => calls.filter(call => call.path.endsWith('/actions'))

describe('protected recovery API', () => {
  it('never posts an unknown attempt through Send or Retry, and retains its exact immutable content', async () => {
    const { owner, calls } = setup()
    const { repo, key, bytes } = await seed(owner, 'unknown')
    const before = await repo.read(key)
    expect(await owner.recovery.list()).toEqual([expect.objectContaining({ id: key, kind: 'unknown', subject: 'message',
      status: 'Fractal has not confirmed this message/thread. Do not send it again.', actions: ['copy', 'discard'] })])
    await expect(owner.recovery.send!(key)).rejects.toThrow('cannot be sent again')
    await expect(owner.recovery.retry!(key)).rejects.toThrow('cannot be sent again')
    expect(actions(calls)).toHaveLength(0)
    expect(await repo.read(key)).toEqual(before)
    expect(await repo.readIntentBytes(key)).toEqual(bytes)
  })

  it('revalidates the immutable original pending recipient before any dispatch and leaves refused pending intact', async () => {
    const { owner, calls, invalidateRecipient } = setup()
    const { repo, key } = await seed(owner)
    const before = await repo.read(key)
    invalidateRecipient()
    await expect(owner.recovery.send!(key)).rejects.toThrow('original recipient')
    expect(calls.some(call => call.path === `/v1/client/agents/${encodeURIComponent(fixture.cases.agent.ref)}`)).toBe(true)
    expect(actions(calls)).toHaveLength(0)
    expect(await repo.read(key)).toEqual(before)
  })

  it('uses the pending original attempt once across concurrent explicit sends', async () => {
    const { owner, calls } = setup()
    const { repo, key } = await seed(owner)
    const results = await Promise.allSettled([owner.recovery.send!(key), owner.recovery.send!(key)])
    expect(results.some(result => result.status === 'fulfilled')).toBe(true)
    expect(actions(calls)).toHaveLength(1)
    expect(await repo.read(key)).toMatchObject({ state: 'delivered' })
    expect(await owner.recovery.list()).toEqual([])
    await expect(owner.recovery.send!(key)).rejects.toThrow('no longer available')
    expect(actions(calls)).toHaveLength(1)
  })

  it('retries explicit-none into one deterministic fresh child, hides the parent, and refuses stale parent commands', async () => {
    const { owner, calls } = setup()
    const { repo, key } = await seed(owner, 'not-applied')
    const before = await repo.read(key)
    expect(await owner.recovery.list()).toEqual([expect.objectContaining({ id: key, kind: 'not-applied', actions: ['retry', 'discard'] })])
    await owner.recovery.retry!(key)
    expect(await repo.read(key)).toEqual(before)
    expect(await repo.read(`${key}/retry`)).toMatchObject({ state: 'delivered' })
    const posted = JSON.parse(actions(calls)[0]!.body!)
    expect(posted.id).not.toBe(request.id)
    expect(posted.idempotency_key).toBe(`${key}/retry`)
    expect(posted.parameters.content).toBe(request.parameters.content)
    expect(await owner.recovery.list()).toEqual([])
    await expect(owner.recovery.retry!(key)).rejects.toThrow('newer attempt')
    expect(actions(calls)).toHaveLength(1)
  })

  it('copies retained images and sends a genuinely independent attempt without changing the original unknown record', async () => {
    const { owner, calls } = setup()
    const { repo, key, bytes } = await seed(owner, 'unknown', true)
    const before = await repo.read(key)
    const copy = await owner.recovery.copy!(key)
    expect(copy).toEqual({ copyKey: expect.any(String), text: 'Original café', images: [{ name: 'Original.png', mediaType: 'image/png', bytes: 3 }] })
    expect(await owner.recovery.sendCopy!(key, copy.copyKey, 'Edited independent message')).toEqual({ _tag: 'Recorded' })
    expect(await owner.recovery.sendCopy!(key, copy.copyKey, 'Must not create a second message')).toEqual({ _tag: 'Recorded' })
    expect(actions(calls)).toHaveLength(1)
    expect(await repo.read(key)).toEqual(before)
    expect(await repo.readIntentBytes(key)).toEqual(bytes)
    const posted = JSON.parse(actions(calls)[0]!.body!)
    expect(posted.idempotency_key).not.toBe(key)
    expect(posted.idempotency_key).not.toContain('/retry')
    expect(posted.parameters).toMatchObject({ to: request.parameters.to, content: 'Edited independent message', title: 'Original title',
      attachments: [{ name: 'Original.png', media_type: 'image/png' }] })
    expect(await owner.recovery.list()).toEqual([expect.objectContaining({ id: key, kind: 'unknown', actions: ['copy', 'discard'] })])
  })

  it('keeps missing immutable intent discard-only and unavailable identity unavailable, with no send work', async () => {
    const { owner, calls, setOffline } = setup()
    const repo = await owner.repository()
    if (repo === undefined) throw new Error('Expected repository')
    await repo.commit(createOutboxRecord('attempt/unreadable', { text: 'Not a recoverable intent', attachments: [] }))
    expect(await owner.recovery.list()).toEqual([expect.objectContaining({ id: 'attempt/unreadable', kind: 'corrupt', actions: ['discard'] })])
    await expect(owner.recovery.send!('attempt/unreadable')).rejects.toThrow('unavailable')
    await owner.recovery.discard('attempt/unreadable')
    expect(await owner.recovery.list()).toEqual([])
    setOffline()
    await expect(owner.recovery.list()).rejects.toMatchObject({ _tag: 'OutboxIdentityUnavailable' })
    expect(actions(calls)).toHaveLength(0)
  })

  it('does not expose foreign-scope content or permit copied/retried work under another identity', async () => {
    const { owner, calls, changeIdentity } = setup()
    const { repo, key } = await seed(owner, 'unknown')
    const before = await repo.read(key)
    changeIdentity()
    const items = await owner.recovery.list()
    expect(items).toEqual([expect.objectContaining({ id: key, kind: 'identity-mismatch', actions: ['discard'] })])
    expect(items[0]).not.toHaveProperty('text')
    await expect(owner.recovery.copy!(key)).rejects.toThrow()
    await expect(owner.recovery.retry!(key)).rejects.toThrow()
    expect(await repo.read(key)).toEqual(before)
    expect(actions(calls)).toHaveLength(0)
  })

  it('publishes cross-tab write wakeups that only cause persisted re-reads, never duplicate dispatch', async () => {
    const indexedDB = new IDBFactory()
    const first = setup(indexedDB)
    const second = setup(indexedDB)
    await first.owner.initialize()
    await second.owner.initialize()
    const wake = Promise.withResolvers<void>()
    const stop = second.owner.recovery.subscribe!(() => { wake.resolve() })
    const { key } = await seed(first.owner)
    await wake.promise
    expect(await second.owner.recovery.list()).toEqual([expect.objectContaining({ id: key, kind: 'pending' })])
    const forged = Promise.withResolvers<void>()
    stop()
    const stopForged = second.owner.recovery.subscribe!(() => { forged.resolve() })
    const channel = new BroadcastChannel('fractal-outbox/recovery')
    try {
      channel.postMessage({ id: key, state: 'not-applied', actions: ['retry'], permit: true })
      await forged.promise
      expect(await second.owner.recovery.list()).toEqual([expect.objectContaining({ id: key, kind: 'pending', actions: ['send', 'discard'] })])
      expect(actions([...first.calls, ...second.calls])).toHaveLength(0)
      await Promise.allSettled([first.owner.recovery.send!(key), second.owner.recovery.send!(key)])
      expect(actions([...first.calls, ...second.calls])).toHaveLength(1)
    } finally { stopForged(); channel.close() }
  })

  it.each(['unknown', 'none'] as const)('records a copied %s outcome under one stable key and never resends it', async outcome => {
    const { owner, calls, setPostOutcome } = setup()
    const { repo, key } = await seed(owner, 'unknown')
    const before = await repo.read(key)
    const copy = await owner.recovery.copy!(key)
    setPostOutcome(outcome)
    const first = owner.recovery.sendCopy!(key, copy.copyKey, 'Independent first content')
    const concurrent = owner.recovery.sendCopy!(key, copy.copyKey, 'Changed concurrent content')
    expect(concurrent).toBe(first)
    expect(await first).toEqual({ _tag: 'Recorded' })
    expect(await concurrent).toEqual({ _tag: 'Recorded' })
    expect(await owner.recovery.sendCopy!(key, copy.copyKey, 'Later edited content')).toEqual({ _tag: 'Recorded' })
    expect(actions(calls)).toHaveLength(1)
    expect(await repo.read(copy.copyKey)).toMatchObject({ state: outcome === 'none' ? 'not-applied' : 'unknown', intent: { text: 'Independent first content' } })
    expect(await repo.read(key)).toEqual(before)
    expect(await owner.recovery.list()).toContainEqual(expect.objectContaining({ id: copy.copyKey, kind: outcome === 'none' ? 'not-applied' : 'unknown' }))
  })

  it('does not call unreadable storage corrupt or silently lose retained images', async () => {
    const { owner, calls } = setup()
    const { repo, key } = await seed(owner, 'unknown', true)
    const failure = new Error('IndexedDB read failed')
    vi.spyOn(repo, 'readIntentBytes').mockRejectedValueOnce(failure)
    await expect(owner.recovery.list()).rejects.toBe(failure)
    vi.spyOn(repo, 'readSpool').mockResolvedValueOnce(undefined)
    await expect(owner.recovery.copy!(key)).rejects.toThrow('saved image is unavailable')
    expect(actions(calls)).toHaveLength(0)
    expect(await repo.read(key)).toMatchObject({ state: 'unknown' })
  })

  it('decodes supported optional message parameters before exposing recovery actions', async () => {
    const { owner, calls } = setup()
    const repo = await owner.repository()
    if (repo === undefined) throw new Error('Expected repository')
    const key = 'attempt/invalid-optional-fields'
    const initial = createOutboxRecord(key, { text: 'Invalid session identity', attachments: [] })
    await repo.accept(initial, [], new TextEncoder().encode(JSON.stringify({ api_version: 'st3.client.v0', type: 'message.send',
      id: 'action/invalid-session', parameters: { ...request.parameters, session_id: 42 } })))
    expect(await owner.recovery.list()).toEqual([expect.objectContaining({ id: key, kind: 'corrupt', actions: ['discard'] })])
    await expect(owner.recovery.send!(key)).rejects.toThrow('damaged')
    expect(actions(calls)).toHaveLength(0)
  })

  const threadInputs: readonly ThreadOutboxInput[] = [
    { key: 'attempt/recovery-thread-start', type: 'thread.start', parameters: { target_id: 'thread/original', agent_id: request.parameters.to,
      title: 'Saved recovery thread', content: 'Saved thread start', origin: { source_id: 'source/recovery', episode: 'episode/original' } } },
    { key: 'attempt/recovery-thread-reply', type: 'thread.reply', parameters: { target_id: 'thread/original', content: 'Saved thread reply', thread_quote_id: 'message/quoted' } },
  ]
  it.each(threadInputs)('revalidates original $type before allowing saved pending thread dispatch', async input => {
    const { owner, calls, invalidateRecipient } = setup()
    const repo = await owner.repository()
    if (repo === undefined) throw new Error('Expected repository')
    const record = createOutboxRecord(input.key, { text: input.parameters.content, attachments: [] })
    await repo.accept(record, [], new TextEncoder().encode(JSON.stringify(input)))
    expect(await owner.recovery.list()).toEqual([expect.objectContaining({ id: input.key, subject: 'thread', kind: 'pending', actions: ['send', 'discard'] })])
    invalidateRecipient()
    await expect(owner.recovery.send!(input.key)).rejects.toThrow()
    expect(actions(calls)).toHaveLength(0)
    expect(await repo.read(input.key)).toEqual(record)
  })

  it.each(threadInputs)('sends saved $type once with its original destination and quote', async input => {
    const { owner, calls } = setup()
    const repo = await owner.repository()
    if (repo === undefined) throw new Error('Expected repository')
    await repo.accept(createOutboxRecord(input.key, { text: input.parameters.content, attachments: [] }), [], new TextEncoder().encode(JSON.stringify(input)))
    await owner.recovery.send!(input.key)
    expect(await repo.read(input.key)).toMatchObject({ state: 'delivered' })
    expect(actions(calls)).toHaveLength(1)
    expect(JSON.parse(actions(calls)[0]!.body!).parameters).toEqual(input.parameters)
    expect(await owner.recovery.list()).toEqual([])
  })

  it('does not follow an unknown child through a stale parent Retry', async () => {
    const { owner, calls } = setup()
    const { key } = await seed(owner, 'not-applied')
    const { repo } = await seed(owner, 'unknown', false, `${key}/retry`)
    const child = await repo.read(`${key}/retry`)
    expect(await owner.recovery.list()).toEqual([expect.objectContaining({ id: `${key}/retry`, kind: 'unknown', actions: ['copy', 'discard'] })])
    await expect(owner.recovery.retry!(key)).rejects.toThrow('newer attempt')
    expect(await repo.read(`${key}/retry`)).toEqual(child)
    expect(actions(calls)).toHaveLength(0)
  })

  it('does not resurrect a retry parent when its dismissed child has become a content-free tombstone', async () => {
    const { owner, calls } = setup()
    const { key } = await seed(owner, 'not-applied')
    const { repo } = await seed(owner, 'unknown', false, `${key}/retry`)
    await repo.dismiss(`${key}/retry`)
    await repo.cleanupTerminal()
    expect(await repo.read(`${key}/retry`)).toBeUndefined()
    expect(await repo.hasAttempt(`${key}/retry`)).toBe(true)
    expect(await owner.recovery.list()).toEqual([])
    await expect(owner.recovery.retry!(key)).rejects.toThrow('newer attempt')
    expect(actions(calls)).toHaveLength(0)
  })

  it('shares copy refusals before acceptance and retains a single key for a later explicit send', async () => {
    const { owner, calls, invalidateRecipient, validateRecipient } = setup()
    const { repo, key } = await seed(owner, 'unknown')
    const copy = await owner.recovery.copy!(key)
    invalidateRecipient()
    const first = owner.recovery.sendCopy!(key, copy.copyKey, 'Initial edit')
    expect(owner.recovery.sendCopy!(key, copy.copyKey, 'Concurrent edit')).toBe(first)
    expect(await first).toMatchObject({ _tag: 'Refused' })
    expect(await repo.read(copy.copyKey)).toBeUndefined()
    expect(actions(calls)).toHaveLength(0)
    validateRecipient()
    expect(await owner.recovery.sendCopy!(key, copy.copyKey, 'Accepted edit')).toEqual({ _tag: 'Recorded' })
    expect(await repo.read(copy.copyKey)).toMatchObject({ state: 'delivered', intent: { text: 'Accepted edit' } })
    expect(actions(calls)).toHaveLength(1)
  })
})
