import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { JSDOM } from 'jsdom'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

// The real React Aria Virtualizer, GridList and ScrollController under jsdom. jsdom has
// no layout, so this harness supplies geometry the way a browser would: RAC positions
// each row wrapper from its ListLayout, a row's natural height comes from the fixture,
// scroll writes clamp and deliver their scroll event in the next frame, and resize
// observations and animation frames are delivered explicitly.
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true })
const saved = new Map()
const install = (name, value) => {
  if (!saved.has(name)) saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value })
}
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'SVGElement', 'Element', 'Node', 'Document', 'DocumentFragment', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'MutationObserver', 'Event', 'KeyboardEvent', 'WheelEvent', 'FocusEvent', 'MouseEvent', 'UIEvent']) install(name, dom.window[name])
install('getComputedStyle', dom.window.getComputedStyle.bind(dom.window))
install('IS_REACT_ACT_ENVIRONMENT', true)
// Aria reads only fixed meta keys here; jsdom lacks CSS.escape.
install('CSS', { escape: value => value })
Object.defineProperty(dom.window.document, 'elementFromPoint', { configurable: true, value: () => null })

const laneHeight = 400
const laneWidth = 800
/** Natural row geometry by entry id: `height` plus leaves at a `top` inside the row. */
let model = new Map()
const observers = []
class ResizeObserverMock {
  targets = new Set()
  constructor(callback) { this.callback = callback; observers.push(this) }
  observe(target) { this.targets.add(target) }
  unobserve(target) { this.targets.delete(target) }
  disconnect() { this.targets.clear(); observers.splice(observers.indexOf(this), 1) }
}
install('ResizeObserver', ResizeObserverMock)
dom.window.ResizeObserver = ResizeObserverMock
let frames = new Map()
let frameId = 0
const requestFrame = callback => { frames.set(++frameId, callback); return frameId }
const cancelFrame = id => { frames.delete(id) }
install('requestAnimationFrame', requestFrame)
install('cancelAnimationFrame', cancelFrame)
dom.window.requestAnimationFrame = requestFrame
dom.window.cancelAnimationFrame = cancelFrame

const isLane = element => element.dataset?.testid === 'lane'
const laneOf = element => element.closest?.('[data-testid="lane"]') ?? null
/** RAC's absolutely positioned VirtualizerItem: a direct child of the lane's sizer. */
const wrapperOf = element => {
  for (let node = element; node !== null; node = node.parentElement) if (node.parentElement?.parentElement !== null && node.parentElement !== null && isLane(node.parentElement.parentElement)) return node
  return null
}
const entryOf = wrapper => wrapper.querySelector('[data-embrace-entry-id]')
const rowModel = wrapper => model.get(entryOf(wrapper)?.dataset.embraceEntryId) ?? { height: 0, leaves: [] }
const scrollTops = new WeakMap()
const queuedScrolls = new Set()
const { HTMLElement: DomHTMLElement, Element: DomElement } = dom.window
Object.defineProperty(DomHTMLElement.prototype, 'clientHeight', { configurable: true, get() { return isLane(this) ? laneHeight : 0 } })
Object.defineProperty(DomHTMLElement.prototype, 'clientWidth', { configurable: true, get() { return isLane(this) ? laneWidth : 0 } })
Object.defineProperty(DomElement.prototype, 'scrollWidth', { configurable: true, get() { return laneWidth } })
Object.defineProperty(DomElement.prototype, 'scrollHeight', { configurable: true, get() {
  if (isLane(this)) return Math.max(laneHeight, Number.parseFloat(this.firstElementChild?.style.height ?? '') || 0)
  // RAC measures a row wrapper's natural size after clearing its fixed height.
  if (this.style?.position === 'absolute' && entryOf(this) !== null) return rowModel(this).height
  return 0
} })
Object.defineProperty(DomElement.prototype, 'scrollTop', { configurable: true,
  get() { return scrollTops.get(this) ?? 0 },
  set(value) {
    const next = Math.max(0, Math.min(value, this.scrollHeight - this.clientHeight))
    if (next === (scrollTops.get(this) ?? 0)) return
    scrollTops.set(this, next)
    queuedScrolls.add(this)
  },
})
const rect = (top, height) => ({ top, bottom: top + height, left: 0, right: laneWidth, width: laneWidth, height, x: 0, y: top, toJSON() { return this } })
DomElement.prototype.getBoundingClientRect = function () {
  if (isLane(this)) return rect(0, laneHeight)
  const lane = laneOf(this)
  if (lane === null) return rect(0, 0)
  const wrapper = wrapperOf(this)
  if (wrapper === null) return rect(-lane.scrollTop, lane.scrollHeight)
  const top = Number.parseFloat(wrapper.style.top) - lane.scrollTop
  const row = rowModel(wrapper)
  const leafId = this.dataset.scrollAnchorId ?? this.dataset.itemId
  const leaf = leafId === undefined ? undefined : row.leaves.find(candidate => candidate.id === leafId)
  return leaf === undefined ? rect(top, row.height) : rect(top + leaf.top, leaf.height)
}

