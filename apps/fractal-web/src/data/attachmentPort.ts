import { ClientError, appliedAnswer, actionReceiptCheck, type Capabilities, type EnvelopeOf, type FractalClient } from '@fractal/client'
import { createOutboxRecord, prepareOutboxRecord, type OutboxRecord, type OutboxIntent } from '@fractal/client/outbox'
import { expectedPreparedPacketIdentities, prepareMessageSendPacket, type PreparedPacket, type PreparedMessageSendPacket } from '@fractal/client/prepared-packet'
import type { ActionResult } from '@fractal/client/schema'
import * as Native from '@fractal/client/schema/read'
import { Effect, Schema } from 'effect'

import { conversationPortFailure, hasConversationCapability } from './conversationPort.ts'
import { loadAttachmentSchema } from './lazySdk.ts'
import type { AttachmentPort, AttachmentSendRequest, ConversationPortResult } from './source.ts'
import { spoolAttachment, type IndexedDbOutbox } from './outbox/indexedDb.ts'
import { OutboxIdentityUnavailable, sameScope, type SpoolInput } from './outbox/repository.ts'
import { scopeFromCapabilities } from './outbox/scoped.ts'
import { dispatchOutbox, OutboxScopeMismatch } from './outbox/delivery.ts'

/** Scope selection is local; this callback must not make send-work network requests. */
export interface ChatOutbox {
  /** Undefined means explicitly unprotected identity mode, never a storage-error fallback. */
  readonly repository: (onDiscovery?: (response: EnvelopeOf<Capabilities>) => void) => Promise<IndexedDbOutbox | undefined>
}

/** Upload, chunk reads and sending use the same gateway client, never a synthetic delivery. */
export const gatewayAttachments = (
  client: FractalClient,
  snapshot: (fresh?: boolean) => Promise<ConversationPortResult<string>>,
  outbox: ChatOutbox = { repository: async () => undefined },
): AttachmentPort => {
  const send = gatewayMessageSend(client, snapshot, outbox)
  return {
  capabilities: async () => {
    try {
      const capabilities = Native.decodeUnknownSync(Native.Capabilities)((await client.capabilities()).value)
      return {
        _tag: 'Success',
        value: {
          upload: hasConversationCapability(capabilities, 'control.messages') ? 'granted' : 'ungranted',
          send: hasConversationCapability(capabilities, 'message.send') ? 'granted' : 'ungranted',
          read: hasConversationCapability(capabilities, 'read.projections') ? 'granted' : 'ungranted',
        },
      }
    } catch (cause) {
      return conversationPortFailure(cause)
    }
  },
  upload: async ({ bytes, mediaType, signal }) => {
    signal?.throwIfAborted()
    try {
      const response = await client.capabilities({ signal })
      signal?.throwIfAborted()
      const capabilities = Native.decodeUnknownSync(Native.Capabilities)(response.value)
      if (!hasConversationCapability(capabilities, 'control.messages')) {
        return { _tag: 'Refused', reason: 'ungranted', detail: 'This device is not granted attachment upload.' }
      }
      const { BlobUpload } = await Effect.runPromise(loadAttachmentSchema)
      signal?.throwIfAborted()
      const uploaded = await client.uploadBlob(bytes, mediaType, { signal })
      signal?.throwIfAborted()
      return { _tag: 'Success', value: Native.decodeUnknownSync(BlobUpload)(uploaded.value) }
    } catch (cause) {
      signal?.throwIfAborted()
      if (cause instanceof Error && cause.name === 'AbortError') throw cause
      return conversationPortFailure(cause)
    }
  },
  chunk: async ({ sha256, ...options }) => {
    try {
      const capabilities = Native.decodeUnknownSync(Native.Capabilities)((await client.capabilities()).value)
      if (!hasConversationCapability(capabilities, 'read.projections')) {
        return { _tag: 'Refused', reason: 'ungranted', detail: 'This device is not granted attachment read access.' }
      }
      const response = await client.blobChunk(sha256, options)
      return { _tag: 'Success', value: Native.decodeUnknownSync(Native.BlobChunk)(response.value) }
    } catch (cause) {
      return conversationPortFailure(cause)
    }
  },
  send,
  }
}

