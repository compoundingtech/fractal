import { ClientError, actionReceiptCheck, appliedAnswer, type Capabilities, type EnvelopeOf, type FractalClient, type ThreadReplyParameters, type ThreadStartParameters } from '@fractal/client'
import { createOutboxRecord, prepareOutboxRecord } from '@fractal/client/outbox'
import { expectedPreparedPacketIdentities, prepareThreadReplyPacket, prepareThreadStartPacket, type PreparedPacket, type PreparedThreadReplyPacket, type PreparedThreadStartPacket } from '@fractal/client/prepared-packet'
import * as Native from '@fractal/client/schema/read'
import { Effect, Schema } from 'effect'
import type * as ThreadSchema from '@fractal/client/schema'
import type { ChatOutbox } from '../attachmentPort.ts'
import { retryableImport } from '../lazySdk.ts'
import { dispatchOutbox, requireOutboxScope } from './delivery.ts'
import type { IndexedDbOutbox } from './indexedDb.ts'
import { sameScope } from './repository.ts'
import { scopeFromCapabilities } from './scoped.ts'

export type ThreadOutboxInput =
  | { readonly key: string; readonly type: 'thread.start'; readonly parameters: Omit<ThreadStartParameters, 'signature'> }
  | { readonly key: string; readonly type: 'thread.reply'; readonly parameters: Omit<ThreadReplyParameters, 'signature'> }
export type ThreadOutboxOutcome =
  | { readonly how: 'sent' }
  | { readonly how: 'unconfirmed' }
  | { readonly how: 'failed'; readonly error: unknown }
  | { readonly how: 'not-applied'; readonly error: unknown }
  /** No dispatch began; retain the previous attempt and retry its deterministic child. */
  | { readonly how: 'retry-refused'; readonly error: unknown }
export interface ThreadOutboxSendOptions {
  /** The caller retains an explicit-not-applied attempt, even before repository discovery. */
  readonly retry?: boolean
  /** Required before any restored never-dispatched intent or explicit-none retry can send. */
  readonly revalidate?: () => Promise<void>
  readonly expectedActor?: string | undefined
  /** Recovery commands cannot follow a descendant hidden behind an older rendered item. */
  readonly expectedRevision?: number
}
export interface ThreadOutboxSender {
  readonly send: (input: ThreadOutboxInput, options?: ThreadOutboxSendOptions) => Promise<ThreadOutboxOutcome>
}
type ThreadPacket = PreparedThreadStartPacket | PreparedThreadReplyPacket
// Write contracts must stay outside the read graph, as with chat's lazy schema boundary.
const loadThreadSchema = retryableImport<typeof ThreadSchema>(() => import('@fractal/client/schema'), 'thread send contracts')
const sha256 = async (bytes: Uint8Array): Promise<Uint8Array> => new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes)))
const unconfirmed = { how: 'unconfirmed' } as const

export const loadThreadIntentSchema = async () => {
  const schema = await Effect.runPromise(loadThreadSchema)
  const { signature: _startSignature, ...startFields } = schema.ThreadStartParameters.fields
  const { signature: _replySignature, ...replyFields } = schema.ThreadReplyParameters.fields
  return Schema.fromJsonString(Schema.Union([
    Schema.Struct({ key: Schema.String, type: Schema.Literal('thread.start'), parameters: Schema.Struct(startFields) }),
    Schema.Struct({ key: Schema.String, type: Schema.Literal('thread.reply'), parameters: Schema.Struct(replyFields) }),
  ]))
}

/** Recovery and mounted composers validate the immutable original destination, never the current pane. */
export const revalidateThreadOutboxTarget = async (client: FractalClient, input: ThreadOutboxInput): Promise<void> => {
  if (input.type === 'thread.start') {
    const { source_id, episode } = input.parameters.origin
    const context = Native.decodeUnknownSync(Native.ThreadStartContext)((await client.threadStartContext(source_id, episode)).value)
    if (context.source_id !== source_id || context.episode !== episode
      || context.start_recipient.agent_id !== input.parameters.agent_id) {
      throw new Error('Review the current recipient before sending this saved thread. Nothing was sent.')
    }
  } else {
    try {
      const current = Native.decodeUnknownSync(Native.ThreadSummary)((await client.threadGet(input.parameters.target_id)).value)
      if (current.id !== input.parameters.target_id) throw new Error('The original thread could not be verified. Nothing was sent.')
      if (current.state === 'closed') throw new Error('This thread is closed. Nothing was sent.')
    } catch (error) {
      if (error instanceof ClientError) throw new Error(error.response.code === 'not-found'
        ? 'Fractal no longer has this thread. Nothing was sent.' : 'The original thread could not be verified. Nothing was sent.')
      throw error
    }
  }
}