const server = await createServer({
  configFile: false,
  root: new URL('../../', import.meta.url).pathname,
  plugins: [stylex.vite({ useCSSLayers: false })],
  server: { middlewareMode: true },
})
const { EmbraceVirtualScrollViewport, ViewportStore, ViewportStoreContext } = await server.ssrLoadModule('/src/assistant-ui/EmbraceScrollViewport.tsx')
const { WorkLogV1 } = await server.ssrLoadModule('/src/assistant-ui/taste/WorkLogV1.tsx')

let root
let container
let handle
let store
let current
let now = 1000
const lane = () => container.querySelector('[data-testid="lane"]')
/** One browser frame: queued scroll events, then animation frames. */
const frame = async () => {
  await act(async () => {
    const scrolled = [...queuedScrolls]
    queuedScrolls.clear()
    for (const element of scrolled) element.dispatchEvent(new Event('scroll'))
    const pending = [...frames.values()]
    frames.clear()
    for (const callback of pending) callback(now)
  })
}
/** One resize-observer delivery, in observer creation order, before the frame paints. */
const observe = async () => {
  await act(async () => {
    for (const observer of [...observers]) if (observer.targets.size > 0) observer.callback([...observer.targets].map(target => ({ target })), observer)
  })
}
/** Asynchronous layout settles over several frames: observation, RAC relayout, observation. */
const settle = async (rounds = 6) => {
  for (let round = 0; round < rounds; round++) {
    await observe()
    await frame()
  }
}
const toEnd = element => element.scrollHeight - element.clientHeight - element.scrollTop
const leafTop = id => {
  const leaf = [...lane().querySelectorAll('[data-scroll-anchor-id], [data-item-id]')].find(candidate => (candidate.dataset.scrollAnchorId ?? candidate.dataset.itemId) === id)
  return leaf === undefined ? undefined : leaf.getBoundingClientRect().top - lane().getBoundingClientRect().top
}
const entryTop = id => {
  const entry = [...lane().querySelectorAll('[data-embrace-entry-id]')].find(candidate => candidate.dataset.embraceEntryId === id)
  return entry === undefined ? undefined : entry.getBoundingClientRect().top - lane().getBoundingClientRect().top
}

const Leaf = ({ leaf }) => React.createElement('div', leaf.members === undefined
  ? { 'data-item-id': leaf.id }
  : { 'data-scroll-anchor-id': leaf.id, 'data-scroll-anchor-members': JSON.stringify(leaf.members) }, leaf.id)
