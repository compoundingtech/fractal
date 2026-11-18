import type { Capabilities, EnvelopeOf, FractalClient } from '@fractal/client'
import { outboxDeploymentId, restoreOutboxEntries, type OutboxScope } from '@fractal/client/outbox'
import * as Native from '@fractal/client/schema/read'
import { Effect } from 'effect'
import type { ChatOutbox } from '../attachmentPort.ts'
import type { OutboxRecoveryItem, OutboxRecoveryPort } from '../source.ts'
import { openIndexedDbOutbox, openIndexedDbOutboxScopes, type IndexedDbOutbox, type IndexedDbOutboxScopes, type OpenOutboxOptions } from './indexedDb.ts'
import { OutboxConflict, OutboxIdentityUnavailable, sameScope } from './repository.ts'
import { CorruptRecoveryIntent, createRecoveryCommands, readRecoveryIntent } from './recovery.ts'

/** One canonical identity formula shared by wire extraction and cached observation. */
const decodedScope = (capabilities: typeof Native.Capabilities.Type, memberId: string): OutboxScope | undefined => {
  if (capabilities.issuer === undefined || capabilities.issuer_key === undefined) return undefined
  return {
    principalId: capabilities.session_actor,
    deploymentId: outboxDeploymentId(capabilities.issuer, capabilities.issuer_key),
    memberId,
  }
}

/** Decode the wire answer before using its generated issuer fields as identity. */
export const scopeFromCapabilities = (response: EnvelopeOf<Capabilities>): OutboxScope | undefined =>
  decodedScope(Native.decodeUnknownSync(Native.Capabilities)(response.value), response.snapshot.host_id)

export interface ScopedChatOutbox extends ChatOutbox {
  readonly beginObservation: () => number
  readonly initialize: (response?: EnvelopeOf<Capabilities>, sequence?: number) => Promise<void>
  readonly observe: (response: EnvelopeOf<Capabilities>, sequence?: number) => typeof Native.Capabilities.Type
  readonly isCurrentIdentity: (response: EnvelopeOf<Capabilities>) => boolean
  readonly recovery: OutboxRecoveryPort
  readonly protection: () => 'durable' | 'unprotected'
  readonly close: () => void
}

/** A changed identity requires another fresh read; equal-identity recovery reads do not starve grants. */
export const readOutboxAccess = async (client: FractalClient, outbox: ScopedChatOutbox, signal: AbortSignal) => {
  while (true) {
    signal.throwIfAborted()
    const sequence = outbox.beginObservation()
    const response = await client.capabilities({ signal })
    signal.throwIfAborted()
    const capabilities = outbox.observe(response, sequence)
    let storageAvailable = true
    try { await outbox.initialize(response, sequence) }
    catch { storageAvailable = false }
    signal.throwIfAborted()
    if (outbox.isCurrentIdentity(response)) return { capabilities, storageAvailable }
    // Only a genuinely newer, different identity can reach the next iteration.
  }
}

