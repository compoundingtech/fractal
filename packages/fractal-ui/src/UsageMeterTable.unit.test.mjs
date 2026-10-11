import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { JSDOM } from 'jsdom'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

// Real UsageMeterTable through the kit's StyleX transform in a DOM; callbacks only record.
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' })
const saved = new Map()
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'SVGElement', 'Element', 'Node', 'NodeFilter', 'Document', 'DocumentFragment', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'MutationObserver', 'Event', 'KeyboardEvent', 'MouseEvent', 'PointerEvent', 'FocusEvent', 'getComputedStyle']) {
  saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value: name === 'getComputedStyle' ? dom.window.getComputedStyle.bind(dom.window) : dom.window[name] })
}
for (const name of ['CSS', 'ResizeObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'IS_REACT_ACT_ENVIRONMENT']) saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
globalThis.CSS = { escape: value => value }
// Records observers so tests can drive the real container-width callback.
const observers = []
globalThis.ResizeObserver = class { constructor(callback) { this.callback = callback; this.targets = new Set(); observers.push(this) } observe(target) { this.targets.add(target) } unobserve(target) { this.targets.delete(target) } disconnect() { this.targets.clear() } }
const resize = async (target, width) => act(async () => { for (const observer of observers) if (observer.targets.has(target)) observer.callback([{ target, contentRect: { width } }], observer) })
globalThis.requestAnimationFrame = callback => setTimeout(callback, 0)
globalThis.cancelAnimationFrame = clearTimeout
globalThis.IS_REACT_ACT_ENVIRONMENT = true
const server = await createServer({ configFile: false, root: new URL('../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], server: { middlewareMode: true } })
const { UsageMeterTable } = await server.ssrLoadModule('/src/UsageMeterTable.tsx')
after(async () => { await server.close(); dom.window.close(); for (const [name, descriptor] of saved) { if (descriptor === undefined) Reflect.deleteProperty(globalThis, name); else Object.defineProperty(globalThis, name, descriptor) } })
const meterWindow = (id, label, meter, facts = []) => ({ id, label, meter, facts })
const rows = [
  { id: 'row-a', label: 'Alpha desk', identity: 'plan one', governing: { label: '5h', text: '64%', fraction: 0.64, tone: 'warning' }, comparison: { label: 'week', text: '20%', fraction: 0.2, tone: 'ok' }, reset: 'in 1h', resetCount: 0, omitted: 1, stale: false,
    detail: { facts: [{ label: 'Plan', value: 'One' }], windows: [meterWindow('a-5h', '5h window', { label: '5h', text: '64%', fraction: 0.64, tone: 'warning' }, [{ label: 'Used', value: '64 of 100' }]), meterWindow('a-week', 'Weekly window', { label: 'week', text: '20%', fraction: 0.2, tone: 'ok' }), meterWindow('a-month', 'Monthly window', { label: 'month', text: '9%', fraction: 0.09, tone: 'ok' })], resets: [{ label: 'Weekly reset', value: 'Monday 09:00' }] } },
  { id: 'row-b', label: 'Beta desk', identity: 'plan two', governing: { label: 'week', text: 'exhausted!', tone: 'exhausted' }, reset: 'in 2d', omitted: 0, stale: false,
    detail: { facts: [], windows: [meterWindow('b-week', 'Weekly window', { label: 'week', text: 'exhausted!', tone: 'exhausted' })], resets: [] } },
  { id: 'row-c', label: 'Gamma desk', identity: 'plan three', governing: { label: 'day', text: 'unknown', tone: 'unknown' }, comparison: { label: 'week', text: '30%', fraction: 0.3, tone: 'ok' }, reset: 'unreported', resetCount: 3, omitted: 0, stale: true,
    detail: { facts: [], windows: [meterWindow('c-day', 'Daily window', { label: 'day', text: 'unknown', tone: 'unknown' })], resets: [] } },
]

const mount = async (props = {}) => {
  const calls = { sort: [], cycle: 0, select: [], close: 0 }
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  const render = async (overrides = {}) => act(async () => root.render(React.createElement(UsageMeterTable, {
    rows, status: { kind: 'current', text: 'Updated now' }, sort: { key: 'governing', direction: 'descending' },
    onSort: sort => calls.sort.push(sort), onCycleSort: () => { calls.cycle++ }, onSelect: id => calls.select.push(id), onCloseDetail: () => { calls.close++ },
    ...props, ...overrides,
  })))
  await render()
  return { container, calls, render, dispose: async () => { await act(async () => root.unmount()); container.remove() } }
}
const press = async (target, key) => act(async () => { target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })) })
const rowOf = (container, id) => container.querySelector(`[data-usage-meter-row="${id}"]`)

test('headers expose every column and aria-sort follows the controlled sort prop', async () => {
  const view = await mount()
  try {
    const headers = [...view.container.querySelectorAll('[role=columnheader]')]
    assert.deepEqual(headers.map(header => header.textContent.replace(/[↑↓]/g, '')), ['Acct', 'Window gov', 'Reset', 'Comparison', 'More'])
    assert.equal(headers[1].getAttribute('aria-sort'), 'descending')
    assert.match(headers[1].textContent, /↓/)
    assert.equal(headers[2].getAttribute('aria-sort'), 'none')
    await view.render({ sort: { key: 'reset', direction: 'ascending' } })
    assert.equal(headers[1].getAttribute('aria-sort'), 'none')
    assert.equal(headers[2].getAttribute('aria-sort'), 'ascending')
    assert.match(headers[2].textContent, /↑/)
    await act(async () => headers[0].click())
    assert.deepEqual(view.calls.sort, [{ key: 'account', direction: 'ascending' }])
    assert.equal(headers[0].getAttribute('aria-sort'), 'none', 'sort stays controlled until the caller updates props')
  } finally { await view.dispose() }
})

test('meters keep unknown bar-free, exhaustion text, reported zero resets and stale dimming', async () => {
  const view = await mount()
  try {
    const alpha = rowOf(view.container, 'row-a'), beta = rowOf(view.container, 'row-b'), gamma = rowOf(view.container, 'row-c')
    assert.equal(alpha.querySelectorAll('[data-usage-meter-bar]').length, 2)
    assert.match(alpha.textContent, /↺0/)
    assert.equal(beta.querySelector('[data-usage-meter-bar]'), null, 'exhaustion without a fraction draws no invented bar')
    assert.match(beta.textContent, /exhausted!/)
    assert.doesNotMatch(beta.textContent, /↺/)
    const unknown = gamma.querySelector('[data-usage-meter-tone=unknown]')
    assert.equal(unknown.querySelector('[data-usage-meter-bar]'), null, 'unknown is never a healthy zero bar')
    assert.match(gamma.textContent, /↺3/)
    assert.equal(gamma.dataset.stale, 'true')
    assert.match(gamma.textContent, /Stale/)
    const staleBar = gamma.querySelector('[data-usage-meter-tone=ok] [data-usage-meter-bar]')
    const freshBar = alpha.querySelector('[data-usage-meter-tone=ok] [data-usage-meter-bar]')
    assert.notEqual(staleBar.className, freshBar.className, 'stale bars carry the dim style')
    const counts = [...alpha.querySelectorAll('button span')].map(span => span.textContent)
    assert.deepEqual(counts, ['+1', '+2'], 'narrow +N folds the comparison into omitted windows')
    assert.deepEqual([...beta.querySelectorAll('button span')].map(span => span.textContent), ['+0', '+0'])
  } finally { await view.dispose() }
})

test('selection uses stable ids and detail shows every supplied window and evidence with a close control', async () => {
  const view = await mount()
  try {
    await act(async () => rowOf(view.container, 'row-c').querySelector('[role=rowheader], [role=gridcell]').click())
    assert.deepEqual(view.calls.select, ['row-c'])
    assert.equal(view.container.querySelector('[data-usage-meter-detail]'), null, 'selection is controlled')
    await view.render({ selectedId: 'row-a', rows: [...rows].reverse() })
    assert.equal(rowOf(view.container, 'row-a').getAttribute('aria-selected'), 'true')
    const detail = view.container.querySelector('[data-usage-meter-detail="row-a"]')
    for (const text of ['5h window', 'Weekly window', 'Monthly window', '64 of 100', 'Plan', 'One', 'Weekly reset', 'Monday 09:00', '↺0']) assert.ok(detail.textContent.includes(text), `detail includes ${text}`)
    await act(async () => detail.querySelector('button[aria-label="Close usage details"]').click())
    assert.equal(view.calls.close, 1)
    await view.render({ selectedId: 'missing-row' })
    assert.equal(view.container.querySelector('[data-usage-meter-detail]'), null)
    assert.equal(view.container.querySelectorAll('[aria-selected=true]').length, 0)
  } finally { await view.dispose() }
})

test('s cycles sort only inside the focused monitor and never from editable targets', async () => {
  const view = await mount()
  const outside = document.createElement('input'); document.body.append(outside)
  try {
    const monitor = view.container.querySelector('[data-usage-meter-table]')
    await press(document.body, 's')
    outside.focus(); await press(outside, 's')
    assert.equal(view.calls.cycle, 0, 'no global shortcut')
    monitor.focus(); await press(monitor, 's')
    assert.equal(view.calls.cycle, 1)
    await press(monitor, 'x')
    await act(async () => { monitor.dispatchEvent(new KeyboardEvent('keydown', { key: 's', metaKey: true, bubbles: true, cancelable: true })) })
    assert.equal(view.calls.cycle, 1, 'other keys and modified s are ignored')
    const editableDiv = document.createElement('div'); editableDiv.setAttribute('contenteditable', 'true'); editableDiv.tabIndex = 0
    for (const editable of [document.createElement('input'), document.createElement('textarea'), editableDiv]) {
      monitor.append(editable); editable.focus()
      assert.equal(document.activeElement, editable)
      await press(editable, 's'); editable.remove()
    }
    assert.equal(view.calls.cycle, 1, 'editable targets keep typing')
    const sortButton = view.container.querySelector('button[aria-label="Cycle usage sort"]')
    sortButton.focus(); await press(sortButton, 's')
    assert.equal(view.calls.cycle, 2, 'focused descendants of the monitor still cycle')
  } finally { outside.remove(); await view.dispose() }
})

test('comparison visibility reports the 36rem container threshold once per change', async () => {
  const visibility = []
  const onComparisonVisibilityChange = visible => visibility.push(visible)
  const view = await mount({ onComparisonVisibilityChange })
  try {
    const monitor = view.container.querySelector('[data-usage-meter-table]')
    await resize(monitor, 575)
    await resize(monitor, 320)
    await resize(monitor, 576)
    await resize(monitor, 900)
    assert.deepEqual(visibility, [false, true])
    await view.render({ onComparisonVisibilityChange, sort: { key: 'more', direction: 'ascending' } })
    await resize(monitor, 900)
    assert.deepEqual(visibility, [false, true], 'stable callbacks keep one observer across renders')
  } finally { await view.dispose() }
})

test('detail history draws supplied periods: unmeasured gaps, measured zeroes, open periods and explicit failures', async () => {
  const points = [
    { id: 'p1', label: 'P1', value: 4, text: '4 units', partial: false },
    { id: 'p2', label: 'P2', value: 0, text: '0 units', partial: false },
    { id: 'p3', label: 'P3', text: 'unreported', partial: false },
    { id: 'p4', label: 'P4', value: 2, text: '2 units', partial: true },
  ]
  const view = await mount({ selectedId: 'row-a', history: { kind: 'ready', label: 'Period history', text: '4 periods · max 4', points, facts: [{ label: 'Older periods omitted', value: '7' }] } })
  try {
    const chart = view.container.querySelector('[data-usage-meter-detail="row-a"] [data-usage-meter-history="ready"]')
    assert.ok(chart, 'history renders inside the row detail')
    assert.match(chart.textContent, /4 periods · max 4/)
    assert.match(chart.textContent, /Older periods omitted7/)
    const slots = [...chart.querySelectorAll('[data-usage-meter-history-point]')]
    assert.deepEqual(slots.map(slot => slot.getAttribute('aria-label')), ['P1: 4 units', 'P2: 0 units', 'P3: unreported', 'P4: 2 units, period still open'])
    assert.equal(slots[2].dataset.unmeasured, 'true', 'unmeasured is a gap marker')
    assert.equal(slots[1].dataset.unmeasured, undefined, 'a measured zero is not a gap')
    assert.equal(slots[3].dataset.partial, 'true')
    assert.notEqual(slots[3].firstElementChild.className, slots[0].firstElementChild.className, 'open periods carry a distinct style')
    assert.notEqual(slots[2].firstElementChild.className, slots[1].firstElementChild.className)
    await view.render({ selectedId: 'row-a', history: { kind: 'unavailable', label: 'Period history', text: 'History unavailable: HTTP 502' } })
    const failed = view.container.querySelector('[data-usage-meter-history="unavailable"]')
    assert.equal(failed.querySelector('[role=alert]').textContent, 'History unavailable: HTTP 502')
    assert.equal(failed.querySelectorAll('[data-usage-meter-history-point]').length, 0, 'a failure never draws zero bars')
    await view.render({ selectedId: undefined })
    assert.equal(view.container.querySelector('[data-usage-meter-history]'), null, 'history belongs to the open detail only')
  } finally { await view.dispose() }
})