const renderItem = row => React.createElement('section', { 'data-item-id': row.id }, ...(model.get(row.id)?.leaves ?? []).map(leaf => React.createElement(Leaf, { key: leaf.id, leaf })))
/** Fixture rows: `{ id, height, version?, leaves?: [{ id, top, height, members? }] }`. */
const project = rows => {
  model = new Map(rows.map(row => [row.id, { height: row.height, leaves: row.leaves ?? [] }]))
  model.set('__fractal_history__', { height: 40, leaves: [] })
  return rows.map(row => ({ id: row.id, version: row.version ?? `${row.id}:${row.height}`, targetIds: (row.leaves ?? []).flatMap(leaf => leaf.members ?? [leaf.id]) }))
}
const render = async ({ rows, key = 'conversation', history = { _tag: 'Complete' }, sourceIds, strict = false }) => {
  const items = rows === undefined ? current.items : project(rows)
  current = { items, key, history, sourceIds, strict }
  const viewport = React.createElement(EmbraceVirtualScrollViewport, {
    key, ref: handle, items, sourceIds, stateKey: key, history, renderItem,
    boundary: () => null, empty: null, 'data-testid': 'lane', 'aria-label': 'Conversation history',
  })
  const tree = React.createElement(ViewportStoreContext.Provider, { value: store }, viewport)
  await act(async () => root.render(strict ? React.createElement(React.StrictMode, null, tree) : tree))
  await settle()
  return lane()
}
/** Same rows, new natural heights: content remeasures without a new host snapshot. */
const remeasure = changes => {
  for (const [id, height] of Object.entries(changes)) model.get(id).height = height
}
const turns = (count, height = 200) => Array.from({ length: count }, (_, index) => ({ id: `turn/${index}`, height }))
const readerScroll = async (element, top, input = new WheelEvent('wheel', { deltaY: top < element.scrollTop ? -120 : 120, bubbles: true })) => {
  await act(async () => {
    element.dispatchEvent(input)
    element.scrollTop = top
  })
  await frame()
  await act(async () => element.dispatchEvent(new Event('scrollend')))
}

beforeEach(() => {
  now = 1000
  handle = React.createRef()
  store = new ViewportStore()
  frames = new Map()
  queuedScrolls.clear()
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
})
after(async () => {
  await server.close()
  dom.window.close()
  for (const [name, descriptor] of saved) {
    if (descriptor === undefined) Reflect.deleteProperty(globalThis, name)
    else Object.defineProperty(globalThis, name, descriptor)
  }
})

test('the harness mounts only the rows in the visible window and starts at the end', async () => {
  const element = await render({ rows: turns(40) })
  assert.equal(element.scrollHeight, 40 + 40 * 200)
  assert.equal(toEnd(element), 0)
  const mounted = element.querySelectorAll('[data-embrace-entry-id]').length
  assert.ok(mounted > 0 && mounted < 40, `virtualized rows: ${mounted}`)
})

test('asynchronous growth of the last row keeps the end pinned through every settle', async () => {
  const element = await render({ rows: turns(12) })
  for (const height of [260, 420, 640, 900]) {
    // Fonts, highlighting or an image finish after the row first measured.
    remeasure({ 'turn/11': height })
    await settle()
    assert.equal(toEnd(element), 0, `last row ${height}`)
  }
})

test('resize delivery repins before the next animation frame once the sizer grows', async () => {
  const element = await render({ rows: turns(12) })
  remeasure({ 'turn/11': 520 })
  // First delivery: RAC remeasures the row and relays out the sizer.
  await observe()
  // Second delivery observes the grown sizer; no animation frame runs in between.
  await act(async () => {
    for (const observer of [...observers]) if (observer.targets.size > 0) observer.callback([...observer.targets].map(target => ({ target })), observer)
    assert.equal(toEnd(element), 0)
  })
})

test('remeasurement above the viewport keeps following', async () => {
  const element = await render({ rows: turns(12) })
  remeasure({ 'turn/0': 900, 'turn/1': 50 })
  await settle()
  assert.equal(toEnd(element), 0)
})

test('streaming growth with layout-generated scroll events stays at the end', async () => {
  const element = await render({ rows: turns(12) })
  for (let step = 0; step < 40; step++) {
    remeasure({ 'turn/11': 200 + 24 * (step + 1) })
    await observe()
    // Content remeasurement can publish a native scroll before the next observation;
    // no wheel, touch, key or held pointer asked for this movement.
    await act(async () => { element.scrollTop -= step % 3 === 0 ? 20 : 8 })
    await settle(3)
    assert.equal(toEnd(element), 0, `stream step ${step}`)
  }
})