const unconfirmed = (): ConversationPortResult<never> => ({
  _tag: 'Refused', reason: 'unconfirmed',
  detail: 'Fractal has not confirmed this message. Do not send it again.',
})
const retryNotApplied = (): ConversationPortResult<never> => ({
  _tag: 'Refused', reason: 'retry-refused',
  detail: 'The existing retry was not applied. Choose Retry again to make a new attempt.',
})
const storageRefusal = (): ConversationPortResult<never> => ({
  _tag: 'Refused', reason: 'storage', detail: 'This browser could not save this message safely. Nothing was sent.',
})
const preparationBlocked = {
  _tag: 'Refused', reason: 'failed',
  detail: 'The message is saved on this device, but preparation could not be completed.',
} as const
const utf8 = new TextEncoder()
const text = new TextDecoder()

/** Explicit legacy mode for capability answers without a complete issuer pair. Never selected on failure. */
const sendUnprotected = async (
  client: FractalClient, snapshot: (fresh?: boolean) => Promise<ConversationPortResult<string>>,
  input: AttachmentSendRequest, key: string, unappliedIntents: Map<string, PreparedMessageSendPacket['parameters']>,
  principals: Map<string, string>,
  retained: PreparedMessageSendPacket['parameters'] | undefined,
  discovered?: EnvelopeOf<Capabilities>,
  onPrepared?: (messageIds: readonly string[]) => void,
  onDispatched?: () => void,
): Promise<ConversationPortResult<ActionResult>> => {
  if (input._tag === 'Resend' && (retained === undefined || unappliedIntents.get(key) !== retained)) {
    return unappliedIntents.has(key) ? retryNotApplied() : unconfirmed()
  }
  // Consume the retry permission synchronously, before preparation can yield.
  unappliedIntents.delete(key)
  let packet: PreparedMessageSendPacket | undefined
  let dispatched = false
  try {
    const schema = await Effect.runPromise(loadAttachmentSchema)
    const decodeParameters = Native.decodeUnknownSync(Schema.Struct({
      content: Schema.String,
      attachments: Schema.Array(schema.AttachmentInput).check(Schema.isMaxLength(4)),
    }))
    let parameters
    try {
      const decoded = decodeParameters(retained ?? input.parameters)
      const source = retained ?? input.parameters
      parameters = { ...source, tags: [...source.tags ?? []], attachments: [...decoded.attachments ?? []] }
    }
    catch { return { _tag: 'Refused', reason: 'invalid', detail: 'The message does not match the supported send format.' } }
    const images = retained === undefined ? input.images ?? [] : []
    if ((parameters.attachments?.length ?? 0) + images.length > 4) return { _tag: 'Refused', reason: 'invalid', detail: 'A message supports at most four images.' }
    // Text acquires its fence first; images acquire it only after their uploads.
    const textSnapshot = images.length === 0 ? await snapshot(input._tag === 'Resend') : undefined
    if (textSnapshot?._tag === 'Refused') return textSnapshot
    const capabilities = Native.decodeUnknownSync(Native.Capabilities)((discovered ?? await client.capabilities()).value)
    const principal = principals.get(key)
    if (principal !== undefined && principal !== capabilities.session_actor) return {
      _tag: 'Refused', reason: 'ungranted', detail: 'Return to the original account to send this message.',
    }
    principals.set(key, capabilities.session_actor)
    if (!hasConversationCapability(capabilities, 'message.send')) return { _tag: 'Refused', reason: 'ungranted', detail: 'This device is not granted message send.' }
    const attachments = [...parameters.attachments ?? []]
    for (const image of images) {
      if (!hasConversationCapability(capabilities, 'control.messages')) return { _tag: 'Refused', reason: 'ungranted', detail: 'This device is not granted attachment upload.' }
      const uploaded = Native.decodeUnknownSync(schema.BlobUpload)((await client.uploadBlob(image.bytes, image.mediaType)).value)
      attachments.push({ blob: uploaded.blob, media_type: uploaded.media_type, name: image.name })
    }
    const current = textSnapshot ?? await snapshot(input._tag === 'Resend')
    if (current._tag === 'Refused') return current
    packet = prepareMessageSendPacket({ id: retained === undefined ? input.id : `action/${crypto.randomUUID()}`,
      idempotency_key: retained === undefined ? key : crypto.randomUUID(),
      fence: { snapshot_id: current.value, subject_revisions: {} }, parameters: { ...parameters, attachments } })
    try { Native.decodeUnknownSync(schema.ActionRequest)(packet) }
    catch { return { _tag: 'Refused', reason: 'invalid', detail: 'The message could not be prepared in the supported send format.' } }
    const identities = await expectedPreparedPacketIdentities(packet, capabilities.session_actor,
      async (bytes) => new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes))))
    onPrepared?.(identities)
    dispatched = true
    onDispatched?.()
    const prepared = packet
    const response = await actionReceiptCheck({ kind: 'message', actionId: prepared.id, messageId: identities[0] })(() => client.submitAction(prepared))
    return { _tag: 'Success', value: Native.decodeUnknownSync(schema.ActionResult)(response.value) }
  } catch (cause) {
    if (dispatched && appliedAnswer(cause) !== 'none') return cause instanceof ClientError
      ? { _tag: 'Refused', reason: 'unconfirmed', detail: 'Fractal has not confirmed this message. Do not send it again.', error: cause.response }
      : unconfirmed()
    if (dispatched && packet !== undefined && appliedAnswer(cause) === 'none') unappliedIntents.set(key, packet.parameters)
    return cause instanceof ClientError && cause.response.code === 'stale-fence'
      ? { _tag: 'Refused', reason: 'stale-fence', detail: cause.response.message, error: cause.response }
      : conversationPortFailure(cause)
  } finally {
    if (!dispatched && retained !== undefined && !unappliedIntents.has(key)) unappliedIntents.set(key, retained)
  }
}

