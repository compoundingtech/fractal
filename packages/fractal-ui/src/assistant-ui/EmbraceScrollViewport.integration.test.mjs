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
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'SVGElement', 'Element', 'Node', 'NodeFilter', 'Text', 'ShadowRoot', 'Document', 'DocumentFragment', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'HTMLAnchorElement', 'HTMLIFrameElement', 'MutationObserver', 'Event', 'CustomEvent', 'KeyboardEvent', 'WheelEvent', 'FocusEvent', 'MouseEvent', 'PointerEvent', 'UIEvent', 'DOMRect']) if (dom.window[name] !== undefined) install(name, dom.window[name])
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
  assert.notEqual(toEnd(element), 0)
  // Second delivery observes the grown sizer; no animation frame runs in between.
  await act(async () => {
    for (const observer of [...observers]) if (observer.targets.size > 0) observer.callback([...observer.targets].map(target => ({ target })), observer)
    assert.equal(toEnd(element), 0)
  })
})

test('remeasurement of mounted rows above the last keeps following', async () => {
  const element = await render({ rows: turns(12) })
  remeasure({ 'turn/9': 900, 'turn/10': 50 })
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

/** Bring a row to the viewport top, then let the reader scroll so the row top sits at `offset`. */
const readAt = async (element, id, offset = 0) => {
  await act(async () => { void handle.current.jumpTo(id) })
  await settle()
  assert.equal(entryTop(id), 0)
  if (offset !== 0) {
    await readerScroll(element, element.scrollTop - offset)
    await settle()
  }
  assert.equal(entryTop(id), offset)
}

test('reader scroll-up stays detached at its row through asynchronous growth', async () => {
  const element = await render({ rows: turns(12) })
  await readerScroll(element, element.scrollTop - 900)
  await settle()
  const top = element.scrollTop
  const [row] = [...element.querySelectorAll('[data-embrace-entry-id]')].filter(entry => entryTop(entry.dataset.embraceEntryId) <= 0).slice(-1)
  const id = row.dataset.embraceEntryId
  const offset = entryTop(id)
  remeasure({ 'turn/11': 900 })
  await settle()
  assert.equal(element.scrollTop, top)
  assert.equal(entryTop(id), offset)
  assert.notEqual(toEnd(element), 0)
})

test('detached reading keeps its row when an older page is inserted above it', async () => {
  const element = await render({ rows: turns(12).slice(4), history: { _tag: 'More', loadOlder: async () => {} } })
  await readAt(element, 'turn/7', -30)
  await render({ rows: turns(12), history: { _tag: 'Complete' } })
  assert.equal(entryTop('turn/7'), -30)
  remeasure({ 'turn/11': 900 })
  await settle()
  assert.equal(entryTop('turn/7'), -30)
})

const pointer = (type, clientY) => {
  const event = new MouseEvent(type, { bubbles: true, clientY })
  Object.defineProperty(event, 'pointerId', { value: 1 })
  Object.defineProperty(event, 'pointerType', { value: 'mouse' })
  return event
}
const touch = (type, clientY) => {
  const event = new Event(type, { bubbles: true })
  Object.defineProperty(event, 'touches', { value: clientY === undefined ? [] : [{ clientY }] })
  return event
}

test('a held scrollbar drag remains reader intent after more than 250 ms', async t => {
  t.mock.method(performance, 'now', () => now)
  const element = await render({ rows: turns(12) })
  await act(async () => element.dispatchEvent(pointer('pointerdown', 300)))
  await act(async () => document.dispatchEvent(pointer('pointermove', 260)))
  now += 500
  await act(async () => { element.scrollTop -= 700 })
  await frame()
  const top = element.scrollTop
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(element.scrollTop, top)
  assert.notEqual(toEnd(element), 0)
  await act(async () => document.dispatchEvent(pointer('pointerup', 260)))
})

for (const input of ['wheel', 'touch']) test(`${input} momentum reaching the end after 250 ms resumes following`, async t => {
  t.mock.method(performance, 'now', () => now)
  const element = await render({ rows: turns(12) })
  await readAt(element, 'turn/6', -40)
  await act(async () => {
    if (input === 'wheel') element.dispatchEvent(new WheelEvent('wheel', { deltaY: 900, bubbles: true }))
    else {
      element.dispatchEvent(touch('touchstart', 300))
      element.dispatchEvent(touch('touchmove', 200))
    }
    element.scrollTop += 300
  })
  await frame()
  now += 500
  // The fling's last frames arrive without new input, past the recent-input window.
  await act(async () => { element.scrollTop = element.scrollHeight })
  await frame()
  await act(async () => element.dispatchEvent(new Event('scrollend')))
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(toEnd(element), 0)
})

test('a scroll after scrollend and outside the input window is layout, not reading', async t => {
  t.mock.method(performance, 'now', () => now)
  const element = await render({ rows: turns(12) })
  await readAt(element, 'turn/6', -40)
  assert.notEqual(toEnd(element), 0)
  now += 500
  // Negative control for momentum: once the gesture ended, a layout scroll does not reattach.
  await act(async () => { element.scrollTop = element.scrollHeight })
  await frame()
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(entryTop('turn/6'), -40)
  assert.notEqual(toEnd(element), 0)
})

for (const input of ['wheel down', 'touchmove down', 'Tab', 'ArrowDown', 'pointerdown']) test(`${input} without an actual reader scroll keeps following`, async () => {
  const element = await render({ rows: turns(12) })
  const row = element.querySelector('[data-embrace-entry-id]')
  await act(async () => {
    if (input === 'wheel down') element.dispatchEvent(new WheelEvent('wheel', { deltaY: 600, bubbles: true }))
    else if (input === 'touchmove down') {
      element.dispatchEvent(touch('touchstart', 200))
      element.dispatchEvent(touch('touchmove', 100))
    } else if (input === 'pointerdown') element.dispatchEvent(pointer('pointerdown', 100))
    else row.dispatchEvent(new KeyboardEvent('keydown', { key: input, bubbles: true }))
  })
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(toEnd(element), 0)
  if (input === 'pointerdown') await act(async () => document.dispatchEvent(pointer('pointerup', 100)))
})

const upward = (input, element) => {
  if (input === 'wheel up') element.dispatchEvent(new WheelEvent('wheel', { deltaY: -600, bubbles: true }))
  else if (input === 'touch up') {
    element.dispatchEvent(touch('touchstart', 100))
    element.dispatchEvent(touch('touchmove', 200))
  } else element.querySelector('[data-embrace-entry-id]').dispatchEvent(new KeyboardEvent('keydown', { key: input, bubbles: true }))
}
for (const input of ['wheel up', 'touch up', 'ArrowUp', 'PageUp', 'Home']) {
  test(`${input} with nothing above a short complete conversation keeps following`, async () => {
    const element = await render({ rows: turns(1, 120) })
    assert.equal(element.scrollTop, 0)
    await act(async () => upward(input, element))
    // A long reply streams in: the conversation now overflows the lane.
    remeasure({ 'turn/0': 1400 })
    await settle()
    assert.equal(toEnd(element), 0)
  })
  test(`${input} at the top of a pageable conversation asks for older history instead of following`, async () => {
    let requests = 0
    const element = await render({ rows: turns(1, 120), history: { _tag: 'More', loadOlder: async () => { requests++ } } })
    const before = requests
    await act(async () => upward(input, element))
    remeasure({ 'turn/0': 1400 })
    await settle()
    // Negative control for the case above: older history exists, so the gesture is reading.
    assert.ok(requests > before)
    assert.equal(element.scrollTop, 0)
    assert.notEqual(toEnd(element), 0)
  })
}

test('find-in-page without reader input keeps a following lane following', async () => {
  const element = await render({ rows: turns(12) })
  await act(async () => { element.scrollTop -= 900 })
  await frame()
  remeasure({ 'turn/11': 800 })
  await settle()
  assert.equal(toEnd(element), 0)
})

for (const detached of [false, true]) test(`programmatic jumpTo from ${detached ? 'detached reading' : 'following'} keeps its target through later growth`, async () => {
  const element = await render({ rows: turns(12) })
  if (detached) {
    await readerScroll(element, element.scrollTop - 900)
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
  await readerScroll(element, element.scrollTop - 700)
  await settle()
  const top = element.scrollTop
  remeasure({ 'turn/11': 900 })
  await settle()
  assert.equal(element.scrollTop, top)
})

// Member anchors: a leading partial turn gains its prompt or earlier work when older
// history arrives, which renames the turn, or splits it while the old id survives.
const summary = id => JSON.stringify(['work-summary', id])
const away = [{ id: 'away/0', height: 900 }]
const older = count => turns(count).map(row => ({ ...row, id: `older/${row.id}` }))
const later = count => turns(count).map(row => ({ ...row, id: `later/${row.id}` }))
const more = { _tag: 'More', loadOlder: async () => {} }
/** Leave and return: the saved state remounts the conversation; `edit` rewrites it in between. */
const switchBack = async (rows, edit = state => state, sourceIds = undefined) => {
  await render({ rows: away, key: 'away' })
  store.save('conversation', edit(store.get('conversation')))
  return render({ rows, history: { _tag: 'Complete' }, sourceIds })
}
const entryOnly = state => ({ ...state, anchor: { ...state.anchor, member: undefined } })

// The reader's line is a folded work summary; backfill adds the prompt and an earlier tool.
const renameBefore = () => [
  { id: 'turn/tool-x', height: 600, leaves: [{ id: summary('turn/tool-x'), top: 0, height: 32, members: ['tool-x'] }, { id: 'text-a', top: 40, height: 560 }] },
  ...later(8),
]
const renamed = { id: 'turn/prompt-p', height: 900, leaves: [{ id: 'prompt-p', top: 0, height: 100 }, { id: summary('turn/prompt-p'), top: 120, height: 32, members: ['earlier-tool', 'tool-x'] }, { id: 'text-a', top: 340, height: 560 }] }
const renameAfter = count => [...older(count), renamed, ...renameBefore().slice(1)]

for (const member of [true, false]) test(`switching back ${member ? 'restores a renamed turn through its summary member' : 'with an entry-only anchor falls back to the saved coordinate'}`, async () => {
  const element = await render({ rows: renameBefore(), history: more })
  await readAt(element, 'turn/tool-x', -10)
  assert.equal(leafTop(summary('turn/tool-x')), -10)
  await render({ rows: away, key: 'away' })
  assert.deepEqual(store.get('conversation').anchor.member, { id: 'tool-x', offset: -10, summary: true })
  await switchBack(renameAfter(6), member ? undefined : entryOnly)
  if (member) {
    assert.equal(leafTop(summary('turn/prompt-p')), -10)
    await render({ rows: away, key: 'away' })
    const { anchor } = store.get('conversation')
    assert.equal(anchor.entry, 'turn/prompt-p')
    assert.deepEqual(anchor.member, { id: 'tool-x', offset: -10, summary: true })
  } else assert.notEqual(leafTop(summary('turn/prompt-p')), -10)
})

for (const member of [true, false]) test(`a member owner outside the rendered window ${member ? 'restores the reader line' : 'is unknown to an entry-only anchor'}`, async () => {
  const element = await render({ rows: renameBefore(), history: more })
  await readAt(element, 'turn/tool-x', -10)
  // Sixty older turns arrive above: the saved coordinate is far from the renamed row,
  // which RAC has not mounted when the restore begins.
  await switchBack(renameAfter(60), member ? undefined : entryOnly)
  if (member) assert.equal(leafTop(summary('turn/prompt-p')), -10)
  else assert.equal(leafTop(summary('turn/prompt-p')), undefined)
})

for (const member of [true, false]) test(`a mounted reader ${member ? 'keeps its line' : 'without a member loses its line'} when backfill renames the turn under it`, async () => {
  const element = await render({ rows: renameBefore(), history: more })
  await readAt(element, 'turn/tool-x', -10)
  if (!member) {
    // Pre-member anchors: the controller's mounted anchor carries no member after a remount.
    await switchBack(renameBefore(), entryOnly)
    assert.equal(leafTop(summary('turn/tool-x')), -10)
  }
  await render({ rows: renameAfter(10), history: { _tag: 'Complete' } })
  if (member) assert.equal(leafTop(summary('turn/prompt-p')), -10)
  else assert.notEqual(leafTop(summary('turn/prompt-p')), -10)
})

// The reader's line is an expanded tool row; a prompt arrives between two tools of one turn.
const splitBefore = () => [
  ...older(4),
  { id: 'turn/tool-a', height: 400, leaves: [{ id: summary('turn/tool-a'), top: 0, height: 32, members: ['tool-a', 'tool-b'] }, { id: 'tool-a', top: 40, height: 80 }, { id: 'tool-b', top: 120, height: 80 }, { id: 'reply-b', top: 200, height: 200 }] },
  ...later(6),
]
const splitAfter = () => [
  ...older(4),
  { id: 'turn/tool-a', height: 120, leaves: [{ id: summary('turn/tool-a'), top: 0, height: 32, members: ['tool-a'] }, { id: 'tool-a', top: 40, height: 80 }] },
  { id: 'turn/prompt-p', height: 380, leaves: [{ id: 'prompt-p', top: 0, height: 60 }, { id: summary('turn/prompt-p'), top: 60, height: 32, members: ['tool-b'] }, { id: 'tool-b', top: 100, height: 80 }, { id: 'reply-b', top: 180, height: 200 }] },
  ...later(6),
]

for (const member of [true, false]) test(`a split that keeps the old id ${member ? 'restores the member in its new turn' : 'restores an entry-only anchor to the old turn'}`, async () => {
  const element = await render({ rows: splitBefore() })
  await readAt(element, 'turn/tool-a', -130)
  assert.equal(leafTop('tool-b'), -10)
  await switchBack(splitAfter(), member ? undefined : entryOnly)
  if (member) {
    assert.equal(leafTop('tool-b'), -10)
    await render({ rows: away, key: 'away' })
    assert.equal(store.get('conversation').anchor.entry, 'turn/prompt-p')
  } else assert.notEqual(leafTop('tool-b'), -10)
})

test('a mounted reader follows its member when a split keeps the old id', async () => {
  const element = await render({ rows: splitBefore() })
  await readAt(element, 'turn/tool-a', -130)
  await render({ rows: splitAfter() })
  assert.equal(leafTop('tool-b'), -10)
})

for (const loaded of [true, false]) test(`a member ${loaded ? 'loaded but awaiting adoption is not retired' : 'absent from the source falls back to the saved coordinate'}`, async () => {
  const element = await render({ rows: renameBefore(), history: more })
  await readAt(element, 'turn/tool-x', -10)
  const saved = element.scrollTop
  // The renamed turn's prompt is not adopted yet: its turn is not committed, the source has it.
  const sourceIds = loaded ? new Set(['prompt-p', 'earlier-tool', 'tool-x', 'text-a']) : new Set()
  await switchBack([...older(6), ...renameBefore().slice(1)], undefined, sourceIds)
  if (!loaded) assert.equal(lane().scrollTop, saved)
  await render({ rows: renameAfter(6), sourceIds })
  if (loaded) assert.equal(leafTop(summary('turn/prompt-p')), -10)
  else assert.notEqual(leafTop(summary('turn/prompt-p')), -10)
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