test('native shrink clamping stays at the end and follows later growth', async () => {
  const element = await render({ rows: turns(12) })
  remeasure({ 'turn/11': 60 })
  await settle()
  assert.equal(toEnd(element), 0)
  remeasure({ 'turn/11': 700 })
  await settle()
  assert.equal(toEnd(element), 0)
})

test('reader scroll-up stays detached at its row through asynchronous growth', async () => {
  const element = await render({ rows: turns(12) })
  await readerScroll(element, 1000)
  await settle()
  const top = entryTop('turn/5')
  assert.equal(element.scrollTop, 1000)
  remeasure({ 'turn/11': 900 })
  await settle()
  assert.equal(element.scrollTop, 1000)
  assert.equal(entryTop('turn/5'), top)
  // Negative control: the same growth while following does move the coordinate.
  assert.notEqual(toEnd(element), 0)
})

test('detached reading compensates remeasurement above its row', async () => {
  const element = await render({ rows: turns(12) })
  await readerScroll(element, 1000)
  await settle()
  const top = entryTop('turn/5')
  remeasure({ 'turn/1': 600 })
  await settle()
  assert.equal(entryTop('turn/5'), top)
  assert.equal(element.scrollTop, 1400)
})

test('a held pointer drag remains reader intent after more than 250 ms', async t => {
  t.mock.method(performance, 'now', () => now)
  const element = await render({ rows: turns(12) })
  const down = new Event('pointerdown', { bubbles: true })
  Object.defineProperty(down, 'pointerId', { value: 1 })
  await act(async () => element.dispatchEvent(down))
  now += 500
  await act(async () => { element.scrollTop = 700 })
  await frame()
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(element.scrollTop, 700)
  const up = new Event('pointerup', { bubbles: true })
  Object.defineProperty(up, 'pointerId', { value: 1 })
  await act(async () => document.dispatchEvent(up))
})

for (const input of ['wheel', 'touchmove']) test(`${input} momentum reaching the end after 250 ms resumes following`, async t => {
  t.mock.method(performance, 'now', () => now)
  const element = await render({ rows: turns(12) })
  await act(async () => {
    element.dispatchEvent(input === 'wheel' ? new WheelEvent('wheel', { deltaY: -900 }) : new Event('touchmove'))
    element.scrollTop = 700
  })
  await frame()
  now += 500
  await act(async () => { element.scrollTop = element.scrollHeight })
  await frame()
  await act(async () => element.dispatchEvent(new Event('scrollend')))
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(toEnd(element), 0)
})

for (const input of ['wheel down', 'touchmove', 'Tab', 'pointerdown']) test(`${input} without an actual reader scroll keeps following`, async () => {
  const element = await render({ rows: turns(12) })
  const event = input === 'wheel down' ? new WheelEvent('wheel', { deltaY: 600, bubbles: true })
    : input === 'Tab' ? new KeyboardEvent('keydown', { key: 'Tab', bubbles: true })
    : new Event(input, { bubbles: true })
  if (input === 'pointerdown') Object.defineProperty(event, 'pointerId', { value: 1 })
  await act(async () => element.dispatchEvent(event))
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(toEnd(element), 0)
})

for (const input of ['wheel up', 'ArrowUp', 'Home']) test(`${input} with nothing above a short complete conversation keeps following`, async () => {
  const element = await render({ rows: turns(1, 120) })
  assert.equal(element.scrollTop, 0)
  const event = input === 'wheel up' ? new WheelEvent('wheel', { deltaY: -600, bubbles: true }) : new KeyboardEvent('keydown', { key: input, bubbles: true })
  const target = input === 'wheel up' ? element : element.querySelector('[data-embrace-entry-id]')
  await act(async () => target.dispatchEvent(event))
  // A long reply streams in: the conversation now overflows the lane.
  remeasure({ 'turn/0': 1400 })
  await settle()
  assert.equal(toEnd(element), 0)
})

