import type { Attachment } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-data/model.ts'
import type { AttachmentPort } from '../data/source.ts'

import type { TranscriptAttachmentImageState as TranscriptImageState } from '@fractal/ui/assistant-ui'

const deferred: TranscriptImageState = { _tag: 'Deferred' }
const unavailable: TranscriptImageState = { _tag: 'Unavailable' }
const imageTypes: Readonly<Record<string, true | undefined>> = { 'image/png': true, 'image/jpeg': true, 'image/gif': true, 'image/webp': true }
const maxImageBytes = 10 * 1024 * 1024

export interface TranscriptImageCache {
  readonly state: (id: string, mediaType?: string) => TranscriptImageState
  readonly load: (attachment: Attachment, messageId: string) => Promise<void>
  readonly snapshot: () => ReadonlyMap<string, TranscriptImageState>
  readonly subscribe: (listener: () => void) => () => void
  /** Each mounted pane acquires the shared viewer cache; the final release revokes its URLs. */
  readonly acquire: () => () => void
  readonly dispose: () => void
}

type TranscriptImageSource = { readonly attachments?: Pick<AttachmentPort, 'chunk'> }
const viewerCaches = new WeakMap<TranscriptImageSource, TranscriptImageCache>()

/** Retained conversation panes share one bounded cache for their data-source/viewer identity. */
export const transcriptImagesFor = (source: TranscriptImageSource): TranscriptImageCache => {
  const existing = viewerCaches.get(source)
  if (existing !== undefined) return existing
  const cache = createTranscriptImages(source.attachments)
  viewerCaches.set(source, cache)
  return cache
}

/** Viewer-scoped, consent-only byte reads. No upload preview URL enters this cache. */
export function createTranscriptImages(port: Pick<AttachmentPort, 'chunk'> | undefined, options: {
  readonly maxEntries?: number
  readonly maxBytes?: number
  readonly urls?: Pick<typeof URL, 'createObjectURL' | 'revokeObjectURL'>
} = {}): TranscriptImageCache {
  const maxEntries = Math.max(1, options.maxEntries ?? 24)
  const maxBytes = Math.max(1, options.maxBytes ?? 40 * 1024 * 1024)
  const urls = options.urls ?? URL
  let states: ReadonlyMap<string, TranscriptImageState> = new Map()
  const sizes = new Map<string, number>()
  const decodingUrls = new Map<string, string>()
  const listeners = new Set<() => void>()
  let bytes = 0
  let owners = 0
  const publish = (next: Map<string, TranscriptImageState>) => {
    states = next
    for (const listener of listeners) listener()
  }
  const remove = (next: Map<string, TranscriptImageState>, id: string) => {
    const state = next.get(id)
    if (state?._tag === 'Loaded') urls.revokeObjectURL(state.src)
    const decoding = decodingUrls.get(id)
    if (decoding !== undefined) {
      urls.revokeObjectURL(decoding)
      decodingUrls.delete(id)
    }
    bytes -= sizes.get(id) ?? 0
    sizes.delete(id)
    next.delete(id)
  }
  const put = (id: string, state: TranscriptImageState, size = 0) => {
    const next = new Map(states)
    remove(next, id)
    while (next.size >= maxEntries || bytes + size > maxBytes) {
      const oldest = next.keys().next().value
      if (oldest === undefined) break
      remove(next, oldest)
    }
    next.set(id, state)
    sizes.set(id, size)
    bytes += size
    publish(next)
  }
  const state = (id: string, mediaType?: string) => {
    if (!/^blob\/[0-9a-f]{64}$/.test(id) || (mediaType !== undefined && imageTypes[mediaType] !== true)) return unavailable
    return states.get(id) ?? deferred
  }
  const load = async (attachment: Attachment, messageId: string): Promise<void> => {
    const previous = state(attachment.id, attachment.mediaType)
    if (previous._tag === 'Unavailable' || previous._tag === 'Loading' || previous._tag === 'Loaded') return
    if (port === undefined) { put(attachment.id, { _tag: 'Unauthorized' }); return }
    const sha256 = attachment.id.slice('blob/'.length)
    const pending: TranscriptImageState = { _tag: 'Loading' }
    put(attachment.id, pending)
    // Disposal or eviction invalidates an in-flight read or decode and releases its URL.
    const current = () => states.get(attachment.id) === pending
    try {
      const parts: ArrayBuffer[] = []
      let offset = 0
      let total: number | undefined
      const carrier = attachment.messageId ?? (messageId.startsWith('message/') || messageId.startsWith('step-run/') ? messageId : undefined)
      do {
        const result = await port.chunk({ sha256, ...(carrier === undefined ? {} : { message: carrier }), offset })
        if (!current()) return
        if (result._tag === 'Refused') {
          put(attachment.id, { _tag: result.reason === 'ungranted' ? 'Unauthorized' : 'Failed' })
          return
        }
        const chunk = result.value
        if (chunk.sha256 !== sha256 || chunk.offset !== offset || !Number.isSafeInteger(chunk.size) || chunk.size <= 0 || chunk.size > Math.min(maxImageBytes, maxBytes) || (total !== undefined && chunk.size !== total)) {
          put(attachment.id, { _tag: 'Failed' }); return
        }
        total = chunk.size
        const decoded = atob(chunk.data)
        if (decoded.length === 0 || decoded.length > 512 * 1024 || offset + decoded.length > total) { put(attachment.id, { _tag: 'Failed' }); return }
        parts.push(Uint8Array.from(decoded, character => character.charCodeAt(0)).buffer)
        offset += decoded.length
      } while (offset < total)
      if (!current()) return
      const blob = new Blob(parts, { type: attachment.mediaType })
      put(attachment.id, pending, blob.size)
      if (!current()) return
      const src = urls.createObjectURL(blob)
      decodingUrls.set(attachment.id, src)
      const image = new Image()
      image.src = src
      await image.decode()
      if (!current()) return
      decodingUrls.delete(attachment.id)
      put(attachment.id, { _tag: 'Loaded', src }, blob.size)
    } catch {
      if (current()) put(attachment.id, { _tag: 'Failed' })
    }
  }
  const dispose = () => {
    const next = new Map(states)
    for (const id of next.keys()) remove(next, id)
    // Teardown must not notify React while its subscriptions are being detached.
    states = next
  }
  return {
    state, load,
    snapshot: () => states,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    acquire: () => {
      owners += 1
      let released = false
      return () => {
        if (released) return
        released = true
        owners -= 1
        if (owners === 0) dispose()
      }
    },
    dispose,
  }
}
