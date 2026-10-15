import type { ConversationRuntimeOptions } from '@smalltalk/fractal-ui/assistant-ui'
import { composerImageRejection, composerImageRejectionMessage, composerImageTypes } from '@smalltalk/fractal-ui/assistant-ui/image-limits'
import type { AttachmentPort } from '../data/source.ts'

export const isComposerImageType = (value: string): value is typeof composerImageTypes[number] => composerImageTypes.some(type => type === value)
type Adapter = NonNullable<NonNullable<ConversationRuntimeOptions['adapters']>['attachments']>

/** Runtime retains the original File on upload failure and restores the entire submission.
 * Upload references are cached by draft image identity: an explicit retry never reuploads
 * a successfully uploaded image merely because another upload or the send answer failed. */
export const createComposerAttachmentAdapter = (port: AttachmentPort): Adapter & {
  readonly submitted: (blobs: readonly string[]) => void
  readonly nativeId: (id: string) => string | undefined
} => {
  const staged = new Set<string>()
  const uploaded = new Map<string, string>()
  return {
    nativeId: id => uploaded.get(id),
    submitted: blobs => {
      for (const [id, blob] of uploaded) if (blobs.includes(blob)) {
        staged.delete(id)
        uploaded.delete(id)
      }
    },
    accept: composerImageTypes.join(','),
    add: async ({ file }) => {
      const rejection = composerImageRejection(file, staged.size)
      if (rejection !== undefined) throw new Error(composerImageRejectionMessage(rejection))
      const id = crypto.randomUUID()
      staged.add(id)
      return { id, type: 'image', name: file.name, contentType: file.type, file,
        status: { type: 'requires-action', reason: 'composer-send' } }
    },
    remove: async attachment => { staged.delete(attachment.id); uploaded.delete(attachment.id) },
    send: async attachment => {
      const mediaType = attachment.file.type
      if (!isComposerImageType(mediaType)) throw new Error('Choose a PNG, JPEG, GIF or WebP image.')
      let blob = uploaded.get(attachment.id)
      if (blob === undefined) {
        const result = await port.upload({ bytes: attachment.file, mediaType })
        if (result._tag === 'Refused') throw new Error(result.detail)
        blob = result.value.blob
        uploaded.set(attachment.id, blob)
      }
      return { ...attachment, contentType: mediaType, status: { type: 'complete' }, content: [] }
    },
  }
}