test('an upward gesture at the top of a pageable conversation is older-page intent, not following', async () => {
  let requests = 0
  const element = await render({ rows: turns(1, 120), history: { _tag: 'More', loadOlder: async () => { requests++ } } })
  await act(async () => element.dispatchEvent(new WheelEvent('wheel', { deltaY: -600, bubbles: true })))
  remeasure({ 'turn/0': 1400 })
  await settle()
  assert.ok(requests > 0)
  // Negative control for the test above: the reader asked for older history.
  assert.equal(element.scrollTop, 0)
  assert.notEqual(toEnd(element), 0)
})

test('find-in-page without reader input keeps a following lane following', async () => {
  const element = await render({ rows: turns(12) })
  await act(async () => { element.scrollTop = 900 })
  await frame()
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(toEnd(element), 0)
})

for (const detached of [false, true]) test(`programmatic jumpTo from ${detached ? 'detached reading' : 'following'} keeps its target through later growth`, async () => {
  const element = await render({ rows: turns(12) })
  if (detached) {
    await readerScroll(element, 1000)
    await settle()
  }
  let result
  await act(async () => { handle.current.jumpTo('turn/3').then(value => { result = value }) })
  await settle()
  assert.deepEqual(result, { _tag: 'Anchored' })
  assert.equal(entryTop('turn/3'), 0)
  remeasure({ 'turn/11': 900 })
  await settle()
  assert.equal(entryTop('turn/3'), 0)
})

test('StrictMode double mount keeps one live controller that follows and detaches normally', async () => {
  const element = await render({ rows: turns(12), strict: true })
  remeasure({ 'turn/11': 600 })
  await settle()
  assert.equal(toEnd(element), 0)
  await readerScroll(element, 700)
  await settle()
  remeasure({ 'turn/11': 900 })
  await settle()
  assert.equal(element.scrollTop, 700)
})

// Member anchors: a leading partial turn gains its prompt or earlier work when older
// history arrives, which renames the turn (or splits it while the old id survives).
const summary = id => JSON.stringify(['work-summary', id])
const away = [{ id: 'away/0', height: 900 }]
const readOnly = rows => rows.map(row => ({ ...row }))

test('switching back restores a renamed turn through the summary member it captured', async () => {
  const before = [
    { id: 'turn/tool-x', height: 600, leaves: [{ id: summary('turn/tool-x'), top: 0, height: 32, members: ['tool-x'] }, { id: 'text-a', top: 40, height: 560 }] },
    ...turns(8).map(row => ({ ...row, id: `later/${row.id}` })),
  ]
  const element = await render({ rows: before, history: { _tag: 'More', loadOlder: async () => {} } })
  await readerScroll(element, 0 + 40 - 24 + 40)
  await settle()
  const offset = leafTop('text-a')
  await render({ rows: away, key: 'away' })
  const saved = store.get('conversation').anchor
  assert.equal(saved.entry, 'turn/tool-x')
  assert.deepEqual(saved.member, { id: 'text-a', offset, summary: false })

  // Older history: the turn gains its prompt and an earlier tool, and older turns arrive.
  const after = [
    ...turns(6).map(row => ({ ...row, id: `older/${row.id}` })),
    { id: 'turn/prompt-p', height: 900, leaves: [{ id: 'prompt-p', top: 0, height: 100 }, { id: summary('turn/prompt-p'), top: 120, height: 32, members: ['earlier-tool', 'tool-x'] }, { id: 'text-a', top: 340, height: 560 }] },
    ...before.slice(1),
  ]
  await render({ rows: after, history: { _tag: 'Complete' } })
  assert.equal(leafTop('text-a'), offset)
  await render({ rows: away, key: 'away' })
  assert.equal(store.get('conversation').anchor.entry, 'turn/prompt-p')
  assert.deepEqual(store.get('conversation').anchor.member, { id: 'text-a', offset, summary: false })
})

