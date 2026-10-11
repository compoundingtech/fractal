import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { JSDOM } from 'jsdom'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

// Same production component and curated data as Transcript/HarnessNotices. No kit book run.
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' })
const saved = new Map()
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'SVGElement', 'Element', 'Node', 'Document', 'DocumentFragment', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'MutationObserver', 'Event', 'KeyboardEvent', 'MouseEvent', 'getComputedStyle']) {
  saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value: name === 'getComputedStyle' ? dom.window.getComputedStyle.bind(dom.window) : dom.window[name] })
}
for (const name of ['CSS', 'ResizeObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'IS_REACT_ACT_ENVIRONMENT']) saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
globalThis.CSS = { escape: value => value }
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} }
globalThis.requestAnimationFrame = callback => setTimeout(callback, 0)
globalThis.cancelAnimationFrame = clearTimeout
globalThis.IS_REACT_ACT_ENVIRONMENT = true
const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], server: { middlewareMode: true } })
const { HarnessNotice } = await server.ssrLoadModule('/src/assistant-ui/composition/HarnessNotice.tsx')
const { harnessNoticeExamples } = await server.ssrLoadModule('/src/assistant-ui/composition/HarnessNotice.fixtures.ts')
const { baselineTheme } = await server.ssrLoadModule('/src/assistant-ui/neutral-theme.ts')
const { lightTheme } = await server.ssrLoadModule('/src/assistant-ui/composition-theme.ts')
const sx = await server.ssrLoadModule('@stylexjs/stylex')

after(async () => { await server.close(); dom.window.close(); for (const [name, descriptor] of saved) { if (descriptor === undefined) Reflect.deleteProperty(globalThis, name); else Object.defineProperty(globalThis, name, descriptor) } })
for (const scheme of ['dark', 'light']) test(`curated ${scheme} harness cards disclose real Markdown with keyboard semantics`, async () => {
  const container = document.createElement('section'); container.dataset.scheme = scheme; document.body.append(container)
  Object.assign(container, sx.props(...baselineTheme, scheme === 'light' && lightTheme))
  const root = createRoot(container)
  try {
    await act(async () => root.render(React.createElement(React.Fragment, null, harnessNoticeExamples.map(item => React.createElement(HarnessNotice, { key: item.id, item })))))
    assert.equal(container.querySelectorAll('[data-testid=harness-notice]').length, 4)
    const buttons = [...container.querySelectorAll('button[aria-expanded]')]
    assert.equal(buttons.length, 4)
    for (const button of buttons) assert.equal(button.getAttribute('aria-expanded'), 'false')
    const summaries = [...container.querySelectorAll('[data-testid=harness-excerpt]')].map(e => e.textContent).join(' ')
    assert.doesNotMatch(summaries, /<system-notice>|<irc|## Goal|"summary"/)
    const read = buttons.find(b => b.textContent === 'Read summary')
    assert.ok(read)
    await act(async () => read.click())
    assert.equal(read.getAttribute('aria-expanded'), 'true')
    const summary = container.querySelector('[data-testid=compaction-summary]')
    assert.equal(summary.querySelector('h2').textContent, 'Goal')
    assert.equal(summary.querySelectorAll('li').length, 2)
    assert.equal(summary.querySelectorAll('table').length, 1)
    assert.equal(summary.querySelectorAll('pre code').length, 1)
    for (const button of buttons.filter(b => b !== read)) await act(async () => button.click())
    assert.equal(container.querySelectorAll('[data-testid=harness-payload]').length, 3)
    // Failing controls exercise both collapsed transport and Markdown-heading assertions.
    const excerpt = container.querySelector('[data-testid=harness-excerpt]')
    const text = excerpt.textContent; excerpt.textContent = '<system-notice>control</system-notice>'
    assert.throws(() => assert.doesNotMatch(excerpt.textContent, /<system-notice>|<irc|## Goal/)); excerpt.textContent = text
    const heading = summary.querySelector('h2'); const replacement = document.createElement('span'); replacement.textContent = '## Goal'; heading.replaceWith(replacement)
    assert.throws(() => assert.equal(summary.querySelector('h2')?.textContent, 'Goal')); replacement.replaceWith(heading)
    await act(async () => read.click()); assert.equal(read.getAttribute('aria-expanded'), 'false')
  } finally { await act(async () => root.unmount()); container.remove() }
})
