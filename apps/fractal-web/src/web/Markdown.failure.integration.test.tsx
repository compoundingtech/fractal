// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type * as LazyChunkModule from '../../../../packages/fractal-ui/src/lazy-chunk.ts'
import { Markdown, HighlightedSource } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const attempts = vi.hoisted(() => ({ load: vi.fn(), retry: vi.fn(), reload: vi.fn() }))
vi.mock('../../../../packages/fractal-ui/src/lazy-chunk.ts', async importOriginal => {
  const original = await importOriginal<typeof LazyChunkModule>()
  return {
    ...original,
    createLazyChunk: ((options: Parameters<typeof original.createLazyChunk>[0]) => {
      const chunk = original.createLazyChunk({ ...options, load: () => { attempts.load(); return options.load() } })
      return { ...chunk, retry: attempts.retry, reload: attempts.reload }
    }) as typeof original.createLazyChunk,
  }
})
vi.mock('@pierre/highlights', () => { throw new TypeError('highlighter chunk unavailable') })

it('keeps failed highlighting plain across later Markdown and diff-source mounts without retry or reload', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const paints: FrameRequestCallback[] = []
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { paints.push(callback); return paints.length })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const settle = async () => {
    for (const callback of paints.splice(0)) callback(0)
    await vi.dynamicImportSettled()
    const { promise, resolve } = Promise.withResolvers<void>()
    setTimeout(resolve, 0)
    await promise
    await vi.dynamicImportSettled()
  }
  try {
    await act(async () => root.render(<Markdown text={'```typescript\nconst first = 1\n```'} />))
    const code = container.querySelector('code')!
    expect(code.textContent).toBe('const first = 1\n')
    await act(settle)
    expect(attempts.load).toHaveBeenCalledTimes(1)
    expect(container.querySelector('code')).toBe(code)
    expect(code.querySelector('[data-syntax-token]')).toBeNull()
    await act(async () => root.render(<Markdown key="later" text={'```typescript\nconst later = 2\n```'} />))
    await act(settle)
    expect(container.querySelector('code')?.textContent).toBe('const later = 2\n')
    // DiffPanel consumes the same HighlightedSource; mounting it must share sticky failure.
    await act(async () => root.render(<code><HighlightedSource code="const diff = 3" language="typescript" /></code>))
    await act(settle)
    expect(container.textContent).toBe('const diff = 3')
    expect(container.querySelector('[data-syntax-token]')).toBeNull()
    expect(container.querySelector('button')).toBeNull()
    expect(attempts.load).toHaveBeenCalledTimes(1)
    expect(attempts.retry).not.toHaveBeenCalled()
    expect(attempts.reload).not.toHaveBeenCalled()
  } finally {
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