test('a folded summary member wins over a surviving old id when a split moves it', async () => {
  const reading = [
    ...turns(4).map(row => ({ ...row, id: `older/${row.id}` })),
    { id: 'turn/tool-a', height: 400, leaves: [{ id: summary('turn/tool-a'), top: 0, height: 32, members: ['tool-a', 'tool-b'] }, { id: 'reply-a', top: 40, height: 360 }] },
    ...turns(6).map(row => ({ ...row, id: `later/${row.id}` })),
  ]
  const element = await render({ rows: reading })
  // The reader's line is the summary of tool-a's turn.
  await readerScroll(element, 40 + 4 * 200 - 24)
  await settle()
  await render({ rows: away, key: 'away' })
  assert.deepEqual(store.get('conversation').anchor.member, { id: 'tool-a', offset: 24, summary: true })

  // A prompt arrives between the tools: tool-b now owns a new turn. The member tool-a
  // stays with the surviving summary, which moved down by the inserted rows.
  const split = [
    ...turns(4).map(row => ({ ...row, id: `older/${row.id}` })),
    { id: 'turn/prompt-q', height: 300, leaves: [{ id: 'prompt-q', top: 0, height: 100 }] },
    { id: 'turn/tool-a', height: 400, leaves: [{ id: summary('turn/tool-a'), top: 0, height: 32, members: ['tool-a'] }, { id: 'reply-a', top: 40, height: 360 }] },
    { id: 'turn/prompt-p', height: 200, leaves: [{ id: summary('turn/prompt-p'), top: 0, height: 32, members: ['tool-b'] }] },
    ...turns(6).map(row => ({ ...row, id: `later/${row.id}` })),
  ]
  await render({ rows: split })
  assert.equal(leafTop(summary('turn/tool-a')), 24)

  // Now the member itself moves: the reader was on tool-b's folded row.
  await render({ rows: away, key: 'away' })
  store.save('conversation', { ...store.get('conversation'), anchor: { ...store.get('conversation').anchor, member: { id: 'tool-b', offset: 24, summary: true } } })
  const moved = split.map(row => row.id === 'turn/tool-a' ? { ...row, leaves: [{ id: summary('turn/tool-a'), top: 0, height: 32, members: ['tool-a'] }, { id: 'reply-a', top: 40, height: 360 }] } : row)
  await render({ rows: readOnly(moved) })
  assert.equal(leafTop(summary('turn/prompt-p')), 24)
  await render({ rows: away, key: 'away' })
  assert.equal(store.get('conversation').anchor.entry, 'turn/prompt-p')
})

test('a mounted reader keeps its line when backfill renames the turn under it', async () => {
  const before = [
    { id: 'turn/tool-x', height: 600, leaves: [{ id: summary('turn/tool-x'), top: 0, height: 32, members: ['tool-x'] }, { id: 'text-a', top: 40, height: 560 }] },
    ...turns(8).map(row => ({ ...row, id: `later/${row.id}` })),
  ]
  const element = await render({ rows: before, history: { _tag: 'More', loadOlder: async () => {} } })
  await readerScroll(element, 56)
  await settle()
  const offset = leafTop('text-a')
  const renamed = [
    ...turns(10).map(row => ({ ...row, id: `older/${row.id}` })),
    { id: 'turn/prompt-p', height: 900, leaves: [{ id: 'prompt-p', top: 0, height: 100 }, { id: summary('turn/prompt-p'), top: 120, height: 32, members: ['earlier-tool', 'tool-x'] }, { id: 'text-a', top: 340, height: 560 }] },
    ...before.slice(1),
  ]
  await render({ rows: renamed, history: { _tag: 'Complete' }, key: 'conversation' })
  assert.equal(leafTop('text-a'), offset)
  assert.equal(element.scrollTop, 10 * 200 + 40 + 340 - offset)
})

