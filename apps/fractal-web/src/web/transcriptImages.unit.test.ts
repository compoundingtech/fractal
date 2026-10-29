import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createTranscriptImages, transcriptImagesFor } from './transcriptImages.ts'
import type { AttachmentPort, ConversationPortResult } from '../data/source.ts'
import type { BlobChunk } from '@fractal/client/schema'

const sha = 'a'.repeat(64)
const attachment = { id: `blob/${sha}`, mediaType: 'image/png', name: 'review.png' }
const chunk = (data = 'aGVsbG8=', offset = 0, size = 5) => ({ _tag: 'Success' as const, value: { sha256: sha, data, offset, size } })
const urls = () => ({ createObjectURL: vi.fn((_blob: Blob) => 'blob:viewer'), revokeObjectURL: vi.fn() })
const decode = vi.fn<() => Promise<void>>()
beforeEach(() => {
  decode.mockReset().mockResolvedValue()
  vi.stubGlobal('Image', class {
    src = ''
    decode = decode
  })
})
afterEach(() => vi.unstubAllGlobals())

describe('transcript image byte reads', () => {
  it('does not read before consent; assembles chunks and includes the native authorization carrier', async () => {
    const read = vi.fn<AttachmentPort['chunk']>().mockResolvedValueOnce(chunk('aGU=', 0)).mockResolvedValueOnce(chunk('bGxv', 2))
    const objectUrls = urls()
    const cache = createTranscriptImages({ chunk: read }, { urls: objectUrls })
    expect(read).not.toHaveBeenCalled()
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Deferred' })
    await cache.load({ ...attachment, messageId: 'message/native' }, 'pending/optimistic')
    expect(read.mock.calls).toEqual([[{ sha256: sha, message: 'message/native', offset: 0 }], [{ sha256: sha, message: 'message/native', offset: 2 }]])
    const blob = objectUrls.createObjectURL.mock.calls[0]![0]
    expect(await blob.text()).toBe('hello')
    expect(blob.type).toBe('image/png')
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Loaded', src: 'blob:viewer' })
    await cache.load(attachment, 'message/native')
    expect(read).toHaveBeenCalledTimes(2)
  })

  it('deduplicates pending reads, publishes stable snapshots, and revokes on disposal', async () => {
    const { promise, resolve: finish } = Promise.withResolvers<ConversationPortResult<BlobChunk>>()
    const read = vi.fn<AttachmentPort['chunk']>(() => promise)
    const objectUrls = urls()
    const cache = createTranscriptImages({ chunk: read }, { urls: objectUrls })
    const listener = vi.fn()
    const unsubscribe = cache.subscribe(listener)
    const pending = cache.load(attachment, 'message/native')
    const snapshot = cache.snapshot()
    expect(cache.snapshot()).toBe(snapshot)
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Loading' })
    await cache.load(attachment, 'message/native')
    expect(read).toHaveBeenCalledTimes(1)
    finish(chunk())
    await pending
    expect(listener).toHaveBeenCalledTimes(3)
    expect(cache.snapshot()).not.toBe(snapshot)
    cache.dispose()
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledWith('blob:viewer')
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Deferred' })
    expect(listener).toHaveBeenCalledTimes(3)
    unsubscribe()
  })

  it.each(['ungranted', 'failed'] as const)('keeps %s reads deferred with an explicit state and allows a user retry', async reason => {
    const read = vi.fn<AttachmentPort['chunk']>().mockResolvedValueOnce({ _tag: 'Refused', reason, detail: 'private diagnostic' }).mockResolvedValueOnce(chunk())
    const cache = createTranscriptImages({ chunk: read }, { urls: urls() })
    await cache.load(attachment, 'timeline-entry/content')
    expect(cache.state(attachment.id)).toEqual({ _tag: reason === 'ungranted' ? 'Unauthorized' : 'Failed' })
    expect(read.mock.calls[0]).toEqual([{ sha256: sha, offset: 0 }])
    await cache.load(attachment, 'message/native')
    expect(cache.state(attachment.id)._tag).toBe('Loaded')
  })

  it('evicts oldest entries at the count bound and revokes their URLs', async () => {
    const objectUrls = urls()
    const read = vi.fn<AttachmentPort['chunk']>(async request => ({ ...chunk(), value: { ...chunk().value, sha256: request.sha256 } }))
    const cache = createTranscriptImages({ chunk: read }, { maxEntries: 1, urls: objectUrls })
    await cache.load(attachment, 'message/a')
    const second = { ...attachment, id: `blob/${'b'.repeat(64)}` }
    await cache.load(second, 'message/b')
    expect(cache.state(attachment.id)._tag).toBe('Deferred')
    expect(cache.state(second.id)._tag).toBe('Loaded')
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledTimes(1)
    cache.dispose()
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledTimes(2)
  })

  it('evicts at the byte bound independently of the count bound', async () => {
    const objectUrls = urls()
    const read = vi.fn<AttachmentPort['chunk']>(async request => ({ ...chunk(), value: { ...chunk().value, sha256: request.sha256 } }))
    const cache = createTranscriptImages({ chunk: read }, { maxEntries: 10, maxBytes: 7, urls: objectUrls })
    await cache.load(attachment, 'message/a')
    await cache.load({ ...attachment, id: `blob/${'b'.repeat(64)}` }, 'message/b')
    expect(cache.state(attachment.id)._tag).toBe('Deferred')
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledTimes(1)
  })

  it('discards a late completion after unmount without allocating any URL', async () => {
    const { promise, resolve: finish } = Promise.withResolvers<ConversationPortResult<BlobChunk>>()
    const objectUrls = urls()
    const cache = createTranscriptImages({ chunk: () => promise }, { urls: objectUrls })
    const pending = cache.load(attachment, 'message/native')
    cache.dispose()
    finish(chunk())
    await pending
    expect(objectUrls.createObjectURL).not.toHaveBeenCalled()
    expect(cache.snapshot().size).toBe(0)
  })

  it('keeps a complete image loading until decoding finishes', async () => {
    const decoding = Promise.withResolvers<void>()
    decode.mockReturnValueOnce(decoding.promise)
    const objectUrls = urls()
    const cache = createTranscriptImages({ chunk: async () => chunk() }, { urls: objectUrls })
    const pending = cache.load(attachment, 'message/native')
    await vi.waitFor(() => expect(decode).toHaveBeenCalledOnce())
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Loading' })
    decoding.resolve()
    await pending
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Loaded', src: 'blob:viewer' })
  })

  it('rejects undecodable PNG-signature bytes, revokes the URL and permits a fresh read on retry', async () => {
    // The PNG magic is valid, but there are no chunks or pixels for a browser to decode.
    const data = btoa(String.fromCharCode(137, 80, 78, 71, 13, 10, 26, 10))
    const read = vi.fn<AttachmentPort['chunk']>().mockResolvedValue(chunk(data, 0, 8))
    const objectUrls = urls()
    decode.mockRejectedValueOnce(new DOMException('The source image cannot be decoded.', 'EncodingError'))
    const cache = createTranscriptImages({ chunk: read }, { urls: objectUrls })
    await cache.load(attachment, 'message/native')
    expect(new Uint8Array(await objectUrls.createObjectURL.mock.calls[0]![0].arrayBuffer())).toEqual(new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]))
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Failed' })
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledExactlyOnceWith('blob:viewer')
    await cache.load(attachment, 'message/native')
    expect(read).toHaveBeenCalledTimes(2)
    expect(decode).toHaveBeenCalledTimes(2)
    expect(cache.state(attachment.id)._tag).toBe('Loaded')
    cache.dispose()
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledTimes(2)
  })

  it.each(['dispose', 'evict'] as const)('revokes an in-flight decode on %s and ignores its late completion', async action => {
    const decoding = Promise.withResolvers<void>()
    decode.mockReturnValueOnce(decoding.promise)
    const objectUrls = urls()
    const cache = createTranscriptImages({ chunk: async request => ({ ...chunk(), value: { ...chunk().value, sha256: request.sha256 } }) }, { maxEntries: 1, urls: objectUrls })
    const pending = cache.load(attachment, 'message/native')
    await vi.waitFor(() => expect(decode).toHaveBeenCalledOnce())
    if (action === 'dispose') cache.dispose()
    else await cache.load({ ...attachment, id: `blob/${'b'.repeat(64)}` }, 'message/other')
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledExactlyOnceWith('blob:viewer')
    decoding.resolve()
    await pending
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Deferred' })
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledTimes(1)
    cache.dispose()
  })

  it.each([
    { ...chunk().value, offset: 1 },
    { ...chunk().value, size: 11 * 1024 * 1024 },
    { ...chunk().value, sha256: 'b'.repeat(64) },
    { ...chunk().value, data: '' },
    { ...chunk().value, data: 'not base64' },
    { ...chunk().value, size: 4 },
  ])('rejects malformed or oversized chunk metadata without creating a URL', async value => {
    const objectUrls = urls()
    const cache = createTranscriptImages({ chunk: async () => ({ _tag: 'Success', value }) }, { urls: objectUrls })
    await cache.load(attachment, 'message/native')
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Failed' })
    expect(objectUrls.createObjectURL).not.toHaveBeenCalled()
  })

  it('turns transport rejection into a retryable failed card', async () => {
    const cache = createTranscriptImages({ chunk: async () => { throw new Error('network failed') } }, { urls: urls() })
    await cache.load(attachment, 'message/native')
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Failed' })
  })

  it.each([
    { ...attachment, id: 'attachment/native' },
    { ...attachment, mediaType: 'image/svg+xml' },
    { ...attachment, mediaType: 'image/avif' },
    { ...attachment, mediaType: 'text/plain' },
  ])('keeps permanently unsupported attachments non-retryable without reading', async unsupported => {
    const read = vi.fn<AttachmentPort['chunk']>()
    const cache = createTranscriptImages({ chunk: read }, { urls: urls() })
    expect(cache.state(unsupported.id, unsupported.mediaType)).toEqual({ _tag: 'Unavailable' })
    await cache.load(unsupported, 'message/native')
    await cache.load(unsupported, 'message/native')
    expect(cache.state(unsupported.id, unsupported.mediaType)).toEqual({ _tag: 'Unavailable' })
    expect(cache.snapshot().size).toBe(0)
    expect(read).not.toHaveBeenCalled()
  })

  it('shares one 24-entry bound across three retained panes and revokes only on final unmount', async () => {
    let createdCount = 0
    const created = vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:viewer-${++createdCount}`)
    const revoked = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
    const read = vi.fn<AttachmentPort['chunk']>(async request => ({ ...chunk(), value: { ...chunk().value, sha256: request.sha256 } }))
    const source = { attachments: { chunk: read } }
    const panes = Array.from({ length: 3 }, () => transcriptImagesFor(source))
    const releases = panes.map(pane => pane.acquire())
    try {
      expect(panes[1]).toBe(panes[0])
      expect(panes[2]).toBe(panes[0])
      for (let index = 0; index < 25; index += 1) {
        await panes[index % 3]!.load({ ...attachment, id: `blob/${index.toString(16).padStart(64, '0')}` }, `message/pane-${index % 3}`)
      }
      expect(panes[0]!.snapshot().size).toBe(24)
      expect(panes[1]!.state(`blob/${'0'.repeat(64)}`)._tag).toBe('Deferred')
      expect(created).toHaveBeenCalledTimes(25)
      expect(revoked).toHaveBeenCalledTimes(1)
      releases[0]!()
      releases[0]!()
      releases[1]!()
      expect(panes[2]!.snapshot().size).toBe(24)
      expect(revoked).toHaveBeenCalledTimes(1)
      releases[2]!()
      expect(panes[2]!.snapshot().size).toBe(0)
      expect(revoked).toHaveBeenCalledTimes(25)
    } finally {
      releases.forEach(release => release())
      created.mockRestore()
      revoked.mockRestore()
    }
  })

  it('never shares a cached image across different viewer/data-source identities', async () => {
    const created = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:viewer-a')
    const revoked = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
    const port = { chunk: vi.fn<AttachmentPort['chunk']>().mockResolvedValue(chunk()) }
    const first = transcriptImagesFor({ attachments: port })
    const second = transcriptImagesFor({ attachments: port })
    const releaseFirst = first.acquire()
    const releaseSecond = second.acquire()
    try {
      await first.load(attachment, 'message/native')
      expect(first.state(attachment.id)._tag).toBe('Loaded')
      expect(second.state(attachment.id)._tag).toBe('Deferred')
      expect(first).not.toBe(second)
    } finally {
      releaseFirst()
      releaseSecond()
      created.mockRestore()
      revoked.mockRestore()
    }
  })
})
