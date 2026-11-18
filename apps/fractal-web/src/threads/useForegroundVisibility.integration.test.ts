// @vitest-environment jsdom
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, expect, it, vi } from 'vitest'
import { foregroundVisibility } from './useForegroundVisibility.ts'
import { meaningfulBodyRegion } from './meaningfulGeometry.ts'

const disposals: (() => void)[] = []
afterEach(() => {
  for (const dispose of disposals.splice(0)) dispose()
  document.body.replaceChildren(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals()
})

const placement = (active = true) => {
  vi.useFakeTimers()
  let focused = true
  let visibility: DocumentVisibilityState = 'visible'
  vi.spyOn(document, 'hasFocus').mockImplementation(() => focused)
  vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility)
  const panel = document.createElement('section'), container = document.createElement('div')
  const row = document.createElement('article'), body = document.createElement('div')
  panel.setAttribute('data-thread-panel', '')
  row.setAttribute('data-thread-anchor', 'message/a')
  row.setAttribute('data-thread-read-eligible', '')
  body.setAttribute('data-thread-message-content', ''); body.textContent = 'Actual message body'; body.style.lineHeight = '20px'
  row.append(body); container.append(row); panel.append(container); document.body.append(panel)
  let rect = new DOMRect(10, 10, 100, 20)
  vi.spyOn(body, 'getBoundingClientRect').mockImplementation(() => rect)
  let hit: (x: number, y: number) => Element | null = () => body
  vi.stubGlobal('IntersectionObserver', class { observe() {} unobserve() {} disconnect() {} })
  let layout = () => {}
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: () => void) { layout = callback }
    observe() {} unobserve() {} disconnect() {}
  })
  // jsdom has no paint tree; only geometry and hit testing are substituted.
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: (x: number, y: number) => hit(x, y) })
  const registry = AtomRegistry.make(), atom = foregroundVisibility(container, active), release = registry.mount(atom)
  disposals.push(() => { release(); registry.dispose(); Reflect.deleteProperty(document, 'elementFromPoint') })
  return {
    panel, container, row, body,
    snapshot: () => registry.get(atom),
    rect: (next: DOMRect) => { rect = next },
    hit: (next: typeof hit) => { hit = next },
    layout: () => layout(),
    focus: (next: boolean) => { focused = next; window.dispatchEvent(new Event(next ? 'focus' : 'blur')) },
    visibility: (next: DocumentVisibilityState) => { visibility = next; document.dispatchEvent(new Event('visibilitychange')) },
  }
}

// Exact 500ms source: doc/threads-delivery/adopted-ux-20261116/unread-implementation,
// “Deterministic visibility default for implementation/review”. No real-clock waits.
it('preserves native DOMRect coordinates for the full-width trailing two-line region', () => {
  const body = new DOMRect(10, -200, 100, 260)
  const viewport = new DOMRect(0, 0, window.innerWidth, window.innerHeight)
  expect(meaningfulBodyRegion(body, 20, [viewport])).toEqual({ left: 10, right: 110, top: 20, bottom: 60 })
  expect(meaningfulBodyRegion(body, 20, [new DOMRect(11, 0, 100, 100)])).toBeUndefined()
  expect(meaningfulBodyRegion(body, 20, [new DOMRect(0, 21, 200, 100)])).toBeUndefined()
  expect(meaningfulBodyRegion(body, 20, [new DOMRect(0, 0, 200, 59)])).toBeUndefined()
})

it('publishes full one-line body only after 500ms, without polling', () => {
  const view = placement()
  vi.advanceTimersByTime(499); expect(view.snapshot().visibleMessageIds).toEqual([])
  vi.advanceTimersByTime(1); expect(view.snapshot().visibleMessageIds).toEqual(['message/a'])
  expect(vi.getTimerCount()).toBe(0)
})

it('does not starve an unchanged body when unrelated external text streams at 499ms', async () => {
  const view = placement()
  const external = document.createElement('aside')
  external.textContent = 'Unrelated clock'
  document.body.append(external)
  await Promise.resolve()
  vi.advanceTimersByTime(499)
  external.firstChild!.textContent = 'Unrelated streaming update'
  await Promise.resolve()
  vi.advanceTimersByTime(1)
  expect(view.snapshot().visibleMessageIds).toEqual(['message/a'])
})

