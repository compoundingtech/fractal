import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'

const server = await createServer({ configFile: false, root: new URL('../../', import.meta.url).pathname, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } })
after(() => server.close())
const { bindComposerDraft } = await server.ssrLoadModule('/src/assistant-ui/ComposerDraft.ts')

test('hydrates before subscribing, saves native changes once and flushes on page hide and unmount', () => {
  const originalWindow = globalThis.window
  const events = new EventTarget()
  globalThis.window = events
  try {
    let text = '', saved = 'Saved draft', writes = 0, flushes = 0
    const listeners = new Set()
    const composer = { getText: () => text, setText: next => { assert.equal(listeners.size, 0); text = next }, subscribe: notify => { listeners.add(notify); return () => listeners.delete(notify) } }
    const storage = { read: () => saved, write: next => { writes++; saved = next }, flush: () => { flushes++ } }
    const edit = next => { text = next; for (const listener of listeners) listener() }
    const close = bindComposerDraft({ composer, storage })
    assert.equal(text, 'Saved draft')
    edit('Saved draft')
    assert.equal(writes, 0)
    edit('Recalled message')
    assert.equal(saved, 'Recalled message')
    assert.equal(writes, 1)
    events.dispatchEvent(new Event('pagehide'))
    assert.equal(flushes, 1)
    edit('')
    assert.equal(saved, '')
    assert.equal(writes, 2)
    text = 'Final unsent draft'
    close()
    assert.equal(saved, 'Final unsent draft')
    assert.equal(flushes, 2)
    assert.equal(listeners.size, 0)
    events.dispatchEvent(new Event('pagehide'))
    assert.equal(flushes, 2)
    const other = { ...storage, read: () => 'Other conversation' }
    const closeOther = bindComposerDraft({ composer, storage: other })
    assert.equal(text, 'Other conversation')
    closeOther()
  } finally {
    if (originalWindow === undefined) delete globalThis.window
    else globalThis.window = originalWindow
  }
})
