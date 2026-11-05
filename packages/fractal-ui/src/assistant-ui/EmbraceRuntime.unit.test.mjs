import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { useAuiState } from '@assistant-ui/react'
import { JSDOM } from 'jsdom'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' })
const saved = new Map()
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'SVGElement', 'Element', 'Node', 'Document', 'DocumentFragment', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'MutationObserver', 'Event', 'CustomEvent', 'KeyboardEvent', 'MouseEvent', 'getComputedStyle', 'CSS', 'ResizeObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'IS_REACT_ACT_ENVIRONMENT']) {
  saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value: name === 'getComputedStyle' ? dom.window.getComputedStyle.bind(dom.window) : dom.window[name] })
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true
globalThis.CSS = { escape: value => value }
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} }
let nextFrame = 0
const frames = new Map()
globalThis.requestAnimationFrame = callback => { frames.set(++nextFrame, callback); return nextFrame }
globalThis.cancelAnimationFrame = id => frames.delete(id)
const server = await createServer({ configFile: false, root: new URL('../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], server: { middlewareMode: true } })
const { EmbraceRuntimeProvider, RuntimeAdoptedIds } = await server.ssrLoadModule('/src/assistant-ui/EmbraceRuntime.tsx')
const { Transcript } = await server.ssrLoadModule('/src/assistant-ui/composition/Transcript.tsx')
const { prepareTranscriptTurns } = await server.ssrLoadModule(new URL('../../../../apps/fractal-web/src/web/conversationTranscript.ts', import.meta.url).pathname)
after(async () => { await server.close(); dom.window.close(); for (const [name, descriptor] of saved) { if (descriptor === undefined) Reflect.deleteProperty(globalThis, name); else Object.defineProperty(globalThis, name, descriptor) } })
const message = id => ({ _tag: 'Text', id, role: 'user', text: id, attachments: [], streaming: false, at: '2026-01-15T12:30:00Z' })
const tail = [message('tail')]
const older = Array.from({ length: 12 }, (_, index) => message(`older-${index}`))
const onNew = async () => {}

for (const change of ['append', 'update', 'replace', 'second prepend']) test(`history adoption preserves ${change} during an unfinished prefix`, async () => {
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  let messages, adoption
  function Probe() { messages = useAuiState(state => state.thread.messages); adoption = React.useContext(RuntimeAdoptedIds); return null }
  const publish = async messages => act(async () => {
    root.render(React.createElement(EmbraceRuntimeProvider, { options: { messages, isRunning: false, onNew } }, React.createElement(Probe)))
  })
  const ids = () => messages.map(message => message.id)
  const paint = async () => act(async () => { const callbacks = [...frames.values()]; frames.clear(); for (const callback of callbacks) callback(0) })
  try {
    await publish(tail)
    await publish([...older, ...tail])
    assert.deepEqual(ids(), [...older.slice(-8), ...tail].map(message => message.id))
    assert.equal(adoption.isSnapshotAdopted(), false)
    const obsolete = [...frames.values()]
    const updated = { ...tail[0], text: 'updated while paging' }
    const expected = change === 'append' ? [...older, ...tail, message('live')]
      : change === 'update' ? [...older, updated]
      : change === 'replace' ? [message('replacement')]
      : [message('earliest'), ...older, ...tail]
    await publish(expected)
    await act(async () => { for (const callback of obsolete) callback(0) })
    for (let frame = 0; frame < 6; frame++) await paint()
    assert.deepEqual(ids(), expected.map(message => message.id))
    assert.equal(new Set(ids()).size, ids().length)
    assert.equal(adoption.isSnapshotAdopted(), true)
    assert.equal(frames.size, 0, 'obsolete batches are cancelled and adoption completes')
    if (change === 'update') assert.equal(messages.at(-1).content[0].text, updated.text)
  } finally { await act(async () => root.unmount()); container.remove(); frames.clear() }
})

for (const withPrompt of [true, false]) test(`a published answer remains mounted through every split-turn batch (${withPrompt ? 'pending prompt' : 'pending item prefix'})`, async () => {
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  const answer = { ...message('existing-answer'), role: 'assistant', text: 'Already visible answer' }
  const prefix = Array.from({ length: withPrompt ? 59 : 60 }, (_, index) => ({ ...message(`prefix-${index}`), role: 'assistant' }))
  const full = [...(withPrompt ? [message('earlier-prompt')] : []), ...prefix, answer]
  let adopted = false, publishedIds = []
  function Probe() { const adoption = React.useContext(RuntimeAdoptedIds); const messages = useAuiState(state => state.thread.messages); publishedIds = messages.map(message => message.id); adopted = adoption.isSnapshotAdopted() && messages.length === full.length; return null }
  const publish = async messages => act(async () => root.render(React.createElement(EmbraceRuntimeProvider, {
    options: { messages, isRunning: false, onNew },
  }, React.createElement(React.Fragment, null, React.createElement(Probe), React.createElement(Transcript, {
    turns: prepareTranscriptTurns(messages, { firstTurnComplete: false }),
    title: 'Split turn', sync: { _tag: 'Live', since: 0 }, now: Date.parse(answer.at),
  })))))
  const visible = () => {
    assert.ok([...container.querySelectorAll('[data-item-id="existing-answer"]')].some(node => node.textContent.includes(answer.text)), 'the previously visible answer disappeared')
    assert.deepEqual(publishedIds, full.slice(-publishedIds.length).map(message => message.id), 'published rows must be an ordered, contiguous source suffix')
    // The viewport may additionally pin the prompt; its message rows must still have no holes.
    const mounted = [...container.querySelectorAll('[data-testid="transcript-turn"]')].flatMap(node => node.dataset.sourceIds.split(' ')).filter(id => id !== 'earlier-prompt')
    assert.deepEqual(mounted, [...prefix, answer].slice(-mounted.length).map(message => message.id), 'mounted message rows must retain source order without holes')
  }
  try {
    await publish([answer])
    visible()
    await publish(full)
    visible()
    assert.equal(adopted, false)
    for (let batch = 0; batch < 7; batch++) {
      await act(async () => { const callbacks = [...frames.values()]; frames.clear(); for (const callback of callbacks) callback(0) })
      visible()
    }
    assert.equal(adopted, true)
  } finally { await act(async () => root.unmount()); container.remove(); frames.clear() }
})

test('unmount during a prefix batch cancels its pending animation frame', async () => {
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  const publish = async messages => act(async () => root.render(React.createElement(EmbraceRuntimeProvider, { options: { messages, isRunning: false, onNew } })))
  try {
    await publish(tail)
    await publish([...older, ...tail])
    assert.ok(frames.size > 0)
    await act(async () => root.unmount())
    assert.equal(frames.size, 0)
  } finally { container.remove(); frames.clear() }
})