/** Both entry paths share acceptance, preparation, dispatch permit and durable outcomes. */
export const gatewayMessageSend = (
  client: FractalClient,
  snapshot: (fresh?: boolean) => Promise<ConversationPortResult<string>>,
  outbox: ChatOutbox = { repository: async () => undefined },
) => {
  // Only proven-unapplied immutable intent grants a deliberate new attempt.
  const unappliedIntents = new Map<string, PreparedMessageSendPacket['parameters']>()
  const scopes = new Map<string, IndexedDbOutbox['scope'] | 'unprotected'>()
  const observed = new Map<string, Pick<OutboxRecord<PreparedPacket>, 'id' | 'revision'>>()
  const principals = new Map<string, string>()
  return async (input: AttachmentSendRequest, chosenKey?: string, onPrepared?: (messageIds: readonly string[]) => void): Promise<ConversationPortResult<ActionResult>> => {
  const rootKey = input._tag === 'Resend' ? input.idempotencyKey : chosenKey ?? crypto.randomUUID()
  // This invocation owns only the generation visible at the synchronous user action.
  const predecessor = input._tag === 'Resend' ? observed.get(rootKey) : undefined
  const retained = input._tag === 'Resend' ? unappliedIntents.get(rootKey) : undefined
  let dispatched = false
  const attempt = async (): Promise<ConversationPortResult<ActionResult>> => {
  let key = predecessor?.id ?? rootKey
  let repo: IndexedDbOutbox | undefined
  let discoveredIdentity: EnvelopeOf<Capabilities> | undefined
  try { repo = await outbox.repository(response => { discoveredIdentity = response }) } catch (cause) {
    return cause instanceof OutboxIdentityUnavailable
      ? { _tag: 'Refused', reason: 'identity-unavailable', detail: 'Identity unavailable, retry later. Nothing was sent.' }
      : storageRefusal()
  }
  const selectedScope = repo?.scope ?? 'unprotected'
  const originalScope = scopes.get(rootKey)
  if (originalScope !== undefined && (originalScope === 'unprotected'
    ? selectedScope !== 'unprotected'
    : selectedScope === 'unprotected' || !sameScope(originalScope, selectedScope))) {
    return { _tag: 'Refused', reason: 'ungranted', detail: 'Return to the original account and deployment to send this message.' }
  }
  scopes.set(rootKey, selectedScope)
  if (repo === undefined) return sendUnprotected(client, snapshot, input, rootKey, unappliedIntents, principals, retained, discoveredIdentity, onPrepared,
    () => { dispatched = true })
  let record: OutboxRecord<PreparedPacket> | undefined
  try { record = await repo.read(key) } catch { return storageRefusal() }
  const pinnedPredecessor = predecessor ?? (input._tag === 'Resend' && record?.state === 'not-applied'
    ? { id: record.id, revision: record.revision } : undefined)
  if (record !== undefined) {
    try {
      const current = scopeFromCapabilities(await client.capabilities())
      if (current === undefined || !sameScope(repo.scope, current)) return {
        _tag: 'Refused', reason: 'ungranted', detail: 'Return to the original account and deployment to inspect or send this message.',
      }
    } catch { return { _tag: 'Refused', reason: 'identity-unavailable', detail: 'Identity unavailable, retry later. Nothing was sent.' } }
  }
  if (input._tag === 'Resend' && record?.state === 'not-applied') {
    try {
      const child = await repo.read(`${key}/retry`)
      if (child !== undefined) {
        key = child.id
        record = child
        if (child.state === 'not-applied') {
          observed.set(rootKey, { id: child.id, revision: child.revision })
          return retryNotApplied()
        }
      }
    } catch { return storageRefusal() }
  }
  if (record?.state === 'not-applied' && input._tag !== 'Resend') return unconfirmed()
  // A retained result is authoritative. No retry map and no replay of dispatched bytes.
  if (record !== undefined && record.state !== 'not-applied' && (record.mayHaveDispatched || record.state !== 'pending')) {
    if (record.state === 'delivered' && record.evidence.kind === 'receipt') {
      const schema = await Effect.runPromise(loadAttachmentSchema)
      return { _tag: 'Success', value: Native.decodeUnknownSync(schema.ActionResult)(record.evidence.receipt) }
    }
    return unconfirmed()
  }
  if (input._tag === 'Resend' && record === undefined) return unconfirmed()
  const schema = await Effect.runPromise(loadAttachmentSchema)
  // Acceptance validates only immutable intent content and attachment references. The real
  // action (including its target and fence) is validated later, after a real snapshot exists.
  const decodeParameters = Native.decodeUnknownSync(Schema.Struct({
    content: Schema.String,
    attachments: Schema.Array(schema.AttachmentInput).check(Schema.isMaxLength(4)),
  }))
  const Parameters = Schema.declare((value: unknown): value is PreparedMessageSendPacket['parameters'] => {
    try { decodeParameters(value); return true }
    catch { return false }
  })
  const intentSchema = Schema.fromJsonString(Schema.Struct({
    api_version: Schema.Literal('st3.client.v0'), type: Schema.Literal('message.send'),
    id: Schema.String, parameters: Parameters,
  }))
  let original: typeof intentSchema.Type
  try {
    if (record === undefined) {
      const decoded = decodeParameters(input.parameters)
      const parameters = { ...input.parameters, tags: [...input.parameters.tags], attachments: [...decoded.attachments ?? []] }
      if ((parameters.attachments?.length ?? 0) + (input.images?.length ?? 0) > 4) {
        return { _tag: 'Refused', reason: 'invalid', detail: 'A message supports at most four images.' }
      }
      original = { api_version: input.api_version, type: input.type, id: input.id, parameters }
      const spool: SpoolInput[] = []
      const attachments: OutboxIntent['attachments'][number][] = []
      for (const image of input.images ?? []) {
        if (!['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(image.mediaType)) {
          return { _tag: 'Refused', reason: 'invalid', detail: 'Only PNG, JPEG, GIF and WebP images can be attached.' }
        }
        const bytes = image.bytes instanceof Uint8Array ? image.bytes
          : image.bytes instanceof ArrayBuffer ? new Uint8Array(image.bytes) : new Uint8Array(await image.bytes.arrayBuffer())
        if (bytes.length > 10 * 1024 * 1024) return { _tag: 'Refused', reason: 'invalid', detail: 'Images must be 10 MiB or smaller.' }
        const item = await Effect.runPromise(spoolAttachment(bytes))
        if (!spool.some((saved) => saved.hash === item.hash)) spool.push(item)
        attachments.push({ contentHash: item.hash, name: image.name, mediaType: image.mediaType, reference: `${key}/${item.hash}` })
      }
      record = createOutboxRecord(key, { text: parameters.content, attachments })
      try {
        if (!await repo.accept(record, spool, utf8.encode(Schema.encodeSync(intentSchema)(original)))) return storageRefusal()
      } catch { return storageRefusal() }
    } else {
      const bytes = await repo.readIntentBytes(key)
      if (bytes === undefined) return preparationBlocked
      original = Schema.decodeUnknownSync(intentSchema)(text.decode(bytes))
      if (input._tag === 'Resend' && record.state === 'not-applied' && pinnedPredecessor !== undefined) {
        const predecessor = record
        const childKey = `${key}/retry`
        const spool: SpoolInput[] = []
        for (const image of predecessor.intent.attachments) {
          if (spool.some(item => item.hash === image.contentHash)) continue
          const bytes = await repo.readSpool(key, image.contentHash)
          if (bytes === undefined) return preparationBlocked
          spool.push({ hash: image.contentHash, bytes })
        }
        const child = createOutboxRecord(childKey, {
          ...predecessor.intent,
          attachments: predecessor.intent.attachments.map(image => ({ ...image, reference: `${childKey}/${image.contentHash}` })),
        })
        const childOriginal = { ...original, id: `action/${crypto.randomUUID()}` }
        try {
          if (await repo.accept(child, spool, utf8.encode(Schema.encodeSync(intentSchema)(childOriginal)),
            pinnedPredecessor)) {
            record = child
            original = childOriginal
          } else {
            record = await repo.read(childKey)
            const acceptedBytes = await repo.readIntentBytes(childKey)
            if (record === undefined || acceptedBytes === undefined) return unconfirmed()
            original = Schema.decodeUnknownSync(intentSchema)(text.decode(acceptedBytes))
          }
        } catch { return storageRefusal() }
        key = childKey
        if (record.state === 'not-applied') {
          observed.set(rootKey, { id: record.id, revision: record.revision })
          return retryNotApplied()
        }
        if (record.state !== 'pending' || record.mayHaveDispatched) return unconfirmed()
      }
    }
  } catch { return { _tag: 'Refused', reason: 'invalid', detail: 'The message does not match the supported send format.' } }
  if (record === undefined) return storageRefusal()
  let currentRecord: OutboxRecord<PreparedPacket> = record
  try {
    // No send-work request occurs until acceptance and its attachment spool commit above.
    const preparingText = currentRecord.state === 'pending' && currentRecord.phase === 'preparing'
      && currentRecord.intent.attachments.length === 0
    const textSnapshot = preparingText ? await snapshot(input._tag === 'Resend') : undefined
    if (textSnapshot?._tag === 'Refused') return textSnapshot
    const discovered = discoveredIdentity ?? await client.capabilities()
    const capabilities = Native.decodeUnknownSync(Native.Capabilities)(discovered.value)
    const currentScope = scopeFromCapabilities(discovered)
    if (currentScope === undefined || !sameScope(repo.scope, currentScope)) {
      return { _tag: 'Refused', reason: 'ungranted', detail: 'Return to the original account and deployment to send this message.' }
    }
    if (!hasConversationCapability(capabilities, 'message.send')
      || currentRecord.intent.attachments.length > 0 && !hasConversationCapability(capabilities, 'control.messages')) {
      return { _tag: 'Refused', reason: 'ungranted', detail: 'This device is not granted this message send.' }
    }
    if (currentRecord.state === 'pending' && currentRecord.phase === 'preparing') {
      const attachments = [...original.parameters.attachments ?? []]
      for (const image of currentRecord.intent.attachments) {
        const bytes = await repo.readSpool(key, image.contentHash)
        if (bytes === undefined) return preparationBlocked
        const mediaType = Schema.decodeUnknownSync(Schema.Literals(['image/png', 'image/jpeg', 'image/gif', 'image/webp']))(image.mediaType)
        const uploaded = Native.decodeUnknownSync(schema.BlobUpload)((await client.uploadBlob(bytes, mediaType)).value)
        attachments.push({ blob: uploaded.blob, media_type: uploaded.media_type, ...(image.name === undefined ? {} : { name: image.name }) })
      }
      const current = textSnapshot ?? await snapshot(input._tag === 'Resend')
      if (current._tag === 'Refused') return current
      const packet = prepareMessageSendPacket({
        id: original.id, idempotency_key: key, fence: { snapshot_id: current.value, subject_revisions: {} },
        parameters: { ...original.parameters, attachments },
      })
      try { Native.decodeUnknownSync(schema.ActionRequest)(packet) }
      catch { return preparationBlocked }
      const expected = await expectedPreparedPacketIdentities(packet, repo.scope.principalId,
        async (bytes) => new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes))))
      const prepared = prepareOutboxRecord(currentRecord, packet, expected)
      if (prepared === undefined) return unconfirmed()
      try { if (!await repo.compareAndSwap(currentRecord.revision, prepared)) return unconfirmed() }
      catch { return preparationBlocked }
      currentRecord = prepared
    }
    if (!('packet' in currentRecord) || currentRecord.packet.type !== 'message.send') return unconfirmed()
    onPrepared?.(currentRecord.expectedIdentities)
    const outcome = await dispatchOutbox(client, repo, currentRecord, {
      kind: 'message', actionId: currentRecord.packet.id, messageId: currentRecord.expectedIdentities[0],
    }, Native.decodeUnknownSync(schema.ActionResult))
    dispatched = true
    if (outcome.how === 'sent') return { _tag: 'Success', value: outcome.receipt }
    if (outcome.how === 'unconfirmed') return unconfirmed()
    // Retain the exact durable revision proven unapplied, not a later descendant.
    try {
      const failed = await repo.read(key)
      if (failed?.state === 'not-applied') observed.set(rootKey, { id: failed.id, revision: failed.revision })
    } catch { /* The refusal remains authoritative, but no new retry permission is inferred. */ }
    const cause = outcome.error
    return cause instanceof ClientError && cause.response.code === 'stale-fence'
      ? { _tag: 'Refused', reason: 'stale-fence', detail: cause.response.message, error: cause.response }
      : conversationPortFailure(cause)
  } catch (cause) {
    if (cause instanceof OutboxScopeMismatch) return {
      _tag: 'Refused', reason: 'ungranted', detail: 'Return to the original account and deployment to send this message.',
    }
    return currentRecord.mayHaveDispatched ? unconfirmed() : conversationPortFailure(cause)
  }
  }
  const result = await attempt().catch(cause => dispatched ? unconfirmed() : conversationPortFailure(cause))
  // A refused retry has not relinquished its original intent or optimistic owner.
  // Durable children and unprotected retained intent remain available to Resend.
  return input._tag === 'Resend' && !dispatched && result._tag === 'Refused' && result.reason !== 'unconfirmed'
    ? { ...result, reason: 'retry-refused' }
    : result
}
}
