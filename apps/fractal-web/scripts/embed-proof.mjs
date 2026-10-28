#!/usr/bin/env node
/** Focused proof against the built iframe entry, including opposite-theme sibling frames. */
import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { preview } from 'vite'

const app = fileURLToPath(new URL('..', import.meta.url))
const server = await preview({ root: app, configFile: fileURLToPath(new URL('../vite.embed.config.ts', import.meta.url)), configLoader: 'runner', preview: { host: '127.0.0.1', port: 0, strictPort: true } })
let browser
try {
  const address = server.httpServer.address()
  assert(address && typeof address !== 'string')
  const origin = `http://127.0.0.1:${address.port}`
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
  browser = await chromium.launch({ headless: true, ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}) })
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } })
  await context.addInitScript(() => {
    window.__embedNetworkCalls = []
    const forbidden = (name) => { window.__embedNetworkCalls.push(name); throw new Error(`Embed attempted ${name}`) }
    window.fetch = () => forbidden('fetch')
    for (const name of ['WebSocket', 'XMLHttpRequest', 'EventSource']) window[name] = class { constructor() { forbidden(name) } }
    navigator.sendBeacon = () => forbidden('sendBeacon')
  })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto(`${origin}/embed.html?theme=light#codex`)
  await page.getByRole('status').waitFor()
  const sentinel = 'real-app-selection-must-not-change'
  await page.evaluate(sentinel => {
    localStorage.setItem('wf.ui@1', sentinel)
    document.body.innerHTML = '<iframe id="light" style="width:1100px;height:700px" src="embed.html?theme=light#web"></iframe><iframe id="dark" style="width:1100px;height:700px" src="embed.html?theme=dark#web-narrow"></iframe>'
  }, sentinel)
  const light = page.frameLocator('#light')
  const dark = page.frameLocator('#dark')
  await light.locator('[data-testid="live-agent-workspace"][data-scheme="light"]').waitFor()
  await dark.locator('[data-testid="live-agent-workspace"][data-scheme="dark"]').waitFor()
  await dark.getByRole('button', { name: 'Expand sidebar', exact: true }).waitFor()
  assert.equal(await dark.getByRole('button', { name: 'Collapse agents', exact: true }).count(), 0)
  const darkFrame = await (await page.$('#dark')).contentFrame()
  assert(await darkFrame.evaluate(() => window.innerWidth) >= 1000, 'narrow branch must work inside a wide iframe')
  await page.waitForTimeout(350)
  assert.equal(await page.evaluate(() => localStorage.getItem('wf.ui@1')), sentinel)
  await light.getByRole('button', { name: 'Toggle right panel', exact: true }).click()
  await light.getByRole('separator', { name: 'Resize Details', exact: true }).press('ArrowLeft')
  assert.deepEqual(await page.evaluate(() => Object.keys(localStorage)), ['wf.ui@1'])
  await light.getByRole('button', { name: 'Toggle right panel', exact: true }).click()
  assert.equal(await darkFrame.locator('.embed-narrow').evaluate(node => Math.round(node.getBoundingClientRect().width)), 440)
  const before = { history: await page.evaluate(() => history.length), url: await light.locator('html').evaluate(() => location.href) }
  await light.locator('[data-wf-agent-ref="agent/build-host-a/palette-review"] [data-row-column="title"]').click()
  await light.getByTestId('thread-header').getByText('PaletteReview', { exact: true }).waitFor()
  assert.equal(await light.locator('html').evaluate(() => location.href), before.url)
  assert.equal(await page.evaluate(() => history.length), before.history)
  await page.evaluate(() => {
    localStorage.setItem('wf.ui@1', JSON.stringify({ version: 1, entries: { 'round2.scheme': { version: 1, value: 'dark', clock: Number.MAX_SAFE_INTEGER, writer: 'real-app' } } }))
  })
  await light.locator('[data-testid="live-agent-workspace"][data-scheme="light"]').waitFor()
  // Allow debounce and storage-event delivery; neither frame may write the real app document.
  await page.waitForTimeout(350)
  const storage = await page.evaluate(() => localStorage.getItem('wf.ui@1'))
  assert.equal(JSON.parse(storage).entries['round2.scheme'].writer, 'real-app')
  assert.equal(await light.locator('[data-testid="live-agent-workspace"]').getAttribute('data-scheme'), 'light')
  assert.equal(await dark.locator('[data-testid="live-agent-workspace"]').getAttribute('data-scheme'), 'dark')
  const lightFrame = await (await page.$('#light')).contentFrame()
  // goto on the unchanged hash URL may be same-document; exercise an actual iframe reload.
  await Promise.all([
    page.waitForEvent('framenavigated', { predicate: frame => frame === lightFrame }),
    lightFrame.evaluate(() => location.reload()),
  ])
  await light.getByTestId('thread-header').getByText('WorkbenchShell', { exact: true }).waitFor()
  assert.equal(await page.evaluate(() => localStorage.getItem('wf.ui@1')), storage)
  for (const frame of page.frames()) assert.deepEqual(await frame.evaluate(() => window.__embedNetworkCalls), [])
  console.log('PASS: memory-only storage, opposite-theme iframe isolation, container-driven narrow layout, private navigation and reload reset')
  await page.close()
  if (process.env.EMBED_SCREENSHOT_DIR) await mkdir(process.env.EMBED_SCREENSHOT_DIR, { recursive: true })
  for (const fragment of ['web', 'web-narrow', 'claude-code', 'codex']) for (const theme of ['light', 'dark']) {
    const snapshot = await context.newPage()
    snapshot.on('pageerror', error => errors.push(error.message))
    try {
      await snapshot.setViewportSize({ width: 1200, height: 800 })
      await snapshot.goto(`${origin}/embed.html?theme=${theme}#${fragment}`)
      await snapshot.locator(`html[data-scheme="${theme}"]`).waitFor()
      // Match ConversationPane's commitment observer rather than accepting an empty loading lane.
      if (fragment.startsWith('web')) await snapshot.locator('[data-testid="transcript-scroll"]:visible:not(:has([data-testid="transcript-placeholder"])) [data-testid="transcript-turn"]').first().waitFor()
      else {
        assert.equal(await snapshot.getByRole('main').count(), 0)
        await snapshot.getByRole('region', { name: fragment === 'codex' ? 'Codex' : 'Claude Code', exact: true }).getByRole('status').waitFor()
      }
      assert.deepEqual(await snapshot.evaluate(() => window.__embedNetworkCalls), [])
      if (process.env.EMBED_SCREENSHOT_DIR) await snapshot.screenshot({ path: `${process.env.EMBED_SCREENSHOT_DIR}/${fragment}-${theme}.png` })
    } finally { await snapshot.close() }
  }
  assert.deepEqual(errors, [])
  console.log('PASS: eight fragment/theme entries, host-owned main landmarks, and zero fetch/WebSocket/XMLHttpRequest/EventSource/sendBeacon calls')
} finally {
  await browser?.close()
  await new Promise(resolve => server.httpServer.close(resolve))
}