/** Full intent is committed before fence discovery; explicit-none retries prepare a fresh attempt. */
export const createThreadOutboxSender = (client: FractalClient, outbox: ChatOutbox): ThreadOutboxSender => {
  const session = new Map<string, { packet: ThreadPacket; actor: string; state: 'sending' | 'none' | 'unknown' | 'sent' }>()
  const protectedRepositories = new Map<string, IndexedDbOutbox>()
  const retryableKeys = new Set<string>()
  const observedPredecessors = new Map<string, { id: string; revision: number }>()
  return { send: async (input, options) => {
    // Pin the generation before repository discovery, capability checks, or target revalidation.
    const predecessor = observedPredecessors.get(input.key)
    const invokedSession = session.get(input.key)
    const invokedSessionState = invokedSession?.state
    let dispatched = false
    let retryIntent = options?.retry === true || retryableKeys.has(input.key) || session.get(input.key)?.state === 'none'
    try {
      input = structuredClone(input)
      let discovered: EnvelopeOf<Capabilities> | undefined
      const selected = protectedRepositories.get(input.key) ?? await outbox.repository(response => { discovered = response })
      const repo = protectedRepositories.get(input.key) ?? selected
      if (repo !== undefined && session.has(input.key)) {
        throw new Error('The connection identity changed. Return to the original connection to retry this action.')
      }
      if (repo !== undefined) protectedRepositories.set(input.key, repo)
      if (repo !== undefined && options?.expectedActor !== undefined && repo.scope.principalId !== options.expectedActor) {
        throw new Error('Return to the original account to send this action.')
      }
      const schema = await Effect.runPromise(loadThreadSchema)
      const intentSchema = await loadThreadIntentSchema()
      const prepare = (intent: typeof intentSchema.Type, snapshotId: string): ThreadPacket => {
        const common = { id: `action/${intent.key}`, idempotency_key: intent.key, fence: { snapshot_id: snapshotId, subject_revisions: {} } }
        const packet = intent.type === 'thread.start'
          ? prepareThreadStartPacket({ ...common, parameters: intent.parameters })
          : prepareThreadReplyPacket({ ...common, parameters: intent.parameters })
        Native.decodeUnknownSync(schema.ActionRequest)(packet)
        return packet
      }
      const capabilities = async () => {
        const response = discovered ?? await client.capabilities()
        discovered = undefined
        const value = Native.decodeUnknownSync(Native.Capabilities)(response.value)
        if (options?.expectedActor !== undefined && value.session_actor !== options.expectedActor) {
          throw new Error('Return to the original account to send this action.')
        }
        if (!value.capabilities.some(grant => grant.id === input.type && grant.state === 'granted')
          || !(value.transport === 'local-web' || value.capabilities.some(grant => grant.id === 'thread-browser-send' && grant.version >= 1 && grant.state === 'granted'))) {
          throw new Error('This connection cannot perform that thread action.')
        }
        return { response, value }
      }
      if (repo === undefined) {
        const { response, value } = await capabilities()
        let retained = session.get(input.key)
        if (retained !== undefined && retained.actor !== value.session_actor) throw new Error('Return to the original account to retry this action.')
        if (retained !== invokedSession || invokedSessionState !== 'none' || retained?.state !== 'none') {
          if (retained !== undefined) return retained.state === 'sent' ? { how: 'sent' }
            : retained.state === 'none' ? { how: 'not-applied', error: new Error('The retained thread attempt was explicitly not applied.') } : unconfirmed
        }
        if (retained !== undefined) {
          const previous = retained
          previous.state = 'sending'
          try {
            if (options?.revalidate === undefined) throw new Error('Revalidate the original thread target before retrying.')
            await options.revalidate()
            const fresh = await capabilities()
            if (fresh.value.session_actor !== previous.actor) throw new Error('Return to the original account to retry this action.')
            const intent = Schema.decodeUnknownSync(intentSchema)(JSON.stringify({
              key: `${previous.packet.idempotency_key}/retry`, type: previous.packet.type, parameters: previous.packet.parameters,
            }))
            retained = { packet: prepare(intent, fresh.response.snapshot.id), actor: previous.actor, state: 'sending' }
            session.set(input.key, retained)
          } catch (error) {
            previous.state = 'none'
            return { how: 'retry-refused', error }
          }
        } else {
          const intent = Schema.decodeUnknownSync(intentSchema)(JSON.stringify(input))
          retained = { packet: prepare(intent, response.snapshot.id), actor: value.session_actor, state: 'sending' }
          session.set(input.key, retained)
        }
        dispatched = true
        try {
          const packet = retained.packet
          const result = await actionReceiptCheck({ kind: 'message', actionId: packet.id, targetId: packet.parameters.target_id })(() => client.submitAction(packet))
          Native.decodeUnknownSync(schema.ActionResult)(result.value)
          retained.state = 'sent'
          retryableKeys.delete(input.key)
          return { how: 'sent' }
        } catch (error) {
          retained.state = appliedAnswer(error) === 'none' ? 'none' : 'unknown'
          retryableKeys.delete(input.key)
          if (retained.state === 'none') retryableKeys.add(input.key)
          return retained.state === 'none' ? { how: 'not-applied', error } : unconfirmed
        }
      }
      let record = await repo.read(predecessor?.id ?? input.key)
      if (options?.expectedRevision !== undefined && record?.revision !== options.expectedRevision) return unconfirmed
      const restored = record !== undefined
      if (record?.state === 'not-applied') {
        retryIntent = true
        retryableKeys.add(input.key)
      }
      if (restored) await requireOutboxScope(client, repo)
      let retryPacket: ThreadPacket | undefined
      let pinnedPredecessor = predecessor
      if (record?.state === 'not-applied') {
        if (record.packet.type === 'message.send') throw new Error('The saved action is not a thread send.')
        retryPacket = record.packet
        pinnedPredecessor ??= { id: record.id, revision: record.revision }
        if (options?.expectedRevision !== undefined && await repo.hasAttempt(`${pinnedPredecessor.id}/retry`)) return unconfirmed
        const child = await repo.read(`${pinnedPredecessor.id}/retry`)
        if (child !== undefined && options?.expectedRevision !== undefined) return unconfirmed
        if (child?.state === 'not-applied') {
          observedPredecessors.set(input.key, { id: child.id, revision: child.revision })
          return { how: 'not-applied', error: new Error('The retained thread retry was explicitly not applied.') }
        }
        if (child !== undefined) record = child
      }
      if (record === undefined && predecessor !== undefined) return unconfirmed
      if (record !== undefined && record.state !== 'not-applied' && (record.mayHaveDispatched || record.state !== 'pending')) {
        return record.state === 'delivered' ? { how: 'sent' } : unconfirmed
      }
      let intent: typeof intentSchema.Type
      if (record === undefined || record.state === 'not-applied') {
        const id = pinnedPredecessor === undefined ? input.key : `${pinnedPredecessor.id}/retry`
        intent = Schema.decodeUnknownSync(intentSchema)(JSON.stringify(retryPacket === undefined ? input : {
          key: id, type: retryPacket.type, parameters: retryPacket.parameters,
        }))
        const fresh = createOutboxRecord(id, { text: intent.parameters.content, attachments: [] })
        if (!await repo.accept(fresh, [], new TextEncoder().encode(Schema.encodeSync(intentSchema)(intent)), pinnedPredecessor)) return unconfirmed
        record = fresh
      } else {
        const bytes = await repo.readIntentBytes(record.id)
        if (bytes === undefined) throw new Error('The saved thread intent is unavailable.')
        intent = Schema.decodeUnknownSync(intentSchema)(new TextDecoder().decode(bytes))
      }
      if (intent.type !== input.type) throw new Error('The saved action has a different thread action type.')
      if (retryPacket !== undefined || restored) {
        if (options?.revalidate === undefined) throw new Error(retryPacket !== undefined
          ? 'Revalidate the original thread target before retrying.'
          : 'Revalidate the saved thread target before sending.')
        await options.revalidate()
        discovered = undefined
      }
      const { response, value } = await capabilities()
      const preparingScope = scopeFromCapabilities(response)
      if (preparingScope === undefined || !sameScope(repo.scope, preparingScope)) {
        throw new Error('Return to the original account and deployment to send this action.')
      }
      if (record.state === 'pending' && record.phase === 'preparing') {
        const packet = prepare(intent, response.snapshot.id)
        const expected = await expectedPreparedPacketIdentities(packet, repo.scope.principalId, sha256)
        const prepared = prepareOutboxRecord<PreparedPacket>(record, packet, expected)
        if (prepared === undefined || !await repo.compareAndSwap(record.revision, prepared)) return unconfirmed
        record = prepared
      }
      if (record === undefined || !('packet' in record) || record.packet.type === 'message.send') return unconfirmed
      // Keep the actor from the real capabilities answer bound to preparation and dispatch.
      if (value.session_actor !== repo.scope.principalId) throw new Error('Return to the original account to send this action.')
      const result = await dispatchOutbox(client, repo, record, {
        kind: 'message', actionId: record.packet.id, targetId: record.packet.parameters.target_id,
      }, Native.decodeUnknownSync(schema.ActionResult))
      retryableKeys.delete(input.key)
      if (result.how === 'not-applied') {
        retryableKeys.add(input.key)
        const observed = await repo.read(record.id)
        if (observed?.state === 'not-applied') observedPredecessors.set(input.key, { id: observed.id, revision: observed.revision })
      }
      return result.how === 'sent' ? { how: 'sent' } : result
    } catch (error) { return dispatched ? unconfirmed : { how: retryIntent ? 'retry-refused' : 'failed', error } }
  } }
}
