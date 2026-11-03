// @vitest-environment jsdom
import { act } from 'react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import type { Mock } from 'vitest'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
// The same adapter replacement as vite.embed.config.ts, without changing the live app tests.
vi.mock('../state/storageAdapter.ts', () => import('./memoryStorageAdapter.ts'))
let network: Mock<() => never>
const beaconDescriptor = Object.getOwnPropertyDescriptor(navigator, 'sendBeacon')
beforeEach(() => {
  network = vi.fn(() => { throw new Error('Embed attempted network access') })
  class ForbiddenTransport { constructor() { network() } }
  vi.stubGlobal('fetch', network)
  vi.stubGlobal('WebSocket', ForbiddenTransport)
  vi.stubGlobal('XMLHttpRequest', ForbiddenTransport)
  vi.stubGlobal('EventSource', ForbiddenTransport)
  Object.defineProperty(navigator, 'sendBeacon', { configurable: true, value: network })
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('CSS', { escape: (value: string) => value.replaceAll('/', '\\/') })
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => setTimeout(fn, 0))
  vi.stubGlobal('cancelAnimationFrame', clearTimeout)
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  if (beaconDescriptor === undefined) Reflect.deleteProperty(navigator, 'sendBeacon')
  else Object.defineProperty(navigator, 'sendBeacon', beaconDescriptor)
})

it('mounts the production entry in every fragment/theme without transport or real storage access', async () => {
  window.localStorage.setItem('wf.ui@1', 'real-app-selection')
  const read = vi.spyOn(Storage.prototype, 'getItem')
  const write = vi.spyOn(Storage.prototype, 'setItem')
  const { mountEmbed } = await import('./entry.tsx')
  const { fragments } = await import('./Embed.tsx')
  const { persistence } = await import('../state/persistence.ts')
  const host = document.createElement('div')
  document.body.append(host)
  try {
    for (const fragment of fragments) for (const scheme of ['light', 'dark']) {
      window.history.replaceState(null, '', `/website/product/embed.html?theme=${scheme}#${fragment}`)
      let dispose = () => {}
      try {
        await act(async () => { dispose = mountEmbed(host); await new Promise(resolve => setTimeout(resolve, 100)) })
        expect(host.textContent).not.toBe('')
        expect(document.documentElement.dataset.scheme).toBe(scheme)
        if (fragment === 'codex' || fragment === 'claude-code') {
          expect(host.querySelector('main')).toBeNull()
          const section = host.querySelector('section.capture-pending')
          expect(section?.getAttribute('aria-labelledby')).toBe(section?.querySelector('h2')?.id)
          expect(section?.querySelector('[role="status"]')?.textContent).toContain('Capture pending')
        }
        const entry = (value: unknown) => ({ version: 1, value, clock: Number.MAX_SAFE_INTEGER, writer: 'other-frame' })
        window.dispatchEvent(new StorageEvent('storage', { key: 'wf.ui@1', newValue: JSON.stringify({ version: 1, entries: { 'round2.scheme': entry(scheme === 'dark' ? 'light' : 'dark') } }) }))
        await act(async () => persistence.flush())
        expect(document.documentElement.dataset.scheme).toBe(scheme)
      } finally { await act(async () => dispose()) }
    }
    expect(network).not.toHaveBeenCalled()
    expect(read).not.toHaveBeenCalled()
    expect(write).not.toHaveBeenCalled()
  } finally { host.remove() }
})

it('uses the narrow container inside a desktop window and keeps agent navigation out of browser history', async () => {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 })
  window.history.replaceState(null, '', '/website/product/embed.html?theme=dark#web-narrow')
  const before = window.location.href
  const historyLength = window.history.length
  const push = vi.spyOn(window.history, 'pushState')
  const { mountEmbed } = await import('./entry.tsx')
  const host = document.createElement('div')
  document.body.append(host)
  let dispose = () => {}
  try {
    await act(async () => { dispose = mountEmbed(host); await new Promise(resolve => setTimeout(resolve, 100)) })
    const expand = host.querySelector<HTMLButtonElement>('[aria-label="Expand sidebar"]')
    expect(expand).not.toBeNull()
    expect(host.querySelector('[aria-label="Collapse agents"]')).toBeNull()
    await act(async () => expand!.click())
    const title = host.querySelector<HTMLElement>('[data-wf-agent-ref="agent/build-host-a/palette-review"] [data-row-column="title"]')
    expect(title).not.toBeNull()
    await act(async () => title!.click())
    expect(window.location.href).toBe(before)
    expect(window.history.length).toBe(historyLength)
    expect(push).not.toHaveBeenCalled()
    expect(network).not.toHaveBeenCalled()
  } finally { await act(async () => dispose()); host.remove() }
})
