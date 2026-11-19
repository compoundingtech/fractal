import { FractalClient } from '@fractal/client'
import type { OutboxRecord } from '@fractal/client/outbox'
import type { PreparedPacket } from '@fractal/client/prepared-packet'
import { flushSync } from 'react-dom'
import { createRoot } from 'react-dom/client'

import { OutboxRecovery } from '../../web/OutboxRecovery.tsx'
import { recoveryStoreFor } from '../../web/outboxRecoveryState.ts'
import { gatewayMessageSend, type ChatOutbox } from '../attachmentPort.ts'
import fixture from '../subjectReadPort.gateway.fixtures.json'
import { attemptsStore, type IndexedDbOutbox } from './indexedDb.ts'
import { createScopedChatOutbox } from './scoped.ts'
import { createThreadOutboxSender, type ThreadOutboxInput } from './threadSender.ts'

/** Only imported by the browser runner's /__outbox-crash HTML, never by the app. */
export interface OutboxCrashSnapshot {
  readonly record: OutboxRecord<PreparedPacket> | undefined
  readonly packetBytes: number[] | undefined
  readonly intentBytes: number[] | undefined
  readonly fingerprint: string | undefined
  readonly storedEnvelope: unknown
}
export interface OutboxCrashApi {
  readonly ready: Promise<void>
  readonly start: () => void
  readonly snapshot: () => Promise<OutboxCrashSnapshot>
  phase: string
  done: boolean
  result: unknown
  error: string | undefined
}
declare global {
  interface Window { outboxCrash: OutboxCrashApi }
}

const query = new URLSearchParams(location.search)
const flow = query.get('flow') ?? 'chat'
const seam = query.get('seam') ?? 'none'
const key = query.get('key') ?? 'attempt/crash-browser'
const databaseName = query.get('database') ?? 'fractal-outbox-crash-browser'
const recipient = fixture.cases.agent.ref
const threadInput: ThreadOutboxInput = {
  key, type: 'thread.start', parameters: {
    target_id: 'thread/crash-browser', agent_id: recipient, content: 'Original café', title: 'Original title',
    origin: { source_id: 'source/crash-browser', episode: 'episode/crash-browser' },
  },
}

// A parked continuation owns no transaction: each wrapped promise resolved only
// after the real IndexedDB transaction completed. Closing the page destroys it.
const park = async (at: string): Promise<void> => {
  if (seam !== at) return
  api.phase = at
  await Promise.withResolvers<void>().promise
}
const wrapRepository = (repo: IndexedDbOutbox): IndexedDbOutbox => ({
  ...repo,
  accept: async (...args) => {
    const accepted = await repo.accept(...args)
    if (accepted) await park('intent-commit')
    return accepted
  },
  compareAndSwap: async (...args) => {
    const written = await repo.compareAndSwap(...args)
    const next = args[1]
    if (written && next.state === 'pending' && next.phase === 'ready') await park('packet-commit')
    return written
  },
  acquireDispatchPermit: async (...args) => {
    const dispatched = await repo.acquireDispatchPermit(...args)
    await park('dispatch-marker')
    return dispatched
  },
  compareAndSwapWritable: async (...args) => {
    // dispatchOutbox already decoded and correlated a completed receipt. Park
    // before its local outcome write, not before receiving the HTTP response.
    if (args[1].state === 'delivered') await park('receipt')
    return repo.compareAndSwapWritable(...args)
  },
})

const client = new FractalClient({
  baseUrl: location.origin,
  fetchImpl: (input, init) => {
    if (init?.method === 'POST' && new URL(String(input), location.origin).pathname.endsWith('/actions')) {
      // For this seam the runner waits for its HTTP server to receive the body;
      // this phase alone is deliberately not proof of crossing the socket.
      if (seam === 'socket') api.phase = 'socket'
    }
    return fetch(input, init)
  },
})
const owner = createScopedChatOutbox(client, { databaseName, indexedDB: window.indexedDB })
const wrapped: ChatOutbox = {
  repository: async (onDiscovery) => {
    const repo = await owner.repository(onDiscovery)
    return repo === undefined ? undefined : wrapRepository(repo)
  },
}
const chatSend = gatewayMessageSend(client, async () => ({ _tag: 'Success', value: (await client.capabilities()).snapshot.id }), wrapped)
const threadSend = createThreadOutboxSender(client, wrapped)

