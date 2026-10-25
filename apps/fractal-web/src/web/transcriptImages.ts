import type { Attachment } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-data/model.ts'
import type { AttachmentPort } from '../data/source.ts'

import type { TranscriptAttachmentImageState as TranscriptImageState } from '@fractal/ui/assistant-ui'

const deferred: TranscriptImageState = { _tag: 'Deferred' }
const imageTypes: Readonly<Record<string, true | undefined>> = { 'image/png': true, 'image/jpeg': true, 'image/gif': true, 'image/webp': true }
const maxImageBytes = 10 * 1024 * 1024

/** Viewer-scoped, consent-only byte reads. No upload preview URL enters this cache. */
export function createTranscriptImages(port: Pick<AttachmentPort, 'chunk'> | undefined, options: {
  readonly maxEntries?: number
  readonly maxBytes?: number
  readonly urls?: Pick<typeof URL, 'createObjectURL' | 'revokeObjectURL'>
} = {}) {
  const maxEntries = Math.max(1, options.maxEntries ?? 24)
  const maxBytes = Math.max(1, options.maxBytes ?? 40 * 1024 * 1024)
  const urls = options.urls ?? URL
  let states: ReadonlyMap<string, TranscriptImageState> = new Map()
  const sizes = new Map<string, number>()
  const listeners = new Set<() => void>()
  let bytes = 0
  const publish = (next: Map<string, TranscriptImageState>) => {
    states = next
    for (const listener of listeners) listener()
  }
  const remove = (next: Map<string, TranscriptImageState>, id: string) => {
    const state = next.get(id)
    if (state?._tag === 'Loaded') urls.revokeObjectURL(state.src)
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
  const state = (id: string) => states.get(id) ?? deferred
  const load = async (attachment: Attachment, messageId: string): Promise<void> => {
    const previous = state(attachment.id)
    if (previous._tag === 'Loading' || previous._tag === 'Loaded') return
    if (port === undefined) { put(attachment.id, { _tag: 'Unauthorized' }); return }
    const sha256 = /^blob\/([0-9a-f]{64})$/.exec(attachment.id)?.[1]
    if (sha256 === undefined || imageTypes[attachment.mediaType] !== true) { put(attachment.id, { _tag: 'Failed' }); return }
    const pending: TranscriptImageState = { _tag: 'Loading' }
    put(attachment.id, pending)
    // Disposal or eviction invalidates an in-flight read; its late answer cannot allocate a URL.
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
      put(attachment.id, { _tag: 'Loaded', src: urls.createObjectURL(blob) }, blob.size)
    } catch {
      if (current()) put(attachment.id, { _tag: 'Failed' })
    }
  }
  return {
    state, load,
    snapshot: () => states,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    dispose: () => {
      const next = new Map(states)
      for (const id of next.keys()) remove(next, id)
      // Teardown must not notify React while its subscriptions are being detached.
      states = next
    },
  }
}
