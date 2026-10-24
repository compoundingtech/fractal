import type { ConversationRuntimeOptions } from '@fractal/ui/assistant-ui'
import type { ConversationItem, SendState } from '../conversation/model.ts'
import type { AttachmentPort, AttachmentSendRequest, DataSource, Grants } from '../data/source.ts'

export type SendRefusal = { readonly reason: string }

type AttachmentAdapter = NonNullable<NonNullable<ConversationRuntimeOptions['adapters']>['attachments']>
type PendingAttachment = Parameters<AttachmentAdapter['send']>[0]
type SendAttachment = AttachmentSendRequest['parameters']['attachments'][number]
type ImageMediaType = SendAttachment['media_type']

/** The message.send contract: at most four attachments, each a PNG, JPEG, GIF or WebP blob reference. */
const messageImageLimit = { max: 4 } as const
const imageMediaTypes: readonly ImageMediaType[] = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']
const isImageMediaType = (value: string | undefined): value is ImageMediaType => imageMediaTypes.some(type => type === value)

/**
 * Composer images upload when the message is sent, never while drafting. A refused upload rejects, so the
 * runtime returns the draft with the attachment marked failed and its reason; nothing is sent.
 */
export const imageUploadAdapter = (upload: AttachmentPort['upload']): AttachmentAdapter & { readonly add: (state: { readonly file: File }) => Promise<PendingAttachment> } => ({
  accept: imageMediaTypes.join(','),
  add: async ({ file }) => {
    if (!isImageMediaType(file.type)) throw new Error('Only PNG, JPEG, GIF and WebP images can be attached.')
    return { id: crypto.randomUUID(), type: 'image', name: file.name, contentType: file.type, file, status: { type: 'requires-action', reason: 'composer-send' } }
  },
  remove: async () => {},
  send: async (attachment) => {
    const mediaType = attachment.contentType
    if (!isImageMediaType(mediaType)) throw new Error('Only PNG, JPEG, GIF and WebP images can be attached.')
    const uploaded = await upload({ bytes: attachment.file, mediaType })
    if (uploaded._tag === 'Refused') throw new Error(uploaded.detail)
    // The upload's blob reference is the attachment; message.send names it, the bytes stay with st.
    return { ...attachment, status: { type: 'complete' }, content: [{ type: 'file', data: uploaded.value.blob, mimeType: uploaded.value.media_type, filename: attachment.name, sourceType: 'id' }] }
  },
})

/** One media-type guard for new sends and outbox retries; undefined when the reference cannot be sent. */
const sendAttachment = ({ blob, mediaType, name }: { readonly blob: string; readonly mediaType: string | undefined; readonly name: string | null }): SendAttachment | undefined =>
  isImageMediaType(mediaType) ? { blob, media_type: mediaType, name } : undefined

type ComposerAttachments = Parameters<ConversationRuntimeOptions['onNew']>[0]['attachments']
const uploadedAttachments = (attachments: NonNullable<ComposerAttachments>): SendAttachment[] | undefined => {
  const sendable: SendAttachment[] = []
  for (const attachment of attachments) {
    const part = attachment.content.find(part => part.type === 'file' && part.sourceType === 'id')
    const reference = part?.type === 'file' ? sendAttachment({ blob: part.data, mediaType: part.mimeType, name: attachment.name }) : undefined
    if (reference === undefined) return undefined
    sendable.push(reference)
  }
  return sendable
}

const sendFailureDetails: Readonly<Record<Extract<SendState, { _tag: 'Failed' }>['reason'], string>>
  & Readonly<Record<string, string | undefined>> = {
    rejected: 'The server did not accept this message.',
    ungranted: 'Message send is not granted for this device.',
    invalid: 'The message does not match the supported send format.',
    failed: 'The message could not be sent. Check your connection and retry.',
    'stale-fence': 'The conversation changed before this message could be sent. Retry to use its current state.',
    'snapshot-unavailable': 'The conversation could not be loaded for sending. Nothing was sent. Retry when it is available.',
    unconfirmed: 'st may have taken this message and has not confirmed it yet. Sending it again is safe.',
  }

/** Send diagnostics are not display copy, including for unclassified failure reasons. */
export const sendFailureDetail = (reason: string): string =>
  (Object.hasOwn(sendFailureDetails, reason) ? sendFailureDetails[reason] : undefined) ?? sendFailureDetails.failed

