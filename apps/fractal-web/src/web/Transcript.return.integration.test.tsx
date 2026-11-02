// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EmbraceScrollViewport, ViewportStore, ViewportStoreContext } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import type { ViewportAnchorHistory } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'

// The harness supplies only browser layout (uniform 100px rows in a 300px lane), frames and media queries;
// the real viewport controller owns following, easing and anchor restoration.
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

interface TestRow { readonly id: string; readonly members?: readonly string[] }
const plain = (prefix: string, count: number, from = 0) => Array.from({ length: count }, (_, index): TestRow => ({ id: `${prefix}${from + index}` }))
const summary = (turn: string, members: readonly string[]): TestRow => ({ id: JSON.stringify(['work-summary', turn]), members })
const rect = (top: number, height: number) => ({ top, bottom: top + height, left: 0, right: 600, width: 600, height, x: 0, y: top, toJSON() {} })
const laneRows = (lane: Element) => [...lane.querySelectorAll<HTMLElement>('[data-test-row]')]

let root: Root
let container: HTMLDivElement
let frames: Map<number, FrameRequestCallback>
let nextFrame: number
let matchMedia: PropertyDescriptor | undefined

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  frames = new Map()
  nextFrame = 0
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++nextFrame, callback); return nextFrame })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  const isLane = (element: HTMLElement) => element.dataset.testid === 'lane'
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) { return isLane(this) ? laneRows(this).length * 100 : 0 })
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (this: HTMLElement) { return isLane(this) ? 300 : 0 })
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => 600)
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const lane = this.closest<HTMLElement>('[data-testid="lane"]')
    if (lane === null || lane === this) return rect(0, 300)
    return rect(laneRows(lane).indexOf(this) * 100 - lane.scrollTop, 100)
  })
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: (_x: number, y: number) => {
    const lane = container.querySelector<HTMLElement>('[data-testid="lane"]')
    return lane === null ? null : laneRows(lane)[Math.floor((lane.scrollTop + y) / 100)] ?? null
  } })
  // The controller reads media queries from its document's window.
  matchMedia = Object.getOwnPropertyDescriptor(document.defaultView!, 'matchMedia')
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  Reflect.deleteProperty(document, 'elementFromPoint')
  if (matchMedia === undefined) Reflect.deleteProperty(document.defaultView!, 'matchMedia')
  else Object.defineProperty(document.defaultView!, 'matchMedia', matchMedia)
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** One browser frame at `time`, then React's resulting commits. */
const frame = async (time: number) => {
  await act(async () => {
    const pending = [...frames.values()]
    frames.clear()
    pending.forEach(callback => callback(time))
  })
}
const settle = async () => { for (let index = 0; index < 6 && frames.size > 0; index++) await frame(index * 16) }
const lane = () => container.querySelector<HTMLElement>('[data-testid="lane"]')!
/** Hosts with summaries supply source membership; by default this harness's source is exactly its rows and their members. */
const render = async (rows: readonly TestRow[], key: string, store: ViewportStore, anchorHistory: ViewportAnchorHistory = { _tag: 'Complete', includes: id => rows.some(row => row.id === id || row.members?.includes(id) === true) }) => {
  await act(async () => root.render(<ViewportStoreContext value={store}>
    <EmbraceScrollViewport items={rows} anchorHistory={anchorHistory} stateKey={key} data-testid="lane">
      {rows.map(row => row.members === undefined
        ? <div key={row.id} data-test-row data-item-id={row.id} />
        : <div key={row.id} data-test-row data-scroll-anchor-id={row.id} data-scroll-anchor-members={JSON.stringify(row.members)} />)}
    </EmbraceScrollViewport>
  </ViewportStoreContext>))
  await settle()
}
/** A reader wheel plus the scroll it causes. */
const readerScrollsTo = async (top: number) => {
  await act(async () => {
    lane().dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }))
    lane().scrollTop = top
    lane().dispatchEvent(new Event('scroll'))
  })
  await settle()
}
const pressJump = () => act(async () => { container.querySelector<HTMLButtonElement>('button')!.click() })
const motion = (reduce: boolean) => {
  const query = Object.assign(new EventTarget(), { media: '(prefers-reduced-motion: reduce)', matches: reduce, onchange: null, addListener() {}, removeListener() {} })
  Object.defineProperty(document.defaultView!, 'matchMedia', { configurable: true, value: (media: string) => media === query.media ? query : Object.assign(new EventTarget(), { media, matches: false }) })
  return query
}

