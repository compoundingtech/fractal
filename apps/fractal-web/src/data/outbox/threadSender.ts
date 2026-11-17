import { actionReceiptCheck, appliedAnswer, createOutboxRecord, expectedPreparedPacketIdentities, prepareOutboxRecord, prepareThreadReplyPacket, prepareThreadStartPacket, type Capabilities, type EnvelopeOf, type FractalClient, type PreparedPacket, type PreparedThreadReplyPacket, type PreparedThreadStartPacket, type ThreadReplyParameters, type ThreadStartParameters } from '@fractal/client'
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
export interface ThreadOutboxSendOptions {
  readonly beforeFirstDispatch?: () => Promise<void>
  readonly expectedActor?: string | undefined
}
export interface ThreadOutboxSender {
  readonly send: (input: ThreadOutboxInput, options?: ThreadOutboxSendOptions) => Promise<ThreadOutboxOutcome>
}
type ThreadPacket = PreparedThreadStartPacket | PreparedThreadReplyPacket
// Write contracts must stay outside the read graph, as with chat's lazy schema boundary.
const loadThreadSchema = retryableImport<typeof ThreadSchema>(() => import('@fractal/client/schema'), 'thread send contracts')
const sha256 = async (bytes: Uint8Array): Promise<Uint8Array> => new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes)))
const unconfirmed = { how: 'unconfirmed' } as const

/** Full intent is committed before fence discovery; retries retain the original complete packet. */
export const createThreadOutboxSender = (client: FractalClient, outbox: ChatOutbox): ThreadOutboxSender => {
  const session = new Map<string, { packet: ThreadPacket; actor: string; state: 'sending' | 'none' | 'unknown' | 'sent' }>()
  const protectedRepositories = new Map<string, IndexedDbOutbox>()
  return { send: async (input, options) => {
    let dispatched = false
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
      const { signature: _startSignature, ...startFields } = schema.ThreadStartParameters.fields
      const { signature: _replySignature, ...replyFields } = schema.ThreadReplyParameters.fields
      const intentSchema = Schema.fromJsonString(Schema.Union([
        Schema.Struct({ key: Schema.String, type: Schema.Literal('thread.start'), parameters: Schema.Struct(startFields) }),
        Schema.Struct({ key: Schema.String, type: Schema.Literal('thread.reply'), parameters: Schema.Struct(replyFields) }),
      ]))
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
        if (retained !== undefined && retained.state !== 'none') return retained.state === 'sent' ? { how: 'sent' } : unconfirmed
        if (retained === undefined) {
          const intent = Schema.decodeUnknownSync(intentSchema)(JSON.stringify(input))
          retained = { packet: prepare(intent, response.snapshot.id), actor: value.session_actor, state: 'sending' }
          session.set(input.key, retained)
        }
        retained.state = 'sending'
        dispatched = true
        try {
          const packet = retained.packet
          const result = await actionReceiptCheck({ kind: 'message', actionId: packet.id, targetId: packet.parameters.target_id })(() => client.submitAction(packet))
          Native.decodeUnknownSync(schema.ActionResult)(result.value)
          retained.state = 'sent'
          return { how: 'sent' }
        } catch (error) {
          retained.state = appliedAnswer(error) === 'none' ? 'none' : 'unknown'
          return retained.state === 'none' ? { how: 'not-applied', error } : unconfirmed
        }
      }
      let record = await repo.read(input.key)
      const restored = record !== undefined
      if (restored) await requireOutboxScope(client, repo)
      let retryPacket: ThreadPacket | undefined
      // Every retry has its own single-use permit. Concurrent tabs converge on the same child.
      while (record?.state === 'not-applied') {
        if (record.packet.type === 'message.send') throw new Error('The saved action is not a thread send.')
        retryPacket = record.packet
        const child = await repo.read(`${record.id}/retry`)
        if (child === undefined) break
        record = child
      }
      if (record !== undefined && record.state !== 'not-applied' && (record.mayHaveDispatched || record.state !== 'pending')) {
        return record.state === 'delivered' ? { how: 'sent' } : unconfirmed
      }
      let intent: typeof intentSchema.Type
      if (record === undefined || record.state === 'not-applied') {
        intent = Schema.decodeUnknownSync(intentSchema)(JSON.stringify(retryPacket === undefined ? input : {
          key: retryPacket.idempotency_key, type: retryPacket.type, parameters: retryPacket.parameters,
        }))
        const id = record === undefined ? input.key : `${record.id}/retry`
        const fresh = createOutboxRecord(id, { text: retryPacket?.parameters.content ?? intent.parameters.content, attachments: [] })
        const predecessor = record?.state === 'not-applied' ? { id: record.id, revision: record.revision } : undefined
        if (!await repo.accept(fresh, [], new TextEncoder().encode(Schema.encodeSync(intentSchema)(intent)), predecessor)) return unconfirmed
        record = fresh
      } else {
        const bytes = await repo.readIntentBytes(record.id)
        if (bytes === undefined) throw new Error('The saved thread intent is unavailable.')
        intent = Schema.decodeUnknownSync(intentSchema)(new TextDecoder().decode(bytes))
      }
      if (intent.type !== input.type) throw new Error('The saved action has a different thread action type.')
      const { response, value } = await capabilities()
      const preparingScope = scopeFromCapabilities(response)
      if (preparingScope === undefined || !sameScope(repo.scope, preparingScope)) {
        throw new Error('Return to the original account and deployment to send this action.')
      }
      if (record.state === 'pending' && record.phase === 'preparing') {
        const packet = retryPacket ?? prepare(intent, response.snapshot.id)
        const expected = await expectedPreparedPacketIdentities(packet, repo.scope.principalId, sha256)
        const prepared = prepareOutboxRecord<PreparedPacket>(record, packet, expected)
        if (prepared === undefined || !await repo.compareAndSwap(record.revision, prepared)) return unconfirmed
        record = prepared
      }
      if (record === undefined || !('packet' in record) || record.packet.type === 'message.send') return unconfirmed
      if (restored && retryPacket === undefined && record.packet.type === 'thread.start') {
        if (options?.beforeFirstDispatch === undefined) throw new Error('Revalidate the saved Start recipient before sending.')
        await options.beforeFirstDispatch()
      }
      // Keep the actor from the real capabilities answer bound to preparation and dispatch.
      if (value.session_actor !== repo.scope.principalId) throw new Error('Return to the original account to send this action.')
      const result = await dispatchOutbox(client, repo, record, {
        kind: 'message', actionId: record.packet.id, targetId: record.packet.parameters.target_id,
      }, Native.decodeUnknownSync(schema.ActionResult))
      return result.how === 'sent' ? { how: 'sent' } : result
    } catch (error) { return dispatched ? unconfirmed : { how: 'failed', error } }
  } }
}
