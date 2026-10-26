// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
afterEach(() => vi.unstubAllGlobals())
it('mounts every real embed view without fetch or WebSocket', async () => {
  const network = vi.fn(() => { throw new Error('Embed attempted network access') })
  vi.stubGlobal('fetch', network)
  vi.stubGlobal('WebSocket', network)
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('CSS', { escape: (value: string) => value.replaceAll('/', '\\/') })
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => setTimeout(fn, 0))
  vi.stubGlobal('cancelAnimationFrame', clearTimeout)
  const { Embed, fragments } = await import('./Embed.tsx')
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    for (const fragment of fragments) {
      await act(async () => { root.render(<Embed fragment={fragment} />); await new Promise(resolve => setTimeout(resolve, 100)) })
      expect(host.textContent).not.toBe('')
      if (fragment === 'codex' || fragment === 'claude-code') expect(host.textContent).toContain('Capture pending')
    }
    expect(network).not.toHaveBeenCalled()
  } finally { await act(async () => root.unmount()); host.remove() }
})
