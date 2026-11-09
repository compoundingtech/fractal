import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { extname, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { strictStoryGuard } from './details-strict-story-guard.mjs'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const build = resolve(fileURLToPath(new URL('../storybook-static/', import.meta.url)))
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }

const serve = async (t, handler) => {
  const server = createServer(handler)
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  t.after(() => new Promise(resolve => server.close(resolve)))
  return `http://127.0.0.1:${server.address().port}`
}
const openBrowser = async t => {
  const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
  t.after(() => browser.close())
  return browser
}

test('strict readiness observes completion in the task that publishes the channel, without a timer turn', async t => {
  const base = await serve(t, (_request, response) => response.setHeader('Content-Type', 'text/html').end(`<!doctype html><script>
    const listeners = new Map()
    window.__STORYBOOK_ADDONS_CHANNEL__ = {
      on(name, listener) {
        if (!listeners.has(name)) listeners.set(name, [])
        listeners.get(name).push(listener)
      }
    }
    for (const listener of listeners.get('storyFinished') ?? []) listener({ storyId: 'fixture--same-task', status: 'success', reporters: [] })
  </script>`))
  const browser = await openBrowser(t), page = await browser.newPage()
  // A missed event is permanent, not slow. Keep the negative control cheap; production's guard is unchanged.
  const wait = page.waitForFunction.bind(page)
  page.waitForFunction = (predicate, argument, options) => wait(predicate, argument, { ...options, timeout: 1000 })
  const guard = await strictStoryGuard(page, base)
  await page.goto(`${base}/iframe.html?id=fixture--same-task`, { waitUntil: 'domcontentloaded' })
  await guard.check()
  assert.equal(guard.records[0].finished.length, 1)
  assert.equal(guard.records[0].finished[0].payload.status, 'success')
})

test('AllStates closes and returns focus while the closing animation frame is withheld', async t => {
  const base = await serve(t, async (request, response) => {
    const file = resolve(build, `.${new URL(request.url, 'http://localhost').pathname}`)
    if (!file.startsWith(build + sep)) { response.writeHead(403).end(); return }
    try { response.writeHead(200, { 'Content-Type': mime[extname(file)] ?? 'application/octet-stream' }).end(await readFile(file)) }
    catch { response.writeHead(404).end() }
  })
  const browser = await openBrowser(t), page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  await page.addInitScript(() => {
    const frame = window.requestAnimationFrame.bind(window)
    let closing = false
    document.addEventListener('keydown', event => {
      if (event.key === 'Escape' && event.target.closest('[data-details-all-state="partial"]')) closing = true
    }, { capture: true })
    window.requestAnimationFrame = callback => closing
      ? frame(time => { window.__detailsHeldCloseFrame = true; setTimeout(() => callback(time), 1500) })
      : frame(callback)
  })
  const guard = await strictStoryGuard(page, base)
  await page.goto(`${base}/iframe.html?id=fractal-ui-details-presentation--all-states&viewMode=story`, { waitUntil: 'domcontentloaded' })
  await guard.check()
  assert.equal(await page.getByRole('button', { name: 'Restore all states' }).evaluate(node => node === node.ownerDocument.activeElement), true)
})
