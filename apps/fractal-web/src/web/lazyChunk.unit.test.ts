import { expect, it, vi } from 'vitest'
import { createLazyChunk } from '@fractal/ui/lazy-chunk'

it('shares each attempt and offers reload only after the same-document retry fails', async () => {
  const failure = new TypeError('chunk fetch failed')
  const importer = vi.fn().mockRejectedValue(failure)
  const chunk = createLazyChunk({ load: importer })
  const initial = chunk.load()
  expect(chunk.load()).toBe(initial)
  await expect(initial).rejects.toBe(failure)
  expect(chunk.recovery).toBe('retry')
  await expect(chunk.load()).rejects.toBe(failure)
  expect(importer).toHaveBeenCalledTimes(1)
  await expect(chunk.retry()).rejects.toBe(failure)
  expect(importer).toHaveBeenCalledTimes(2)
  expect(chunk.recovery).toBe('reload')
})

it('retries successfully on engines that release failed module fetches', async () => {
  const module = { value: 'loaded' }
  const importer = vi.fn().mockRejectedValueOnce(new TypeError('network')).mockResolvedValue(module)
  const chunk = createLazyChunk({ load: importer })
  await expect(chunk.load()).rejects.toThrow('network')
  await expect(chunk.retry()).resolves.toBe(module)
  await expect(chunk.load()).resolves.toBe(module)
  expect(importer).toHaveBeenCalledTimes(2)
})
