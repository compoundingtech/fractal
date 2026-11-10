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
  t.after(() => { server.closeAllConnections(); return new Promise(resolve => server.close(resolve)) })
  return `http://127.0.0.1:${server.address().port}`
}
const openBrowser = async t => {
  const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
  t.after(() => browser.close())
  return browser
}

test('strict readiness observes completion in the task that publishes the channel, without a timer turn', { timeout: 30000 }, async t => {
  const base = await serve(t, (_request, response) => response.setHeader('Content-Type', 'text/html').end(`<!doctype html><script>
    const listeners = new Map()
    window.__STORYBOOK_ADDONS_CHANNEL__ = {
      on(name, listener) {
        if (!listeners.has(name)) listeners.set(name, [])
        listeners.get(name).push(listener)
      }
    }
    for (const listener of listeners.get('storyFinished') ?? []) listener({ storyId: 'fixture--same-task', status: 'success', reporters: [] })
    window.__fixtureEmitted = true
  </script>`))
  const browser = await openBrowser(t), page = await browser.newPage()
  const navigate = page.goto.bind(page)
  await strictStoryGuard(page, base)
  // Inspect the capture immediately after the fixture has emitted, without entering the guard's readiness wait.
  await navigate(`${base}/iframe.html?id=fixture--same-task`, { waitUntil: 'domcontentloaded' })
  const captured = await page.evaluate(() => ({ emitted: window.__fixtureEmitted, events: window.__dt10StrictEvents }))
  assert.equal(captured.emitted, true)
  assert.equal(captured.events.length, 1, 'Storybook emitted storyFinished but the guard missed it')
  assert.equal(captured.events[0].payload.status, 'success')
})

test('AllStates returns focus in the close event without waiting for an animation frame', { timeout: 30000 }, async t => {
  const base = await serve(t, async (request, response) => {
    const file = resolve(build, `.${new URL(request.url, 'http://localhost').pathname}`)
    if (!file.startsWith(build + sep)) { response.writeHead(403).end(); return }
    try { const bytes = await readFile(file); response.writeHead(200, { 'Content-Type': mime[extname(file)] ?? 'application/octet-stream' }).end(bytes) }
    catch { response.writeHead(404).end() }
  })
  const browser = await openBrowser(t), page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  let reportClose
  const closed = new Promise(resolve => { reportClose = resolve })
  await page.exposeFunction('__detailsCloseObserved', reportClose)
  await page.addInitScript(() => {
    const frame = window.requestAnimationFrame.bind(window)
    let closing = false
    document.addEventListener('keydown', event => {
      if (event.key !== 'Escape' || !event.target.closest('[data-details-all-state="partial"]')) return
      closing = true
      // Observe the completed event, not the later frame or the story's timeout/error serialization.
      queueMicrotask(() => {
        const target = [...document.querySelectorAll('button')].find(button => button.textContent === 'Restore all states')
        void window.__detailsCloseObserved({ returned: document.activeElement === target, active: document.activeElement?.textContent })
      })
    }, { capture: true })
    window.requestAnimationFrame = callback => closing ? 0 : frame(callback)
  })
  await page.goto(`${base}/iframe.html?id=fractal-ui-details-presentation--all-states&viewMode=story`, { waitUntil: 'domcontentloaded' })
  const focus = await closed
  assert.equal(focus.returned, true, `Close left focus on ${focus.active}`)
})