test('a member outside the rendered window resolves through full row membership', async () => {
  const reading = [
    { id: 'turn/tool-x', height: 400, leaves: [{ id: summary('turn/tool-x'), top: 0, height: 32, members: ['tool-x'] }, { id: 'text-a', top: 40, height: 360 }] },
    ...turns(4).map(row => ({ ...row, id: `later/${row.id}` })),
  ]
  const element = await render({ rows: reading, history: { _tag: 'More', loadOlder: async () => {} } })
  await readerScroll(element, 80)
  await settle()
  const offset = leafTop('text-a')
  await render({ rows: away, key: 'away' })
  // Sixty older turns arrive above: the saved scrollTop now lands far from the renamed row,
  // which RAC has not mounted when the restore begins.
  const backfilled = [
    ...turns(60).map(row => ({ ...row, id: `older/${row.id}` })),
    { id: 'turn/prompt-p', height: 700, leaves: [{ id: 'prompt-p', top: 0, height: 100 }, { id: summary('turn/prompt-p'), top: 120, height: 32, members: ['earlier-tool', 'tool-x'] }, { id: 'text-a', top: 340, height: 360 }] },
    ...reading.slice(1),
  ]
  await render({ rows: backfilled, history: { _tag: 'Complete' } })
  assert.equal(leafTop('text-a'), offset)
})

test('a loaded member awaiting adoption is not retired; an absent member falls back', async () => {
  const reading = [
    { id: 'turn/tool-x', height: 400, leaves: [{ id: summary('turn/tool-x'), top: 0, height: 32, members: ['tool-x'] }, { id: 'text-a', top: 40, height: 360 }] },
    ...turns(4).map(row => ({ ...row, id: `later/${row.id}` })),
  ]
  const element = await render({ rows: reading, history: { _tag: 'More', loadOlder: async () => {} } })
  await readerScroll(element, 80)
  await settle()
  const offset = leafTop('text-a')
  const top = element.scrollTop
  await render({ rows: away, key: 'away' })
  const renamed = { id: 'turn/prompt-p', height: 700, leaves: [{ id: 'prompt-p', top: 0, height: 100 }, { id: summary('turn/prompt-p'), top: 120, height: 32, members: ['earlier-tool', 'tool-x'] }, { id: 'text-a', top: 340, height: 360 }] }
  const later = reading.slice(1)
  const older = turns(6).map(row => ({ ...row, id: `older/${row.id}` }))
  // The renamed turn's prompt is not adopted yet, so its turn is not committed; the source has it.
  const sourceIds = new Set(['prompt-p', 'earlier-tool', 'tool-x', 'text-a'])
  await render({ rows: [...older, ...later], sourceIds })
  await render({ rows: [...older, renamed, ...later], sourceIds })
  assert.equal(leafTop('text-a'), offset)
  // Negative control: without source membership the saved line is gone and the saved coordinate wins.
  await render({ rows: away, key: 'away' })
  store.save('conversation', { ...store.get('conversation'), anchor: { ...store.get('conversation').anchor, entry: 'turn/gone', member: { id: 'gone', offset: 24, summary: false } }, top })
  await render({ rows: [...older, renamed, ...later], sourceIds })
  assert.equal(element.isConnected ? lane().scrollTop : lane().scrollTop, top)
})

test('WorkLogV1 exposes its summary identity and full source members for scroll memory', () => {
  const turn = { calls: [{ id: 'tool-x', kind: 'run', title: 'Run', status: 'success', startedAt: '2026-01-15T12:00:00Z', endedAt: '2026-01-15T12:00:01Z' }], durationMs: 1000, running: false, failed: false, interrupted: false }
  for (const interactiveCalls of [true, false]) {
    const markup = renderToStaticMarkup(React.createElement(WorkLogV1, { turn, interactiveCalls, summaryAnchorId: summary('turn/prompt-p'), summaryMemberIds: ['earlier-tool', 'tool-x'] }))
    assert.ok(markup.includes(`data-scroll-anchor-id="${summary('turn/prompt-p').replaceAll('"', '&quot;')}"`))
    assert.ok(markup.includes(`data-scroll-anchor-members="${JSON.stringify(['earlier-tool', 'tool-x']).replaceAll('"', '&quot;')}"`))
  }
  const plain = renderToStaticMarkup(React.createElement(WorkLogV1, { turn }))
  assert.ok(!plain.includes('data-scroll-anchor'))
})
