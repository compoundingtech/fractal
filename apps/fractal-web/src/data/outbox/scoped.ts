import { outboxDeploymentId, restoreOutboxEntries, type Capabilities, type EnvelopeOf, type FractalClient, type OutboxScope } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { Effect } from 'effect'
import type { ChatOutbox } from '../attachmentPort.ts'
import type { OutboxRecoveryItem, OutboxRecoveryPort } from '../source.ts'
import { openIndexedDbOutbox, type IndexedDbOutbox, type OpenOutboxOptions } from './indexedDb.ts'
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
  let cached: { readonly scope: OutboxScope | undefined } | undefined
  let discovering: Promise<{ readonly scope: OutboxScope | undefined }> | undefined
  const connections = new Map<string, Promise<IndexedDbOutbox>>()
  const observe = (response: EnvelopeOf<Capabilities>): typeof Native.Capabilities.Type => {
    const capabilities = Native.decodeUnknownSync(Native.Capabilities)(response.value)
    cached = { scope: decodedScope(capabilities, response.snapshot.host_id) }
    return capabilities
  }
  const discover = async (): Promise<{ readonly scope: OutboxScope | undefined }> => {
    if (discovering !== undefined) return discovering
    discovering = (async () => {
      try {
        const response = await client.capabilities()
        observe(response)
        return { scope: cached?.scope }
      } catch { throw new OutboxIdentityUnavailable() }
      finally { discovering = undefined }
    })()
    return discovering
  }
  const repository = async (): Promise<IndexedDbOutbox | undefined> => {
    const identity = cached ?? await discover()
    if (identity.scope === undefined) return undefined
    const scope = identity.scope
    const key = JSON.stringify([scope.principalId, scope.deploymentId, scope.memberId])
    let connection = connections.get(key)
    if (connection === undefined) {
      connection = Effect.runPromise(openIndexedDbOutbox({ ...options, scope, recoverOnOpen: false }))
      connections.set(key, connection)
      void connection.catch(() => { if (connections.get(key) === connection) connections.delete(key) })
    }
    return connection
  }
  return {
    observe, repository,
    protection: () => cached?.scope === undefined ? 'unprotected' : 'durable',
    recovery: {
      list: async () => {
        // Initial identity discovery also supplies this operation's current comparison.
        const initial = cached === undefined ? await discover() : undefined
        await repository()
        const current = initial ?? await discover()
        const items: OutboxRecoveryItem[] = []
        for (const connection of connections.values()) {
          const repo = await connection
          const matches = current.scope !== undefined && sameScope(repo.scope, current.scope)
          if (matches) await repo.recover()
          for (const entry of restoreOutboxEntries(await repo.list())) {
            if (entry._tag === 'corrupt') items.push({ id: entry.id, status: entry.status, actions: entry.actions })
            else items.push({
              id: entry.record.id,
              status: matches ? `Saved message: ${entry.record.state}.` : 'Identity changed; delivery unknown. Return to the original account and deployment.',
              actions: ['discard'],
            })
          }
        }
        return items
      },
      discard: async (id) => {
        for (const connection of connections.values()) {
          const repo = await connection
          const entries = await repo.list()
          if (entries.some((entry) => (entry._tag === 'record' ? entry.record.id : entry.id) === id)) await repo.dismiss(id)
        }
      },
    },
    close: () => { for (const connection of connections.values()) void connection.then(repo => repo.close(), () => undefined) },
  }
}
