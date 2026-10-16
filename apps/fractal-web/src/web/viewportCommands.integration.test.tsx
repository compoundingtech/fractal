// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { EmbraceVirtualScrollViewport } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import type { ViewportRow, VirtualViewportHandle } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import { ScrollController } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-virtual/ScrollController.ts'
import { AnchoredListLayout } from '../../../../packages/fractal-ui/src/assistant-ui/composition/virtual-layout.ts'
import type { LayoutInfo } from 'react-stately/useVirtualizerState'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
class TestResizeObserver {
  static instances: TestResizeObserver[] = []
  readonly targets = new Set<Element>()
  constructor(readonly callback: ResizeObserverCallback) { TestResizeObserver.instances.push(this) }
  observe(target: Element) { this.targets.add(target) }
  unobserve(target: Element) { this.targets.delete(target) }
  disconnect() { this.targets.clear() }
  deliver() { this.callback([], this as unknown as ResizeObserver) }
}
let nextFrameId = 0
const frames = new Map<number, FrameRequestCallback>()
const flushFrame = async () => {
  const pending = [...frames.values()]
  frames.clear()
  await act(async () => { for (const callback of pending) callback(performance.now()) })
}
beforeEach(() => {
  TestResizeObserver.instances = []
  frames.clear()
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', TestResizeObserver)
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { const id = ++nextFrameId; frames.set(id, callback); return id })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
})
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); frames.clear() })
const geometryObserver = (lane: HTMLElement) => {
  const observer = TestResizeObserver.instances.find(value => value.targets.has(lane) && value.targets.has(lane.firstElementChild!))
  if (observer === undefined) throw new Error('Missing real controller geometry observer')
  return observer
}

it('publishes 31 metadata batches without replaying source commands, but preserves append, explicit follow and jump commits', async () => {
  const calls = vi.spyOn(ScrollController.prototype, 'afterRowsChange')
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const ref = React.createRef<VirtualViewportHandle>()
  const rows: readonly ViewportRow[] = Array.from({ length: 2000 }, (_, index) => ({ id: `command/${index}`, version: '1' }))
  const ids = new Set(rows.map(row => row.id))
  const history = { _tag: 'Complete' } as const
  const renderItem = (row: ViewportRow) => <p>{row.id}</p>
  const boundary = () => <span>Start</span>
  const view = (items = rows, scrollToBottomKey?: string) => <EmbraceVirtualScrollViewport ref={ref} items={items} sourceIds={ids} history={history} empty={null} boundary={boundary} renderItem={renderItem} scrollToBottomKey={scrollToBottomKey} aria-label="Source command test history" data-testid="command-lane" />
  try {
    await act(async () => root.render(view()))
    const lane = container.querySelector<HTMLElement>('[data-testid="command-lane"]')!
    expect(lane).not.toBeNull()
    expect(lane.dataset['embraceCollectionReady']).toBe('false')
    const initial = calls.mock.calls.length
    let publications = 0
    while (lane.dataset['embraceCollectionReady'] !== 'true' && publications < 40) {
      await flushFrame()
      publications++
    }
    expect(publications).toBe(31)
    expect(calls.mock.calls.length).toBe(initial)
    expect(initial).toBe(1)
    const counts = { initial, metadataPublications: publications, afterMetadata: calls.mock.calls.length, afterAppend: 0, afterExplicitFollow: 0, afterMissingJump: 0 }
    let height = 1000
    let top = 0
    Object.defineProperties(lane, {
      scrollHeight: { configurable: true, get: () => height },
      clientHeight: { configurable: true, get: () => 200 },
      scrollTop: { configurable: true, get: () => top, set: value => { top = Math.max(0, Math.min(Number(value), height - 200)) } },
    })
    const appended = [...rows, { id: 'command/appended', version: '1' }]
    await act(async () => root.render(view(appended)))
    expect(top).toBe(800)
    expect(calls.mock.calls.length).toBe(initial + 1)
    counts.afterAppend = calls.mock.calls.length
    // A geometry-only change is corrected synchronously by the existing observer,
    // before any queued animation frame, without a source command or added delay.
    height = 1200
    const beforeGeometry = calls.mock.calls.length
    geometryObserver(lane).deliver()
    expect(top).toBe(1000)
    expect(calls.mock.calls.length).toBe(beforeGeometry)
    await act(async () => root.render(view(appended, 'explicit-follow')))
    expect(calls.mock.calls.length).toBe(initial + 2)
    counts.afterExplicitFollow = calls.mock.calls.length
    let missing: Promise<{ readonly _tag: string }> | undefined
    await act(async () => { missing = ref.current!.jumpTo('command/absent') })
    expect(await missing).toEqual({ _tag: 'Missing' })
    expect(calls.mock.calls.length).toBe(initial + 3)
    counts.afterMissingJump = calls.mock.calls.length
    console.log('viewport-source-command-counts', JSON.stringify(counts))
  } finally {
    await act(async () => root.unmount())
    container.remove()
  }
// Real RAC adopts all 2000 metadata nodes in jsdom; this is not a timing gate.
}, 60000)

