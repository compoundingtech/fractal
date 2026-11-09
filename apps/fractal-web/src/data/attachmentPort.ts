import { ClientError, ActionRejected, actionReceiptCheck, sendBoundAction, resendBoundAction, type BoundAction, type FractalClient } from '@fractal/client'
import type { ActionResult, AttachmentInput } from '@fractal/client/schema'
import * as Native from '@fractal/client/schema/read'
import { Effect } from 'effect'

import { conversationPortFailure, hasConversationCapability } from './conversationPort.ts'
import { loadAttachmentSchema } from './lazySdk.ts'
import type { AttachmentPort, ConversationPortResult, MessageSendAction } from './source.ts'

/** Upload, chunk reads and sending use the same gateway client, never a synthetic delivery. */
export const gatewayAttachments = (
  client: FractalClient,
  snapshot: () => Promise<ConversationPortResult<string>>,
): AttachmentPort => ({
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
  send: async (request) => {
    const current = await snapshot()
    if (current._tag === 'Refused') return current
    return gatewayMessageSend(client)({
      api_version: request.api_version,
      type: request.type,
      id: request.id,
      fence: { snapshot_id: current.value, subject_revisions: {} },
      parameters: request.parameters,
      idempotency_key: request._tag === 'Resend' ? request.idempotencyKey : crypto.randomUUID(),
    })
  },
})

/**
 * Submit one message action whose idempotency key the caller already chose. Optimistic
 * senders derive the message identity from that key, so it must be single-sourced.
 */
export const gatewayMessageSend = (client: FractalClient) => {
  // The outbox owns presentation and hydration; delivery owns the captured immutable action.
  // Retain its original transport after ambiguity, including when permission later disappears.
  type Attempt = { retry?: BoundAction<MessageSendAction>; receipt?: ActionResult; error?: ClientError }
  const unconfirmed = new Map<string, Attempt>()
  return async (input: MessageSendAction): Promise<ConversationPortResult<ActionResult>> => {
    const request = structuredClone(input)
    let schema
    try {
      schema = await Effect.runPromise(loadAttachmentSchema)
    } catch (cause) {
      return conversationPortFailure(cause)
    }
    let attachments: readonly AttachmentInput[]
    try {
      const action = Native.decodeUnknownSync(schema.ActionRequest)(request)
      if (action.type !== 'message.send') {
        return { _tag: 'Refused', reason: 'invalid', detail: 'Attachment sending requires a message.send action.' }
      }
      attachments = action.parameters.attachments ?? []
    } catch (cause) {
      return { _tag: 'Refused', reason: 'invalid', detail: cause instanceof Error ? cause.message : String(cause) }
    }
    const previous = unconfirmed.get(request.idempotency_key)
    if (previous?.retry !== undefined && JSON.stringify({ id: previous.retry.request.id, parameters: previous.retry.request.parameters })
      !== JSON.stringify({ id: request.id, parameters: request.parameters })) {
      return { _tag: 'Refused', reason: 'invalid', detail: 'Retry the original message before changing this action.' }
    }
    const attempt = previous ?? {}
    try {
      const capabilities = Native.decodeUnknownSync(Native.Capabilities)((await client.capabilities()).value)
      if (!hasConversationCapability(capabilities, 'message.send') && previous === undefined) {
        return { _tag: 'Refused', reason: 'ungranted', detail: 'This device is not granted message send.' }
      }
      const actor = capabilities.session_actor
      const current = async () => {
        const response = await client.capabilities()
        if (response.value.session_actor !== actor) throw new Error('Return to the original account to retry this message.')
        if (!hasConversationCapability(Native.decodeUnknownSync(Native.Capabilities)(response.value), 'message.send')) throw new Error('This device is not granted message send.')
        return response
      }
      const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(request.idempotency_key)))
      const messageId = `message/${[...digest.slice(0, 8)].map(byte => byte.toString(16).padStart(2, '0')).join('')}`
      const confirm = actionReceiptCheck({ kind: 'message', actionId: request.id, messageId })
      let first = true
      const result = previous?.retry === undefined
        ? await sendBoundAction(actor, {
          build: async () => {
            if (first) { first = false; return request }
            return { ...request, fence: { ...request.fence, snapshot_id: (await current()).snapshot.id } }
          },
          fence: async () => ({ ...request.fence, snapshot_id: (await current()).snapshot.id }),
          send: async input => {
            await confirm(async () => {
              const response = await client.messageSend({ ...input, parameters: {
                ...input.parameters, tags: [...input.parameters.tags], attachments: [...attachments],
              } })
              attempt.receipt = Native.decodeUnknownSync(schema.ActionResult)(response.value)
              return response
            }).catch(cause => { if (cause instanceof ClientError) attempt.error = cause; throw cause })
          },
          retry: false,
          wait: ms => new Promise(resolve => setTimeout(resolve, ms)),
        })
        : await resendBoundAction(previous.retry)
      if (result.how === 'unconfirmed') {
        attempt.retry = result.request
        unconfirmed.set(request.idempotency_key, attempt)
        return { _tag: 'Refused', reason: 'unconfirmed', detail: attempt.error?.response.message ?? 'Fractal has not confirmed this message. Retry to check the same message.', ...(attempt.error === undefined ? {} : { error: attempt.error.response }) }
      }
      unconfirmed.delete(request.idempotency_key)
      if (result.how === 'failed') {
        if (result.error instanceof ActionRejected) return { _tag: 'Success', value: Native.decodeUnknownSync(schema.ActionResult)(result.error.receipt) }
        if (result.error instanceof ClientError && result.error.response.code === 'stale-fence') {
          return { _tag: 'Refused', reason: 'stale-fence', detail: result.error.response.message, error: result.error.response }
        }
        return conversationPortFailure(result.error)
      }
      if (attempt.receipt === undefined) return { _tag: 'Refused', reason: 'unconfirmed', detail: 'Fractal has not confirmed this message.' }
      return { _tag: 'Success', value: attempt.receipt }
    } catch (cause) {
      if (previous !== undefined) return { _tag: 'Refused', reason: 'unconfirmed', detail: 'Return to the original connection to retry this message.' }
      return conversationPortFailure(cause)
    }
  }
}