describe('return to the end', () => {
  const detachedWithNews = async () => {
    const store = new ViewportStore()
    await render(plain('r', 20), 'a', store)
    expect(lane().scrollTop).toBe(1700)
    await readerScrollsTo(400)
    await render(plain('r', 21), 'a', store)
    expect(lane().scrollTop).toBe(400)
    return store
  }

  it('eases from the reader line to the live end over the standard duration', async () => {
    motion(false)
    const store = await detachedWithNews()
    await pressJump()
    await frame(1000)
    expect(lane().scrollTop, 'the first frame starts from the reader line').toBe(400)
    await frame(1100)
    // Ease-out cubic halfway: 400 + (1800 - 400) * 0.875.
    expect(lane().scrollTop).toBe(1625)
    await frame(1200)
    expect(lane().scrollTop).toBe(1800)
    await render(plain('r', 22), 'a', store)
    expect(lane().scrollTop, 'the return resumes following').toBe(1900)
  })

  it('stops where the reader interrupts it', async () => {
    motion(false)
    await detachedWithNews()
    await pressJump()
    await frame(1000)
    await frame(1100)
    expect(lane().scrollTop).toBe(1625)
    await act(async () => { lane().dispatchEvent(new WheelEvent('wheel', { deltaY: -40 })) })
    await frame(1150)
    await frame(1200)
    expect(lane().scrollTop, 'the eased return kept moving after reader input').toBe(1625)
  })

  it('returns at once under reduced motion, including when it turns on mid-return', async () => {
    const query = motion(true)
    await detachedWithNews()
    await pressJump()
    await frame(1000)
    expect(lane().scrollTop).toBe(1800)
    query.matches = false
    await readerScrollsTo(400)
    await pressJump()
    await frame(2000)
    await frame(2100)
    expect(lane().scrollTop).toBe(1625)
    query.matches = true
    await act(async () => { query.dispatchEvent(Object.assign(new Event('change'), { matches: true })) })
    expect(lane().scrollTop).toBe(1800)
  })
})

describe('source-member anchors', () => {
  // The summary at index 10 owns call-1; the reader's line sits 10px into it.
  const before = [...plain('r', 10), summary('t1', ['call-1', 'call-2']), ...plain('r', 9, 11)]
  // Backfill prepends rows and splits the turn: the summary's id changes but it still owns call-1, now at index 13.
  const backfilled = [...plain('e', 3), ...plain('r', 10), summary('t0', ['call-0', 'call-1', 'call-2']), ...plain('r', 9, 11)]
  const withoutSummary = [...plain('e', 3), ...plain('r', 10), ...plain('r', 9, 11)]
  const readSummary = async (store: ViewportStore) => {
    await render(before, 'a', store)
    await readerScrollsTo(1010)
    await render(plain('b', 20), 'b', store)
    expect(store.get('a')?.anchor).toEqual({ id: '["work-summary","t1"]', offset: -10, memberId: 'call-1' })
  }

  it.each(['switch', 'remount'] as const)('restores the summary that still owns the captured member after backfill (%s)', async path => {
    const store = new ViewportStore()
    await readSummary(store)
    if (path === 'remount') await act(async () => root.render(null))
    await render(backfilled, 'a', store)
    expect(lane().scrollTop, 'saved top alone drifts by the backfilled rows').toBe(1310)
    expect(lane().querySelector('[data-scroll-anchor-id]')!.getBoundingClientRect().top).toBe(-10)
  })

  it.each(['switch', 'remount'] as const)('preserves a non-first leaf through folding, splitting and expansion (%s)', async path => {
    const store = new ViewportStore()
    const leafRows = [...plain('r', 10), { id: 'tool-b' }, ...plain('r', 9, 11)]
    const foldedRows = [...plain('r', 10), summary('folded', ['tool-a', 'tool-b']), ...plain('r', 9, 11)]
    const splitRows = [...plain('e', 3), ...plain('r', 10), summary('split-a', ['tool-a']), summary('split-b', ['tool-b']), ...plain('r', 9, 11)]
    const leave = async () => {
      await render(plain('other', 20), 'other', store)
      if (path === 'remount') await act(async () => root.render(null))
    }
    await render(leafRows, 'a', store)
    await readerScrollsTo(1010)
    await leave()
    expect(store.get('a')?.anchor).toEqual({ id: 'tool-b', offset: -10 })
    await render(foldedRows, 'a', store)
    expect(lane().scrollTop).toBe(1010)
    await leave()
    expect(store.get('a')?.anchor).toEqual({ id: '["work-summary","folded"]', offset: -10, memberId: 'tool-b' })
    await render(splitRows, 'a', store)
    expect(lane().scrollTop).toBe(1410)
    await leave()
    expect(store.get('a')?.anchor).toEqual({ id: '["work-summary","split-b"]', offset: -10, memberId: 'tool-b' })
    await render(leafRows, 'a', store)
    expect(lane().scrollTop).toBe(1010)
    await leave()
    expect(store.get('a')?.anchor).toEqual({ id: 'tool-b', offset: -10 })
  })

  it('keeps a missing row pending while history is partial, then restores it when it mounts', async () => {
    const store = new ViewportStore()
    await readSummary(store)
    await render(withoutSummary, 'a', store, { _tag: 'Partial' })
    expect(lane().scrollTop).toBe(1010)
    await render(backfilled, 'a', store, { _tag: 'Partial' })
    expect(lane().scrollTop).toBe(1310)
  })

  it('retires an anchor absent from complete history and keeps the saved top', async () => {
    const store = new ViewportStore()
    await readSummary(store)
    await render(withoutSummary, 'a', store)
    expect(lane().scrollTop).toBe(1010)
    // A later row owning the old member must not pull the reader away from the line they now hold.
    await render(backfilled, 'a', store)
    expect(lane().scrollTop).toBe(1010)
  })
})
