import type { FractalClient } from '@fractal/client'
import type { OutboxRecord } from '@fractal/client/outbox'
import type { PreparedPacket, PreparedMessageSendPacket } from '@fractal/client/prepared-packet'
import * as Native from '@fractal/client/schema/read'
import { Effect, Option, Schema } from 'effect'
import { gatewayMessageSend, type ChatOutbox } from '../attachmentPort.ts'
import { loadAttachmentSchema } from '../lazySdk.ts'
import type { OutboxCopyOutcome, OutboxRecoveryPort, SendImage } from '../source.ts'
import type { IndexedDbOutbox } from './indexedDb.ts'
import { requireOutboxScope } from './delivery.ts'
import { createThreadOutboxSender, loadThreadIntentSchema, revalidateThreadOutboxTarget, type ThreadOutboxInput } from './threadSender.ts'

type Located = { readonly repo: IndexedDbOutbox; readonly record: OutboxRecord<PreparedPacket> }
type RecoveryIntent = { readonly api_version: 'st3.client.v0'; readonly type: 'message.send'; readonly id: string; readonly parameters: PreparedMessageSendPacket['parameters'] } | ThreadOutboxInput
type Restored = Located & { readonly intent: RecoveryIntent }
type CopyDraft = { readonly sourceId: string; readonly repo: IndexedDbOutbox; pending?: Promise<OutboxCopyOutcome> }
const decoder = new TextDecoder()
export class CorruptRecoveryIntent extends Error {}

/** Only readable malformed/missing content is corrupt. Storage and lazy-import errors remain unavailable. */
export const readRecoveryIntent = async ({ repo, record }: Located): Promise<RecoveryIntent> => {
  const bytes = await repo.readIntentBytes(record.id)
  if (bytes === undefined) throw new CorruptRecoveryIntent('The saved message content is unavailable. Discard this saved item.')
  const raw = decoder.decode(bytes)
  let kind: { readonly type: string }
  try { kind = Schema.decodeUnknownSync(Schema.fromJsonString(Schema.Struct({ type: Schema.String })))(raw) }
  catch { throw new CorruptRecoveryIntent('The saved message intent is damaged.') }
  if (kind.type === 'thread.start' || kind.type === 'thread.reply') {
    const schema = await loadThreadIntentSchema()
    try {
      const intent = Schema.decodeUnknownSync(schema)(raw)
      if (intent.key !== record.id) throw new CorruptRecoveryIntent('The saved thread intent has a different identity.')
      return intent
    } catch { throw new CorruptRecoveryIntent('The saved thread intent is damaged.') }
  }
  const schema = await Effect.runPromise(loadAttachmentSchema)
  const message = schema.ActionRequest.members.find(member => Schema.is(member.fields.type)('message.send'))
  if (message === undefined) throw new Error('The supported message intent schema is unavailable.')
  // Select the generated parameter schema, not a synthetic action with a fabricated fence.
  const parameters = Schema.declare((value: unknown): value is PreparedMessageSendPacket['parameters'] => {
    try { Schema.decodeUnknownSync(message.fields.parameters)(value); return true }
    catch { return false }
  })
  try {
    return Schema.decodeUnknownSync(Schema.fromJsonString(
      Schema.Struct({ api_version: Schema.Literal('st3.client.v0'), type: Schema.Literal('message.send'), id: Schema.String, parameters }),
    ))(raw)
  } catch { throw new CorruptRecoveryIntent('The saved message intent is damaged.') }
}