/** Each connection keeps its original tuple. New identity observations never rebind old attempts. */
export const createScopedChatOutbox = (
  client: FractalClient,
  options: Pick<OpenOutboxOptions, 'indexedDB' | 'storage' | 'databaseName'> = {},
): ScopedChatOutbox => {
  type Identity = { readonly scope: OutboxScope | undefined; readonly response: EnvelopeOf<Capabilities>; readonly sequence: number }
  let cached: Identity | undefined
  let sequence = 0
  const beginObservation = () => ++sequence
  const observations = new WeakMap<EnvelopeOf<Capabilities>, number>()
  let discovering: Promise<Identity> | undefined
  let closed = false
  const listeners = new Set<() => void>()
  const channelName = `${options.databaseName ?? 'fractal-outbox'}/recovery`
  let channel: BroadcastChannel | undefined
  // Neither the payload nor its sender has authority. Consumers must read IndexedDB again.
  const changed = () => { queueMicrotask(() => { for (const listener of listeners) listener() }) }
  const written = () => {
    if (closed) return
    changed()
    if (typeof BroadcastChannel === 'undefined') return
    try {
      const sender = channel ?? new BroadcastChannel(channelName)
      try { sender.postMessage('changed') }
      finally { if (sender !== channel) sender.close() }
    } catch { /* Wakeups are advisory and cannot change the result of a durable write. */ }
  }
  const notifying = (repo: IndexedDbOutbox): IndexedDbOutbox => ({
    ...repo,
    accept: async (...args) => { const result = await repo.accept(...args); if (result) written(); return result },
    commit: async (...args) => { const result = await repo.commit(...args); if (result) written(); return result },
    compareAndSwap: async (...args) => { const result = await repo.compareAndSwap(...args); if (result) written(); return result },
    compareAndSwapWritable: async (...args) => { const result = await repo.compareAndSwapWritable(...args); if (result._tag === 'written') written(); return result },
    acquireDispatchPermit: async (...args) => { const result = await repo.acquireDispatchPermit(...args); written(); return result },
    dismiss: async (...args) => { await repo.dismiss(...args); written() },
    recover: async () => { await repo.recover(); written() },
    cleanupTerminal: async () => { await repo.cleanupTerminal(); written() },
  })
  const connections = new Map<string, Promise<IndexedDbOutbox>>()
  const initialized = new Map<string, Promise<IndexedDbOutbox>>()
  let catalog: Promise<IndexedDbOutboxScopes> | undefined
  const persistedScopes = (): Promise<IndexedDbOutboxScopes> => {
    if (closed) return Promise.reject(new OutboxIdentityUnavailable())
    if (catalog === undefined) {
      const opening = Effect.runPromise(openIndexedDbOutboxScopes(options))
      catalog = opening
      void opening.catch(() => { if (catalog === opening) catalog = undefined })
    }
    return catalog
  }
  const observe: ScopedChatOutbox['observe'] = (response, started = observations.get(response) ?? beginObservation()) => {
    const capabilities = Native.decodeUnknownSync(Native.Capabilities)(response.value)
    observations.set(response, started)
    if (cached === undefined || started >= cached.sequence) {
      cached = { scope: decodedScope(capabilities, response.snapshot.host_id), response, sequence: started }
    }
    return capabilities
  }
  const isCurrentIdentity = (response: EnvelopeOf<Capabilities>) => {
    if (cached === undefined) return false
    const capabilities = Native.decodeUnknownSync(Native.Capabilities)(response.value)
    const current = Native.decodeUnknownSync(Native.Capabilities)(cached.response.value)
    const scope = decodedScope(capabilities, response.snapshot.host_id)
    return capabilities.session_actor === current.session_actor
      && response.snapshot.host_id === cached.response.snapshot.host_id
      && (scope === undefined ? cached.scope === undefined : cached.scope !== undefined && sameScope(scope, cached.scope))
  }
  const discover = async (): Promise<Identity> => {
    if (discovering !== undefined) return discovering
    const started = beginObservation()
    discovering = (async () => {
      try {
        const response = await client.capabilities()
        observe(response, started)
        if (cached === undefined) throw new OutboxIdentityUnavailable()
        return cached
      } catch { throw new OutboxIdentityUnavailable() }
      finally { discovering = undefined }
    })()
    return discovering
  }
  const connectionFor = (scope: OutboxScope): Promise<IndexedDbOutbox> => {
    if (closed) return Promise.reject(new OutboxIdentityUnavailable())
    const key = JSON.stringify([scope.principalId, scope.deploymentId, scope.memberId])
    let connection = connections.get(key)
    if (connection === undefined) {
      connection = Effect.runPromise(openIndexedDbOutbox({ ...options, scope, recoverOnOpen: false })).then(notifying)
      connections.set(key, connection)
      void connection.catch(() => { if (connections.get(key) === connection) connections.delete(key) })
    }
    return connection
  }
  // Recovery belongs to this connection's startup, not capability refresh or list polling.
  // Foreign scopes may be opened for inspection without acquiring this send barrier.
  const initializeConnection = (scope: OutboxScope, currentIdentity?: Identity): Promise<IndexedDbOutbox> => {
    const key = JSON.stringify([scope.principalId, scope.deploymentId, scope.memberId])
    let initialization = initialized.get(key)
    if (initialization === undefined) {
      initialization = (async () => {
        const repo = await connectionFor(scope)
        const current = currentIdentity ?? await discover()
        if (current.scope === undefined || !sameScope(scope, current.scope)
          || cached?.scope === undefined || !sameScope(scope, cached.scope)) throw new OutboxIdentityUnavailable()
        await repo.recover()
        // Validate persisted envelopes without treating discard-only corruption as unavailability.
        await repo.list()
        return repo
      })()
      initialized.set(key, initialization)
      void initialization.catch(() => { if (initialized.get(key) === initialization) initialized.delete(key) })
    }
    return initialization
  }
  const initialize: ScopedChatOutbox['initialize'] = async (response, started) => {
    let identity: Identity
    if (response === undefined) identity = await discover()
    else {
      try {
        observe(response, started)
        if (cached === undefined || !isCurrentIdentity(response)) return
        // Join only the current identity's barrier, never initialize an obsolete tuple.
        identity = cached
      } catch { throw new OutboxIdentityUnavailable() }
    }
    // A successful answer without an issuer explicitly opts out of durable protection.
    if (identity.scope !== undefined) await initializeConnection(identity.scope, identity)
  }
  const repository: ScopedChatOutbox['repository'] = async (onDiscovery) => {
    const needsDiscovery = cached === undefined
    const identity = cached ?? await discover()
    const repo = identity.scope === undefined ? undefined : await initializeConnection(identity.scope, needsDiscovery ? identity : undefined)
    // Share only this operation's discovery, never stale cached grants.
    if (needsDiscovery) onDiscovery?.(identity.response)
    return repo
  }
  const locate = async (id: string, expectedRevision: number | undefined) => {
    const current = await discover()
    if (current.scope === undefined) throw new OutboxIdentityUnavailable()
    const repo = await initializeConnection(current.scope, current)
    const entry = (await repo.list()).find(entry => entry._tag === 'record' && entry.record.id === id)
    const record = entry?._tag === 'record' ? entry.record : undefined
    if (record === undefined) throw new OutboxConflict({ localAttemptId: id, expectedRevision })
    return { repo, record }
  }
  return {
    beginObservation, initialize, observe, isCurrentIdentity, repository,
    protection: () => cached?.scope === undefined ? 'unprotected' : 'durable',
    recovery: {
      ...createRecoveryCommands(client, locate),
      subscribe: listener => {
        if (closed) return () => undefined
        listeners.add(listener)
        if (channel === undefined && typeof BroadcastChannel !== 'undefined') {
          try { channel = new BroadcastChannel(channelName); channel.onmessage = changed }
          catch { /* Local subscriptions still work when cross-tab notifications are unavailable. */ }
        }
        return () => {
          listeners.delete(listener)
          if (listeners.size === 0) { channel?.close(); channel = undefined }
        }
      },
      list: async () => {
        // Initial identity discovery also supplies this operation's current comparison.
        const initial = cached === undefined ? await discover() : undefined
        const persisted = await (await persistedScopes()).list()
        const current = initial ?? await discover()
        const items = new Map<string, OutboxRecoveryItem>()
        for (const entry of restoreOutboxEntries(persisted.unscoped)) {
          if (entry._tag === 'corrupt') items.set(entry.id, { id: entry.id, revision: entry.revision, kind: 'corrupt', subject: 'message', status: entry.status, actions: ['discard'] })
        }
        for (const scope of persisted.scopes) {
          const matches = current.scope !== undefined && sameScope(scope, current.scope)
          const repo = await (matches ? initializeConnection(scope, current) : connectionFor(scope))
          // UI certainty may be unknown for a dispatch-marked pending row, but command
          // authority must retain the persisted revision, not a synthetic restoration.
          for (const entry of await repo.list()) {
            if (entry._tag === 'corrupt') items.set(entry.id, { id: entry.id, revision: entry.revision, kind: 'corrupt', subject: 'message', status: 'storage damaged; delivery unknown', actions: ['discard'] })
            else {
              const record = entry.record
              if (record.state === 'delivered') continue
              // Display only the newest persisted child; stale parent commands never follow it.
              if (record.state === 'not-applied' && await repo.hasAttempt(`${record.id}/retry`)) continue
              const subject = 'packet' in record && record.packet.type !== 'message.send' ? 'thread' : 'message'
              if (!matches) {
                items.set(record.id, { id: record.id, revision: record.revision, kind: 'identity-mismatch', subject,
                  status: 'Identity changed; delivery unknown. Return to the original account and deployment.', actions: ['discard'] })
                continue
              }
              try {
                const intent = await readRecoveryIntent({ repo, record })
                const kind = record.state === 'not-applied' ? 'not-applied' : record.state === 'pending' && !record.mayHaveDispatched ? 'pending' : 'unknown'
                items.set(record.id, { id: record.id, revision: record.revision, kind, subject: intent.type === 'message.send' ? 'message' : 'thread', text: intent.parameters.content,
                  status: kind === 'unknown' ? 'Fractal has not confirmed this message/thread. Do not send it again.'
                    : kind === 'pending' ? 'This saved message has not been sent. Revalidate the original recipient before sending.'
                    : 'Fractal confirmed this attempt was not applied. Retry creates a fresh attempt.',
                  actions: kind === 'unknown' ? ['copy', 'discard'] : kind === 'pending' ? ['send', 'discard'] : ['retry', 'discard'] })
              } catch (cause) {
                if (!(cause instanceof CorruptRecoveryIntent)) throw cause
                items.set(record.id, { id: record.id, revision: record.revision, kind: 'corrupt', subject,
                  status: 'This saved message cannot be read safely. It can only be discarded.', actions: ['discard'] })
              }
            }
          }
        }
        return [...items.values()]
      },
      discard: async (id, expectedRevision) => {
        const scopes = await persistedScopes()
        const persisted = await scopes.list()
        await scopes.discardUnscoped(id, expectedRevision)
        for (const scope of persisted.scopes) {
          const repo = await connectionFor(scope)
          const entries = await repo.list()
          if (entries.some((entry) => (entry._tag === 'record' ? entry.record.id : entry.id) === id)) await repo.dismiss(id, expectedRevision)
        }
        written()
      },
    },
    close: () => {
      closed = true
      channel?.close()
      listeners.clear()
      for (const connection of connections.values()) void connection.then(repo => repo.close(), () => undefined)
      void catalog?.then(scopes => scopes.close(), () => undefined)
    },
  }
}
