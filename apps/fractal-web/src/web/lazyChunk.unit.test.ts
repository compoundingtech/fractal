// @vitest-environment jsdom
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

it('isolates sticky highlighter failure from pane recovery', async () => {
  const highlighterImport = vi.fn().mockRejectedValue(new TypeError('highlighter unavailable'))
  const highlighter = createLazyChunk({ load: highlighterImport })
  const paneModule = { value: 'pane' }
  const paneImport = vi.fn().mockRejectedValueOnce(new TypeError('pane unavailable')).mockResolvedValue(paneModule)
  const pane = createLazyChunk({ load: paneImport })
  await expect(highlighter.load()).rejects.toThrow('highlighter unavailable')
  await expect(pane.load()).rejects.toThrow('pane unavailable')
  expect(pane.recovery).toBe('retry')
  await expect(pane.retry()).resolves.toBe(paneModule)
  await expect(highlighter.load()).rejects.toThrow('highlighter unavailable')
  expect(highlighterImport).toHaveBeenCalledTimes(1)
  expect(paneImport).toHaveBeenCalledTimes(2)
})

it('shares concurrent boundary retries, including the stylesheet replacement', async () => {
  const stylesheet = document.createElement('link')
  stylesheet.rel = 'stylesheet'
  stylesheet.href = '/assets/conversation.css'
  document.head.append(stylesheet)
  const replacement = vi.spyOn(stylesheet, 'replaceWith')
  const fetchAsset = vi.fn().mockResolvedValue({ ok: true, arrayBuffer: async () => new ArrayBuffer(0) })
  vi.stubGlobal('fetch', fetchAsset)
  const module = { value: 'loaded' }
  let finish!: (value: typeof module) => void
  const importer = vi.fn().mockRejectedValueOnce(new TypeError('network'))
    .mockImplementation(() => new Promise<typeof module>(resolve => { finish = resolve }))
  const chunk = createLazyChunk({ load: importer, assets: () => [stylesheet.href] })
  try {
    await expect(chunk.load()).rejects.toThrow('network')
    const firstBoundary = chunk.retry()
    const secondBoundary = chunk.retry()
    expect(secondBoundary).toBe(firstBoundary)
    expect(chunk.load()).toBe(firstBoundary)
    await vi.waitFor(() => expect(replacement).toHaveBeenCalledTimes(1))
    const restoredStylesheet = document.querySelector<HTMLLinkElement>('link[rel="stylesheet"]')!
    expect(fetchAsset).toHaveBeenCalledTimes(1)
    restoredStylesheet.dispatchEvent(new Event('load'))
    await vi.waitFor(() => expect(importer).toHaveBeenCalledTimes(2))
    expect(chunk.retry()).toBe(firstBoundary)
    finish(module)
    await expect(Promise.all([firstBoundary, secondBoundary])).resolves.toEqual([module, module])
    expect(replacement).toHaveBeenCalledTimes(1)
    expect(fetchAsset).toHaveBeenCalledTimes(1)
  } finally {
    document.querySelector('link[rel="stylesheet"]')?.remove()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  }
})