export const createRecoveryCommands = (client: FractalClient, locate: (id: string) => Promise<Located>): Pick<OutboxRecoveryPort, 'copy' | 'sendCopy' | 'retry' | 'send'> => {
  const copies = new Map<string, CopyDraft>()
  const original = async (id: string): Promise<Restored> => {
    const found = await locate(id)
    await requireOutboxScope(client, found.repo)
    return { ...found, intent: await readRecoveryIntent(found) }
  }
  const images = async ({ repo, record }: Located): Promise<readonly SendImage[]> => Promise.all(record.intent.attachments.map(async image => {
    const bytes = await repo.readSpool(record.id, image.contentHash)
    if (bytes === undefined) throw new Error('A saved image is unavailable. This message cannot be copied or sent without it.')
    const mediaType = Schema.decodeUnknownSync(Schema.Literals(['image/png', 'image/jpeg', 'image/gif', 'image/webp']))(image.mediaType)
    return { bytes, mediaType, name: image.name ?? 'Saved image' }
  }))
  const validate = async (saved: Restored) => {
    await requireOutboxScope(client, saved.repo)
    if (saved.intent.type === 'message.send') {
      const agent = Native.decodeUnknownSync(Native.Agent)((await client.agentsGet(saved.intent.parameters.to)).value)
      if (agent.id !== saved.intent.parameters.to) throw new Error('The original recipient could not be verified. Nothing was sent.')
      if (saved.intent.parameters.session_id !== undefined && Option.getOrUndefined(agent.current_session_id) !== saved.intent.parameters.session_id) {
        throw new Error('The original recipient session is no longer current. Nothing was sent.')
      }
    } else await revalidateThreadOutboxTarget(client, saved.intent)
  }
  const run = async (id: string, operation: 'pending' | 'not-applied', copy?: { readonly key: string; readonly content: string }) => {
    const saved = await original(id)
    const { repo, record, intent } = saved
    if (copy === undefined && (record.state !== operation || operation === 'pending' && record.mayHaveDispatched)) {
      throw new Error('This saved attempt cannot be sent again. Refresh its recovery state.')
    }
    if (copy === undefined && await repo.hasAttempt(`${id}/retry`)) throw new Error('A newer attempt exists. Review its recovery state before continuing.')
    if (copy !== undefined && !(record.state === 'unknown' || record.state === 'pending' && record.mayHaveDispatched)) throw new Error('Only an unconfirmed message can be copied into a new attempt.')
    await validate(saved)
    const outbox: ChatOutbox = { repository: async () => { await requireOutboxScope(client, repo); return repo } }
    if (intent.type === 'message.send') {
      const send = gatewayMessageSend(client, async () => ({ _tag: 'Success', value: (await client.capabilities()).snapshot.id }), outbox)
      const request = { ...intent, parameters: { ...intent.parameters, tags: [...intent.parameters.tags ?? []], attachments: [...intent.parameters.attachments ?? []],
        ...(copy === undefined ? {} : { content: copy.content }) } }
      const result = copy !== undefined
        ? await send({ ...request, id: `action/${copy.key}`, _tag: 'Send', images: await images(saved) }, copy.key)
        : await send({ ...request, _tag: 'Resend', idempotencyKey: id }, undefined, undefined, { expectedRevision: record.revision })
      if (result._tag !== 'Success') throw new Error(result.detail)
    } else {
      const input: ThreadOutboxInput = copy === undefined ? { ...intent, key: id }
        : intent.type === 'thread.start' ? { ...intent, key: copy.key, parameters: { ...intent.parameters, content: copy.content, target_id: `thread/${copy.key}` } }
        : { ...intent, key: copy.key, parameters: { ...intent.parameters, content: copy.content } }
      const result = await createThreadOutboxSender(client, outbox).send(input, { expectedActor: repo.scope.principalId,
        retry: operation === 'not-applied', ...(copy === undefined ? { expectedRevision: record.revision } : {}), revalidate: () => validate(saved) })
      if (result.how !== 'sent') throw new Error('error' in result && result.error instanceof Error ? result.error.message
        : 'Fractal has not confirmed this message/thread. Do not send it again.')
    }
  }
  return {
    send: id => run(id, 'pending'),
    retry: id => run(id, 'not-applied'),
    copy: async id => {
      const saved = await original(id)
      if (!(saved.record.state === 'unknown' || saved.record.state === 'pending' && saved.record.mayHaveDispatched)) throw new Error('Only an unconfirmed message can be copied.')
      const retained = await images(saved)
      const copyKey = crypto.randomUUID()
      copies.set(copyKey, { sourceId: id, repo: saved.repo })
      return { copyKey, text: saved.intent.parameters.content, images: [
        ...retained.map(image => ({ name: image.name, mediaType: image.mediaType, bytes: image.bytes instanceof Uint8Array ? image.bytes.byteLength : 0 })),
        ...(saved.intent.type === 'message.send' ? (saved.intent.parameters.attachments ?? []).map(image => ({ name: image.name ?? 'Attached image', mediaType: image.media_type, bytes: 0 })) : []),
      ] }
    },
    sendCopy: (id, copyKey, content) => {
      const draft = copies.get(copyKey)
      if (draft === undefined || draft.sourceId !== id) return Promise.resolve({ _tag: 'Refused', detail: 'Open a new copy composer before sending.' })
      if (draft.pending !== undefined) return draft.pending
      // Concurrent calls share the first result. Recorded is reported only after durable acceptance.
      const pending = (async (): Promise<OutboxCopyOutcome> => {
        await requireOutboxScope(client, draft.repo)
        if (await draft.repo.hasAttempt(copyKey)) return { _tag: 'Recorded' }
        try { await run(id, 'pending', { key: copyKey, content }) }
        catch (cause) {
          // Even unknown, explicit-none and preparation-failed copies now have their own recovery row.
          await requireOutboxScope(client, draft.repo)
          if (await draft.repo.hasAttempt(copyKey)) return { _tag: 'Recorded' }
          return { _tag: 'Refused', detail: cause instanceof Error ? cause.message : 'The copied message was not saved or sent.' }
        }
        return { _tag: 'Recorded' }
      })()
      draft.pending = pending
      void pending.finally(() => { if (draft.pending === pending) delete draft.pending }).catch(() => undefined)
      return pending
    },
  }
}