it('restarts the dwell when the relevant body content changes without changing its geometry', async () => {
  const view = placement()
  vi.advanceTimersByTime(499)
  view.body.firstChild!.textContent = 'Changed message content'
  await Promise.resolve()
  vi.advanceTimersByTime(1)
  expect(view.snapshot().visibleMessageIds).toEqual([])
  vi.advanceTimersByTime(498)
  expect(view.snapshot().visibleMessageIds).toEqual([])
  vi.advanceTimersByTime(1)
  expect(view.snapshot().visibleMessageIds).toEqual(['message/a'])
})

it.each([
  ['one pixel', new DOMRect(10, window.innerHeight - 1, 100, 20)],
  ['header only', new DOMRect(10, window.innerHeight + 10, 100, 20)],
  ['clipped line', new DOMRect(10, window.innerHeight - 10, 100, 20)],
  ['top of long body', new DOMRect(10, 10, 100, window.innerHeight + 100)],
  ['partial width', new DOMRect(-1, 10, 100, 20)],
] as const)('never qualifies %s', (_name, rect) => {
  const view = placement(); view.rect(rect); view.layout()
  vi.advanceTimersByTime(1000); expect(view.snapshot().visibleMessageIds).toEqual([])
})

it('qualifies the final two lines of a long body', () => {
  const view = placement(); view.rect(new DOMRect(10, -200, 100, 260)); view.layout()
  vi.advanceTimersByTime(500); expect(view.snapshot().visibleMessageIds).toEqual(['message/a'])
})

it.each(['scroll', 'layout', 'focus', 'visibility'] as const)('restarts 499ms dwell on %s', event => {
  const view = placement(); vi.advanceTimersByTime(499)
  if (event === 'scroll') document.dispatchEvent(new Event('scroll'))
  if (event === 'layout') { view.rect(new DOMRect(10, 20, 100, 20)); view.layout() }
  if (event === 'focus') { view.focus(false); vi.advanceTimersByTime(500); expect(view.snapshot().foreground).toBe(false); view.focus(true) }
  if (event === 'visibility') { view.visibility('hidden'); vi.advanceTimersByTime(500); expect(view.snapshot().foreground).toBe(false); view.visibility('visible') }
  vi.advanceTimersByTime(499); expect(view.snapshot().visibleMessageIds).toEqual([])
  vi.advanceTimersByTime(1); expect(view.snapshot().visibleMessageIds).toEqual(['message/a'])
})

it('remeasures at expiry even if layout changed without an observer event', () => {
  const view = placement(); vi.advanceTimersByTime(499)
  view.rect(new DOMRect(10, 20, 100, 20))
  vi.advanceTimersByTime(1); expect(view.snapshot().visibleMessageIds).toEqual([])
  vi.advanceTimersByTime(500); expect(view.snapshot().visibleMessageIds).toEqual(['message/a'])
})

it('cancels for removed or replaced body, including identical replacement geometry', async () => {
  const view = placement(); vi.advanceTimersByTime(499)
  const replacement = view.body.cloneNode(true)
  if (!(replacement instanceof HTMLElement)) throw new Error('Expected a message body')
  view.body.replaceWith(replacement)
  await Promise.resolve()
  vi.advanceTimersByTime(500); expect(view.snapshot().visibleMessageIds).toEqual([])
  replacement.remove(); await Promise.resolve()
  expect(vi.getTimerCount()).toBe(0)
})

it('rejects keyboard visual viewport clipping', () => {
  vi.stubGlobal('visualViewport', Object.assign(new EventTarget(), { offsetLeft: 0, offsetTop: 0, width: 200, height: 25 }))
  const view = placement(); vi.advanceTimersByTime(500)
  expect(view.snapshot().visibleMessageIds).toEqual([])
})

it('rejects ancestor clipping, collapse and restores a fresh dwell', async () => {
  const view = placement()
  view.container.style.overflow = 'hidden'
  vi.spyOn(view.container, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 200, 25))
  Object.defineProperty(view.container, 'clientWidth', { configurable: true, value: 200 })
  Object.defineProperty(view.container, 'clientHeight', { configurable: true, value: 25 })
  await Promise.resolve(); vi.advanceTimersByTime(500); expect(view.snapshot().visibleMessageIds).toEqual([])
  view.container.style.overflow = 'visible'; view.panel.hidden = true
  await Promise.resolve(); vi.advanceTimersByTime(500); expect(view.snapshot().foreground).toBe(false)
  view.panel.hidden = false; await Promise.resolve()
  vi.advanceTimersByTime(499); expect(view.snapshot().visibleMessageIds).toEqual([])
  vi.advanceTimersByTime(1); expect(view.snapshot().visibleMessageIds).toEqual(['message/a'])
})

