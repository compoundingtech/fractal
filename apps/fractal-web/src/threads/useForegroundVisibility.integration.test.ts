// @vitest-environment jsdom
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, expect, it, vi } from 'vitest'
import { foregroundVisibility } from './useForegroundVisibility.ts'

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals() })

it('requires foreground focus, an active pane and actual intersections; dialogs suspend views', async () => {
  let focused = true
  vi.spyOn(document, 'hasFocus').mockImplementation(() => focused)
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
  let intersect: ((entries: IntersectionObserverEntry[]) => void) | undefined
  const disconnected = vi.fn()
  class Observer {
    constructor(callback: (entries: IntersectionObserverEntry[]) => void) { intersect = callback }
    observe() {}
    unobserve() {}
    disconnect = disconnected
  }
  vi.stubGlobal('IntersectionObserver', Observer)
  const container = document.createElement('div')
  const row = document.createElement('p')
  row.setAttribute('data-thread-message', 'message/a')
  row.textContent = 'Meaningful message'
  container.append(row); document.body.append(container)
  vi.spyOn(row, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, 10, 100, 50))
  const registry = AtomRegistry.make()
  const atom = foregroundVisibility(container, true)
  const release = registry.mount(atom)
  try {
    expect(registry.get(atom).visibleMessageIds).toEqual([])
    intersect?.([{ target: row, isIntersecting: true, intersectionRatio: 1 } as IntersectionObserverEntry])
    expect(registry.get(atom).visibleMessageIds).toEqual(['message/a'])
    focused = false; window.dispatchEvent(new Event('blur'))
    expect(registry.get(atom)).toEqual({ foreground: false, visibleMessageIds: [] })
    focused = true; window.dispatchEvent(new Event('focus'))
    expect(registry.get(atom).visibleMessageIds).toEqual(['message/a'])
    const dialog = document.createElement('div'); dialog.setAttribute('role', 'dialog'); document.body.append(dialog)
    await Promise.resolve()
    expect(registry.get(atom).foreground).toBe(false)
    dialog.remove(); await Promise.resolve()
    expect(registry.get(atom).foreground).toBe(true)
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    expect(registry.get(atom).visibleMessageIds).toEqual([])
    expect(registry.get(foregroundVisibility(container, false)).visibleMessageIds).toEqual([])
  } finally { release(); registry.dispose() }
  expect(disconnected).toHaveBeenCalled()
})

it('does not expose an offscreen, occluded or hidden-pane message', () => {
  vi.spyOn(document, 'hasFocus').mockReturnValue(true)
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
  let intersect: ((entries: IntersectionObserverEntry[]) => void) | undefined
  vi.stubGlobal('IntersectionObserver', class {
    constructor(callback: (entries: IntersectionObserverEntry[]) => void) { intersect = callback }
    observe() {} unobserve() {} disconnect() {}
  })
  const container = document.createElement('div'), row = document.createElement('p')
  row.setAttribute('data-thread-message', 'message/a'); container.append(row); document.body.append(container)
  vi.spyOn(row, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, -100, 100, 20))
  const registry = AtomRegistry.make(), atom = foregroundVisibility(container, true), release = registry.mount(atom)
  try {
    intersect?.([{ target: row, isIntersecting: true, intersectionRatio: 1 } as IntersectionObserverEntry])
    expect(registry.get(atom).visibleMessageIds).toEqual([])
    vi.spyOn(row, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, 10, 100, 20))
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => document.body })
    window.dispatchEvent(new Event('resize'))
    expect(registry.get(atom).visibleMessageIds).toEqual([])
    container.hidden = true; window.dispatchEvent(new Event('focus'))
    expect(registry.get(atom).foreground).toBe(false)
  } finally { release(); registry.dispose(); Reflect.deleteProperty(document, 'elementFromPoint') }
})
