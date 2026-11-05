import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { useAuiState } from '@assistant-ui/react'
import { JSDOM } from 'jsdom'
import { createServer } from 'vite'

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' })
const saved = new Map()
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'IS_REACT_ACT_ENVIRONMENT']) {
  saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value: dom.window[name] })
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true
let nextFrame = 0
const frames = new Map()
globalThis.requestAnimationFrame = callback => { frames.set(++nextFrame, callback); return nextFrame }
globalThis.cancelAnimationFrame = id => frames.delete(id)
const server = await createServer({ configFile: false, root: new URL('../../', import.meta.url).pathname, server: { middlewareMode: true } })
const { EmbraceRuntimeProvider, RuntimeAdoptedIds } = await server.ssrLoadModule('/src/assistant-ui/EmbraceRuntime.tsx')
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