// Read the actual storage envelope, including its persisted fingerprint. Never
// rebuild a packet, serialize a replacement, or hash a newly prepared closure.
const readStoredEnvelope = (): Promise<unknown> => {
  const { promise, resolve, reject } = Promise.withResolvers<unknown>()
  const opening = window.indexedDB.open(databaseName)
  opening.onerror = () => reject(opening.error)
  opening.onsuccess = () => {
    const db = opening.result
    const tx = db.transaction(attemptsStore, 'readonly')
    const request = tx.objectStore(attemptsStore).get(key)
    let stored: unknown
    request.onsuccess = () => { stored = request.result }
    tx.oncomplete = () => { db.close(); resolve(stored) }
    tx.onabort = () => { db.close(); reject(tx.error) }
  }
  return promise
}
const storedFingerprint = (stored: unknown): string | undefined => {
  if (typeof stored !== 'object' || stored === null || !('fingerprint' in stored)) return undefined
  const fingerprint = stored.fingerprint
  return typeof fingerprint === 'object' && fingerprint !== null && 'sha256' in fingerprint && typeof fingerprint.sha256 === 'string'
    ? fingerprint.sha256 : undefined
}

let started = false
let releaseRecovery: (() => void) | undefined
const initialize = async (): Promise<void> => {
  if (flow !== 'chat' && flow !== 'thread.start') throw new Error(`Unsupported crash fixture flow: ${flow}`)
  if (!['intent-commit', 'packet-commit', 'dispatch-marker', 'socket', 'receipt', 'none'].includes(seam)) throw new Error(`Unsupported crash fixture seam: ${seam}`)
  await owner.initialize()
  // Use the component's real shared store and port. Recovery commands bypass
  // the initial-send seam wrappers, including after the document is reopened.
  const store = recoveryStoreFor(owner.recovery)
  const { promise: listed, resolve, reject } = Promise.withResolvers<void>()
  const unsubscribe = store.subscribe(() => {
    const list = store.snapshot().list
    if (list._tag === 'Checking') return
    unsubscribe()
    if (list._tag === 'Unavailable') reject(new Error('Crash fixture recovery unavailable'))
    else resolve()
  })
  releaseRecovery = store.acquire()
  const rootElement = document.getElementById('root') ?? document.body.appendChild(document.createElement('div'))
  const root = createRoot(rootElement)
  await listed
  flushSync(() => root.render(<OutboxRecovery port={owner.recovery} />))
  api.phase = 'ready'
}
const failed = (cause: unknown): void => {
  api.error = cause instanceof Error ? `${cause.name}: ${cause.message}` : String(cause)
  api.phase = 'error'
  api.done = true
}
const api: OutboxCrashApi = {
  // The microtask ensures the global is visible before initialization proceeds.
  ready: Promise.resolve().then(initialize).catch(cause => { failed(cause); throw cause }),
  start: () => {
    if (started) return
    started = true
    void api.ready.then(async () => {
      api.phase = 'running'
      api.result = flow === 'chat'
        ? await chatSend({ _tag: 'Send', api_version: 'st3.client.v0', type: 'message.send', id: `action/${key}`,
          parameters: { to: recipient, content: 'Original café', tags: ['original'], attachments: [], title: 'Original title' } }, key)
        : await threadSend.send(threadInput)
      api.done = true
      api.phase = 'done'
    }).catch(failed)
  },
  snapshot: async () => {
    await api.ready
    const repo = await owner.repository()
    if (repo === undefined) throw new Error('Crash fixture requires a durable scoped repository')
    const record = await repo.read(key)
    const packetBytes = await repo.readPacketBytes(key)
    const intentBytes = await repo.readIntentBytes(key)
    const storedEnvelope = await readStoredEnvelope()
    return { record, packetBytes: packetBytes === undefined ? undefined : Array.from(packetBytes),
      intentBytes: intentBytes === undefined ? undefined : Array.from(intentBytes),
      fingerprint: storedFingerprint(storedEnvelope), storedEnvelope }
  },
  phase: 'initializing', done: false, result: undefined, error: undefined,
}
window.outboxCrash = api
window.addEventListener('pagehide', () => { releaseRecovery?.(); owner.close() }, { once: true })
