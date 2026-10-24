import { describe, expect, it, vi } from 'vitest'
import { createTranscriptImages } from './transcriptImages.ts'
import type { AttachmentPort, ConversationPortResult } from '../data/source.ts'
import type { BlobChunk } from '@fractal/client/schema'

const sha = 'a'.repeat(64)
const attachment = { id: `blob/${sha}`, mediaType: 'image/png', name: 'review.png' }
const chunk = (data = 'aGVsbG8=', offset = 0, size = 5) => ({ _tag: 'Success' as const, value: { sha256: sha, data, offset, size } })
const urls = () => ({ createObjectURL: vi.fn((_blob: Blob) => 'blob:viewer'), revokeObjectURL: vi.fn() })

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
    expect(listener).toHaveBeenCalledTimes(2)
    expect(cache.snapshot()).not.toBe(snapshot)
    unsubscribe()
    cache.dispose()
    expect(objectUrls.revokeObjectURL).toHaveBeenCalledWith('blob:viewer')
    expect(cache.state(attachment.id)).toEqual({ _tag: 'Deferred' })
    expect(listener).toHaveBeenCalledTimes(2)
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
})
