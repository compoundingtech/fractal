import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { JSDOM } from 'jsdom'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' })
const saved = new Map()
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'SVGElement', 'Element', 'Node', 'NodeFilter', 'Document', 'DocumentFragment', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'HTMLSelectElement', 'MutationObserver', 'Event', 'CustomEvent', 'KeyboardEvent', 'MouseEvent', 'getComputedStyle']) {
  saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value: name === 'getComputedStyle' ? dom.window.getComputedStyle.bind(dom.window) : dom.window[name] })
}
for (const name of ['CSS', 'ResizeObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'IS_REACT_ACT_ENVIRONMENT']) saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
globalThis.CSS = { escape: value => value }
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} }
globalThis.requestAnimationFrame = callback => setTimeout(callback, 0)
globalThis.cancelAnimationFrame = clearTimeout
globalThis.IS_REACT_ACT_ENVIRONMENT = true
// React Aria's test mode writes scroll offsets directly; jsdom does not implement Element.scrollTo.
const savedNodeEnv = process.env.NODE_ENV
process.env.NODE_ENV = 'test'
const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], server: { middlewareMode: true } })
const { SidebarTabs } = await server.ssrLoadModule('/src/assistant-ui/sidebar/SidebarTabs.tsx')
const { GridList, GridListItem } = await import('react-aria-components')
after(async () => { await server.close(); dom.window.close(); if (savedNodeEnv === undefined) delete process.env.NODE_ENV; else process.env.NODE_ENV = savedNodeEnv; for (const [name, descriptor] of saved) { if (descriptor === undefined) Reflect.deleteProperty(globalThis, name); else Object.defineProperty(globalThis, name, descriptor) } })
const key = async (target, value) => act(async () => target.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true })))
const tabs = [{ id: 'agents', label: 'Agents', questionCount: 2 }, { id: 'alerts', label: 'Alerts' }]

for (const empty of [false, true]) test(`sidebar panel preserves ARIA and tab order with ${empty ? 'empty' : 'populated'} roster`, async () => {
  const container = document.createElement('section'); document.body.append(container)
  const root = createRoot(container)
  function Host() {
    const [selectedId, onSelectionChange] = React.useState('agents')
    return React.createElement(SidebarTabs, { tabs, selectedId, onSelectionChange, panelHasTabbableContent: selectedId === 'agents' },
      selectedId === 'agents'
        ? React.createElement(GridList, { 'aria-label': 'Roster', selectionMode: 'none' }, empty ? null : React.createElement(GridListItem, { id: 'agent', textValue: 'Agent' }, 'Agent'))
        : 'No alerts')
  }
  try {
    await act(async () => root.render(React.createElement(Host)))
    const [agents, alerts] = container.querySelectorAll('[role=tab]')
    const panel = () => container.querySelector('[role=tabpanel]:not([hidden])')
    const assertWiring = selected => {
      for (const tab of [agents, alerts]) {
        const target = document.getElementById(tab.getAttribute('aria-controls'))
        assert.ok(target, 'every tab controls a mounted panel')
        assert.equal(target.getAttribute('aria-labelledby'), tab.id)
        assert.equal(target.hidden, tab !== selected)
        assert.equal(tab.getAttribute('aria-selected'), String(tab === selected))
        assert.equal(tab.tabIndex, tab === selected ? 0 : -1)
      }
      assert.equal(panel().getAttribute('aria-labelledby'), selected.id)
    }
    assertWiring(agents)
    assert.equal(agents.getAttribute('aria-label'), 'Agents, 2 open questions')
    assert.equal(container.querySelector('[role=tablist]').getAttribute('aria-label'), 'Sidebar')
    assert.equal(panel().tabIndex, -1, 'the panel must not precede the roster in the tab order')
    const grid = panel().querySelector('[role=grid]')
    assert.equal(grid.tabIndex, 0, 'React Aria keeps even an empty collection tabbable')
    await act(async () => agents.focus())
    await key(agents, 'ArrowRight')
    assertWiring(alerts)
    assert.equal(document.activeElement, alerts)
    assert.equal(panel().tabIndex, 0, 'an empty panel without controls remains keyboard reachable')
    await key(alerts, 'Home')
    assertWiring(agents)
    await key(agents, 'End')
    assertWiring(alerts)
    await key(alerts, 'ArrowLeft')
    assertWiring(agents)
  } finally { await act(async () => root.unmount()); container.remove() }
})