it('rejects partial composer occlusion away from the midpoint', () => {
  const view = placement(), composer = document.createElement('div')
  view.panel.append(composer)
  vi.spyOn(composer, 'getClientRects').mockReturnValue(Object.assign([new DOMRect(15, 15, 3, 3)], { item: () => null }))
  view.hit((x, y) => x >= 15 && x <= 18 && y >= 15 && y <= 18 ? composer : view.body)
  view.layout(); vi.advanceTimersByTime(500)
  expect(view.snapshot().visibleMessageIds).toEqual([])
})

it('suspends behind a dialog and on background; inactive placements never start timers', async () => {
  const view = placement()
  const dialog = document.createElement('div'); dialog.setAttribute('role', 'dialog'); document.body.append(dialog)
  await Promise.resolve(); vi.advanceTimersByTime(500); expect(view.snapshot().foreground).toBe(false)
  dialog.remove(); await Promise.resolve(); view.visibility('hidden')
  vi.advanceTimersByTime(500); expect(view.snapshot().visibleMessageIds).toEqual([])
})
it('does not schedule an inactive placement', () => {
  const view = placement(false)
  vi.advanceTimersByTime(1000)
  expect(view.snapshot()).toEqual({ foreground: false, visibleMessageIds: [] })
  expect(vi.getTimerCount()).toBe(0)
})

it('rejects a composer even when its cover is pointer-transparent', async () => {
  const view = placement(), composer = document.createElement('div')
  composer.setAttribute('data-thread-composer', ''); composer.style.pointerEvents = 'none'
  vi.spyOn(composer, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, 25, 100, 20))
  view.panel.append(composer); await Promise.resolve()
  vi.advanceTimersByTime(500); expect(view.snapshot().visibleMessageIds).toEqual([])
})

it('shares one cancellable timer across all message bodies and bounds the result to fifty IDs', async () => {
  const view = placement()
  // Count only the placement's 500ms timers, not AtomRegistry's own scheduler.
  const schedule = globalThis.setTimeout.bind(globalThis), cancel = globalThis.clearTimeout.bind(globalThis)
  const pending = new Set<NodeJS.Timeout | string | number>()
  vi.spyOn(window, 'setTimeout').mockImplementation((handler, delay, ...args) => {
    const id = schedule(() => { pending.delete(id); if (typeof handler === 'function') handler(...args) }, delay)
    if (delay === 500) pending.add(id)
    return id
  })
  vi.spyOn(window, 'clearTimeout').mockImplementation(id => { if (id !== undefined) pending.delete(id); cancel(id) })
  view.layout()
  const bodies: HTMLElement[] = [view.body]
  for (let index = 1; index <= 60; index++) {
    const row = view.row.cloneNode(true)
    if (!(row instanceof HTMLElement)) throw new Error('Expected a message row')
    row.setAttribute('data-thread-anchor', `message/${index}`)
    const body = row.querySelector<HTMLElement>('[data-thread-message-content]')!
    vi.spyOn(body, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, 10, 100, 20))
    bodies.push(body); view.container.append(row)
  }
  // A real viewport cannot paint coincident bodies. Place fixture bodies in separate
  // horizontal slots to exercise the placement scheduler, not overlapping text.
  bodies.forEach((body, index) => {
    vi.spyOn(body, 'getBoundingClientRect').mockReturnValue(new DOMRect(index * 3, 10, 2, 20))
  })
  view.hit((x) => bodies[Math.floor(x / 3)] ?? null)
  await Promise.resolve()
  expect(pending.size).toBe(1)
  vi.advanceTimersByTime(500)
  expect(view.snapshot().visibleMessageIds).toHaveLength(50)
  expect(pending.size).toBe(0)
  view.container.remove(); await Promise.resolve()
  expect(view.snapshot().visibleMessageIds).toEqual([])
  expect(pending.size).toBe(0)
})