/** The app owns draft persistence; the source owns keys, outbox, send state and echo reconciliation. */
export const composerSendBinding = ({ source, agentRef, grants, readable, items, refusal, onRefused, onSending, attachmentAdapter }: {
  readonly source: Pick<DataSource, 'attachments'>
  readonly agentRef: string
  readonly grants: Grants
  /** The outbox shows a send only in a readable conversation; an unreadable one would hide its row and outcome. */
  readonly readable: boolean
  readonly items: NonNullable<ConversationRuntimeOptions['messages']>
  readonly refusal?: SendRefusal
  readonly onRefused: (refusal: SendRefusal) => void
  /** Capture before dispatch; optimistic clears are not confirmed sends. */
  readonly onSending?: (content: string) => (outcome: { readonly _tag: 'Confirmed' } | { readonly _tag: 'Unconfirmed' }) => void
  /** Stable across renders (see `imageUploadAdapter`); offered only while sending and image upload are granted. */
  readonly attachmentAdapter?: AttachmentAdapter
}): {
  readonly runtime: ConversationRuntimeOptions
  readonly disabledReason?: string
  /** Present while the composer may attach images: the per-message limit it enforces. */
  readonly imageAttachments?: { readonly max: number }
  readonly retry: (item: ConversationItem) => Promise<void>
} => {
  const disabledReason = refusal !== undefined ? sendFailureDetail(refusal.reason)
    : source.attachments === undefined ? 'This view cannot send messages.'
    : grants.messageSend !== 'granted' ? 'Message send is not granted for this device.'
    : !readable ? 'Messages can be sent once this conversation loads.' : undefined
  const uploads = attachmentAdapter !== undefined && disabledReason === undefined && grants.attachmentUpload === 'granted' ? attachmentAdapter : undefined
  const dispatch = async (content: string, attachments: AttachmentSendRequest['parameters']['attachments'], retryKey?: string) => {
    if (source.attachments === undefined || grants.messageSend !== 'granted') return
    // TODO(live-migration: arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both): keep old names until all members accept aliases; sunset 60 days after first shipping release.
    const request = {
      api_version: 'st3.client.v0', type: 'message.send', id: `action/${crypto.randomUUID()}`,
      parameters: { to: agentRef, content, tags: [], attachments },
    } as const
    // An outbox retry has no original composer revision in this port. Never capture the
    // current draft as its submission: equal text does not establish submission identity.
    const settled = retryKey === undefined ? onSending?.(content) : undefined
    let confirmed = false
    try {
      const result = await source.attachments.send(retryKey === undefined
        ? { ...request, _tag: 'Send' }
        : { ...request, _tag: 'Resend', idempotencyKey: retryKey })
      // Retryable send failures belong to the outbox row, not the next draft's availability.
      if (result._tag === 'Refused' && result.reason !== 'failed' && result.reason !== 'unconfirmed'
        && result.reason !== 'stale-fence' && result.reason !== 'snapshot-unavailable') onRefused({ reason: result.reason })
      confirmed = result._tag === 'Success' && result.value.status === 'completed'
    } finally {
      settled?.({ _tag: confirmed ? 'Confirmed' : 'Unconfirmed' })
    }
  }
  return {
    ...(disabledReason === undefined ? {} : { disabledReason }),
    ...(uploads === undefined ? {} : { imageAttachments: messageImageLimit }),
    runtime: {
      messages: items, isDisabled: disabledReason !== undefined,
      ...(uploads === undefined ? {} : { adapters: { attachments: uploads } }),
      onNew: async (message) => {
        if (disabledReason !== undefined) return
        const sendable = uploadedAttachments(message.attachments ?? [])
        if (sendable === undefined) {
          onRefused({ reason: 'invalid' })
          return
        }
        await dispatch(message.content.filter(part => part.type === 'text').map(part => part.text).join('\n'), sendable)
      },
      // Deliberately no onCancel: runtime.signal is not a send-cancel contract. The pane wires Stop to the agent's interrupt.
    },
    retry: async (item) => {
      if (item._tag !== 'Text' || (item.sendState?._tag !== 'Failed' && item.sendState?._tag !== 'Unconfirmed')
        || !item.id.startsWith('pending/')) return
      // pending/<key> is the source-owned outbox identity, not a text/time dedupe heuristic.
      // The source resends the key's recorded action id and parameters; these only route it.
      const attachments: SendAttachment[] = []
      for (const attachment of item.attachments) {
        const reference = sendAttachment({ blob: attachment.id, mediaType: attachment.mediaType, name: attachment.name ?? null })
        if (reference === undefined) {
          onRefused({ reason: 'invalid' })
          return
        }
        attachments.push(reference)
      }
      await dispatch(item.text, attachments, item.id.slice('pending/'.length))
    },
  }
}
