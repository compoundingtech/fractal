import type { Capabilities, EnvelopeOf, FractalClient } from '@fractal/client'
import { outboxDeploymentId, restoreOutboxEntries, type OutboxScope } from '@fractal/client/outbox'
import * as Native from '@fractal/client/schema/read'
import { Effect } from 'effect'
import type { ChatOutbox } from '../attachmentPort.ts'
import type { OutboxRecoveryItem, OutboxRecoveryPort } from '../source.ts'
import { openIndexedDbOutbox, openIndexedDbOutboxScopes, type IndexedDbOutbox, type IndexedDbOutboxScopes, type OpenOutboxOptions } from './indexedDb.ts'
import { OutboxIdentityUnavailable, sameScope } from './repository.ts'

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
  readonly observe: (response: EnvelopeOf<Capabilities>) => typeof Native.Capabilities.Type
  readonly recovery: OutboxRecoveryPort
  readonly protection: () => 'durable' | 'unprotected'
  readonly close: () => void
}

/** Each connection keeps its original tuple. New identity observations never rebind old attempts. */
export const createScopedChatOutbox = (
  client: FractalClient,
  options: Pick<OpenOutboxOptions, 'indexedDB' | 'storage' | 'databaseName'> = {},
): ScopedChatOutbox => {
  type Identity = { readonly scope: OutboxScope | undefined; readonly response: EnvelopeOf<Capabilities> }
  let cached: Identity | undefined
  let discovering: Promise<Identity> | undefined
  const connections = new Map<string, Promise<IndexedDbOutbox>>()
  let catalog: Promise<IndexedDbOutboxScopes> | undefined
  const persistedScopes = (): Promise<IndexedDbOutboxScopes> => {
    if (catalog === undefined) {
      const opening = Effect.runPromise(openIndexedDbOutboxScopes(options))
      catalog = opening
      void opening.catch(() => { if (catalog === opening) catalog = undefined })
    }
    return catalog
  }
  const observe = (response: EnvelopeOf<Capabilities>): typeof Native.Capabilities.Type => {
    const capabilities = Native.decodeUnknownSync(Native.Capabilities)(response.value)
    cached = { scope: decodedScope(capabilities, response.snapshot.host_id), response }
    return capabilities
  }
  const discover = async (): Promise<Identity> => {
    if (discovering !== undefined) return discovering
    discovering = (async () => {
      try {
        const response = await client.capabilities()
        const capabilities = observe(response)
        return { scope: decodedScope(capabilities, response.snapshot.host_id), response }
      } catch { throw new OutboxIdentityUnavailable() }
      finally { discovering = undefined }
    })()
    return discovering
  }
  const connectionFor = (scope: OutboxScope): Promise<IndexedDbOutbox> => {
    const key = JSON.stringify([scope.principalId, scope.deploymentId, scope.memberId])
    let connection = connections.get(key)
    if (connection === undefined) {
      connection = Effect.runPromise(openIndexedDbOutbox({ ...options, scope, recoverOnOpen: false }))
      connections.set(key, connection)
      void connection.catch(() => { if (connections.get(key) === connection) connections.delete(key) })
    }
    return connection
  }
  const repository: ScopedChatOutbox['repository'] = async (onDiscovery) => {
    const needsDiscovery = cached === undefined
    const identity = cached ?? await discover()
    // Share only this operation's discovery, never stale cached grants.
    if (needsDiscovery) onDiscovery?.(identity.response)
    return identity.scope === undefined ? undefined : connectionFor(identity.scope)
  }
  return {
    observe, repository,
    protection: () => cached?.scope === undefined ? 'unprotected' : 'durable',
    recovery: {
      list: async () => {
        // Initial identity discovery also supplies this operation's current comparison.
        const initial = cached === undefined ? await discover() : undefined
        const persisted = await (await persistedScopes()).list()
        const current = initial ?? await discover()
        const items = new Map<string, OutboxRecoveryItem>()
        for (const entry of restoreOutboxEntries(persisted.unscoped)) {
          if (entry._tag === 'corrupt') items.set(entry.id, { id: entry.id, status: entry.status, actions: entry.actions })
        }
        for (const scope of persisted.scopes) {
          const repo = await connectionFor(scope)
          const matches = current.scope !== undefined && sameScope(scope, current.scope)
          if (matches) await repo.recover()
          for (const entry of restoreOutboxEntries(await repo.list())) {
            if (entry._tag === 'corrupt') items.set(entry.id, { id: entry.id, status: entry.status, actions: entry.actions })
            else items.set(entry.record.id, {
              id: entry.record.id,
              status: matches ? `Saved message: ${entry.record.state}.` : 'Identity changed; delivery unknown. Return to the original account and deployment.',
              actions: ['discard'],
            })
          }
        }
        return [...items.values()]
      },
      discard: async (id) => {
        const scopes = await persistedScopes()
        const persisted = await scopes.list()
        await scopes.discardUnscoped(id)
        for (const scope of persisted.scopes) {
          const repo = await connectionFor(scope)
          const entries = await repo.list()
          if (entries.some((entry) => (entry._tag === 'record' ? entry.record.id : entry.id) === id)) await repo.dismiss(id)
        }
      },
    },
    close: () => {
      for (const connection of connections.values()) void connection.then(repo => repo.close(), () => undefined)
      void catalog?.then(scopes => scopes.close(), () => undefined)
    },
  }
}