// jsdom has no layout engine. Supply deterministic geometry to the real controller;
// no runtime/data hook is replaced and observer delivery stays synchronous.
function controllerFixture() {
  const lane = document.createElement('div')
  const sizer = document.createElement('div')
  lane.append(sizer)
  document.body.append(lane)
  let top = 0
  let height = 600
  const positions = new Map<string, number>([['a', 0], ['b', 100], ['c', 200], ['d', 300]])
  const unavailable = new Set<string>()
  Object.defineProperties(lane, {
    scrollHeight: { get: () => height },
    clientHeight: { get: () => 200 },
    scrollTop: { get: () => top, set: value => { top = Math.max(0, Math.min(Number(value), height - 200)) } },
  })
  lane.getBoundingClientRect = () => new DOMRect(0, 0, 400, 200)
  const mount = (id: string) => {
    const row = document.createElement('div')
    row.dataset['embraceEntryId'] = id
    row.getBoundingClientRect = () => new DOMRect(0, (positions.get(id) ?? 0) - top, 400, 100)
    sizer.append(row)
    return row
  }
  const a = mount('a')
  const b = mount('b')
  const layout = new AnchoredListLayout()
  vi.spyOn(layout, 'getLayoutInfo').mockImplementation(key => {
    const y = positions.get(String(key))
    if (y === undefined || unavailable.has(String(key))) return null
    return { rect: new DOMRect(0, y, 400, 100) } as unknown as LayoutInfo
  })
  const controller = new ScrollController({ layout, initial: { _tag: 'Following' }, save: () => {}, setUnread: () => {} })
  const detach = controller.attach(lane)
  const rows = [{ id: 'a' }, { id: 'b' }, { id: 'c' }, { id: 'd' }]
  const ids = new Set(rows.map(row => row.id))
  controller.afterRowsChange(rows, undefined, ids)
  return { lane, controller, rows, ids, unavailable, positions, a, b, mount, observer: () => geometryObserver(lane), setHeight: (value: number) => { height = value }, dispose: () => { detach?.(); lane.remove() } }
}

it('keeps loaded-unadopted jumps pending, then anchors; later commands supersede and absent targets settle Missing', async () => {
  const fixture = controllerFixture()
  try {
    fixture.unavailable.add('c')
    let settled = false
    const loaded = fixture.controller.jumpTo('c').then(value => { settled = true; return value })
    fixture.controller.afterRowsChange(fixture.rows, undefined, fixture.ids)
    await Promise.resolve()
    expect(settled).toBe(false)
    fixture.unavailable.delete('c')
    fixture.mount('c')
    fixture.observer().deliver()
    expect(await loaded).toEqual({ _tag: 'Anchored' })
    expect(fixture.lane.scrollTop).toBe(200)
    fixture.unavailable.add('d')
    const previous = fixture.controller.jumpTo('d')
    const missing = fixture.controller.jumpTo('absent')
    fixture.controller.afterRowsChange(fixture.rows, undefined, fixture.ids)
    expect(await previous).toEqual({ _tag: 'Superseded' })
    expect(await missing).toEqual({ _tag: 'Missing' })
  } finally { fixture.dispose() }
})

it('preserves reading/prepend and held-press offsets through observer-only geometry before paint', () => {
  const fixture = controllerFixture()
  const pointer = (type: string) => {
    const event = new MouseEvent(type, { bubbles: true })
    Object.defineProperty(event, 'pointerId', { value: 7 })
    return event
  }
  try {
    fixture.lane.scrollTop = 100
    fixture.controller.read()
    fixture.controller.prepareForPrepend()
    const originalOffset = fixture.b.getBoundingClientRect().top
    fixture.positions.set('older', 0)
    for (const key of ['a', 'b', 'c', 'd']) fixture.positions.set(key, fixture.positions.get(key)! + 100)
    fixture.setHeight(700)
    fixture.mount('older')
    const prepended = [{ id: 'older' }, ...fixture.rows]
    fixture.controller.afterRowsChange(prepended, undefined, new Set(prepended.map(row => row.id)))
    fixture.observer().deliver()
    expect(fixture.b.getBoundingClientRect().top).toBe(originalOffset)
    expect(fixture.lane.scrollTop).toBe(200)
    expect(fixture.controller.position()._tag).toBe('Reading')
    fixture.b.dispatchEvent(pointer('pointerdown'))
    const pressedOffset = fixture.b.getBoundingClientRect().top
    for (const key of fixture.positions.keys()) fixture.positions.set(key, fixture.positions.get(key)! + 70)
    fixture.setHeight(770)
    fixture.observer().deliver()
    expect(fixture.b.getBoundingClientRect().top).toBe(pressedOffset)
    expect(fixture.lane.scrollTop).toBe(270)
    fixture.b.dispatchEvent(pointer('pointerup'))
    expect(fixture.controller.position()._tag).toBe('Reading')
  } finally { fixture.dispose() }
})
