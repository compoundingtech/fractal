import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'
import { test } from 'node:test'
import { collectResponse } from './response-wait.mjs'

const pane = { url: () => 'http://localhost:5173/src/web/ConversationPane.tsx' }
const other = { url: () => 'http://localhost:5173/src/web/Sidebar.tsx' }
const isPane = response => new URL(response.url()).pathname.endsWith('/ConversationPane.tsx')

test('a wait created before a setup longer than its bound neither rejects early nor misses a later response', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const page = new EventEmitter()
  const rejections = []
  const note = reason => rejections.push(reason)
  process.on('unhandledRejection', note)
  t.after(() => process.off('unhandledRejection', note))
  const wait = collectResponse(page, isPane)
  t.mock.timers.tick(30_000) // setup takes three times the bound
  await new Promise(setImmediate)
  const loaded = wait.within(10_000, 'ConversationPane.tsx')
  page.emit('response', other)
  t.mock.timers.tick(9_000)
  page.emit('response', pane)
  assert.equal(await loaded, pane)
  await new Promise(setImmediate)
  assert.deepEqual(rejections, [])
  assert.equal(page.listenerCount('response'), 0)
})

test('a response that arrives before anything waits is kept', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const page = new EventEmitter()
  const wait = collectResponse(page, isPane)
  page.emit('response', pane)
  t.mock.timers.tick(30_000)
  assert.equal(await wait.within(10_000, 'ConversationPane.tsx'), pane)
})

test('a missing response still fails at the bound, counted from the wait', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const page = new EventEmitter()
  const wait = collectResponse(page, isPane)
  t.mock.timers.tick(30_000)
  const loaded = wait.within(10_000, 'ConversationPane.tsx')
  page.emit('response', other)
  t.mock.timers.tick(10_000)
  await assert.rejects(loaded, /ConversationPane\.tsx did not arrive within 10000 ms/)
  assert.equal(page.listenerCount('response'), 0)
})
