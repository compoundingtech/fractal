import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'
import { mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises'
import { test } from 'node:test'
import { waitForDevtoolsEndpoint } from './devtools-endpoint.mjs'

const fixture = async t => {
  const root = new URL('../../../tmp/devtools-endpoint-tests/', import.meta.url)
  await mkdir(root, { recursive: true })
  const profile = await mkdtemp(new URL('profile-', root))
  t.after(() => rm(profile, { recursive: true, force: true }))
  // A lifecycle source only: filesystem publication is real, no browser launch is mocked.
  const browser = new EventEmitter()
  browser.isConnected = () => true
  return { profile, browser, path: `${profile}/DevToolsActivePort` }
}
const contents = '45678\n/devtools/browser/12345678-abcd-4123-8123-123456789abc'
const endpoint = 'ws://127.0.0.1:45678/devtools/browser/12345678-abcd-4123-8123-123456789abc'

test('waits for delayed atomic publication where the old immediate read fails', async t => {
  const { profile, browser, path } = await fixture(t)
  await assert.rejects(readFile(path, 'utf8'), { code: 'ENOENT' })
  const ready = waitForDevtoolsEndpoint(profile, browser)
  await writeFile(`${path}.tmp`, contents)
  await rename(`${path}.tmp`, path)
  assert.equal(await ready, endpoint)
  assert.equal(browser.listenerCount('disconnected'), 0)
})

test('accepts an endpoint already published before observation starts', async t => {
  const { profile, browser, path } = await fixture(t)
  await writeFile(path, contents)
  assert.equal(await waitForDevtoolsEndpoint(profile, browser), endpoint)
  assert.equal(browser.listenerCount('disconnected'), 0)
})

test('partial publication is not readiness; subsequent complete content is observed', async t => {
  const { profile, browser, path } = await fixture(t)
  await writeFile(path, '45678\n/devtools/browser/12345678-abcd')
  let settled = false
  const ready = waitForDevtoolsEndpoint(profile, browser).then(value => { settled = true; return value })
  await new Promise(setImmediate)
  assert.equal(settled, false)
  await writeFile(path, contents)
  assert.equal(await ready, endpoint)
})

test('a disconnect fails immediately and removes its lifecycle listener', async t => {
  const { profile, browser } = await fixture(t)
  const ready = waitForDevtoolsEndpoint(profile, browser)
  const failed = assert.rejects(ready, /disconnected before publishing/)
  browser.emit('disconnected')
  await failed
  assert.equal(browser.listenerCount('disconnected'), 0)
})

test('already disconnected browser cannot publish readiness from a stale file', async t => {
  const { profile, browser, path } = await fixture(t)
  await writeFile(path, contents)
  browser.isConnected = () => false
  await assert.rejects(waitForDevtoolsEndpoint(profile, browser), /disconnected before publishing/)
  assert.equal(browser.listenerCount('disconnected'), 0)
})

test('malformed publication fails at the startup bound', async t => {
  const { profile, browser, path } = await fixture(t)
  await writeFile(path, '70000\n/devtools/browser/invalid-port')
  await assert.rejects(waitForDevtoolsEndpoint(profile, browser, 25), /complete DevTools endpoint within 25 ms/)
  assert.equal(browser.listenerCount('disconnected'), 0)
})

test('permanent read errors are surfaced rather than treated as startup progress', async t => {
  const { profile, browser, path } = await fixture(t)
  await mkdir(path)
  await assert.rejects(waitForDevtoolsEndpoint(profile, browser), { code: 'EISDIR' })
  assert.equal(browser.listenerCount('disconnected'), 0)
})

test('missing publication fails at the startup bound', async t => {
  const { profile, browser } = await fixture(t)
  await assert.rejects(waitForDevtoolsEndpoint(profile, browser, 25), /complete DevTools endpoint within 25 ms/)
  assert.equal(browser.listenerCount('disconnected'), 0)
})
