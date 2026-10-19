#!/usr/bin/env node
// Real app regression: lane-responsive header, not viewport-responsive chrome.
// Run through gate-slot with CI=1. Managed environments may set PLAYWRIGHT_MODULE and
// CHROMIUM_PATH. SPLIT_PROOF_DIR optionally records screenshots and the JSON receipt.
import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createServer, loadConfigFromFile } from 'vite'
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const app = fileURLToPath(new URL('..', import.meta.url))
const out = process.env.SPLIT_PROOF_DIR
if (out) await mkdir(out, { recursive: true })
const loaded = await loadConfigFromFile({ command: 'serve', mode: 'development' }, `${app}/vite.config.ts`, app)
const server = await createServer({ ...loaded.config, configFile: false, root: `${app}/src/web`,
  optimizeDeps: { ...loaded.config.optimizeDeps, entries: ['split-header-geometry.browser.html'], include: ['react', 'react-dom/client', '@assistant-ui/react'] },
  plugins: loaded.config.plugins.flat().filter(plugin => plugin?.name !== 'wf:shared-client-gateway'),
  server: { ...loaded.config.server, host: '127.0.0.1', port: 0, strictPort: false, hmr: false },
})
let browser
const rows = []
try {
  await server.listen()
  const port = server.httpServer.address().port
  browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
  for (const scheme of ['dark', 'light']) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme })
    await context.addInitScript(scheme => localStorage.setItem('wf.ui@1', JSON.stringify({ version: 1, entries: { 'round2.scheme': { version: 1, value: scheme, clock: 1, writer: 'split-header-proof' } } })), scheme)
    const page = await context.newPage(), errors = []
    page.on('pageerror', error => errors.push(String(error)))
    await page.goto(`http://127.0.0.1:${port}/split-header-geometry.browser.html`)
    await page.getByRole('button', { name: 'Toggle right panel', exact: true }).click()
    await page.getByTestId('diff-code').waitFor()
    const sidebar = page.getByRole('separator', { name: 'Agent sidebar width' })
    const dock = page.getByRole('separator', { name: 'Resize changes' })
    const inspect = async (width, size) => {
      await page.setViewportSize({ width, height: 900 })
      await sidebar.focus(); await page.keyboard.press('Home')
      if (width === 1440) for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowRight')
      await dock.focus(); await page.keyboard.press('End')
      if (size === 440) { await page.keyboard.press('Home'); for (let i = 0; i < 3; i++) await page.keyboard.press('Shift+ArrowLeft'); for (let i = 0; i < 3; i++) await page.keyboard.press('ArrowLeft') }
      await page.waitForFunction(() => {
        const split = document.querySelector('[data-docked-split]'), handle = split?.querySelector('[role="separator"]')
        return handle && Number(handle.getAttribute('aria-valuemax')) === Math.min(720, Math.max(0, Math.floor(split.getBoundingClientRect().width - handle.getBoundingClientRect().width - 360)))
      })
      await page.evaluate(() => document.fonts.ready)
      const facts = await page.getByTestId('thread-header').evaluate(header => {
        const lane = header.getBoundingClientRect()
        const controls = [...header.querySelectorAll('button')].map(node => {
          const r = node.getBoundingClientRect()
          return { name: node.getAttribute('aria-label') ?? node.textContent?.trim(), left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height }
        }).filter(r => r.width > 0 && r.height > 0)
        const failures = []
        for (const a of controls) {
          if (a.left < lane.left || a.right > lane.right || a.top < lane.top || a.bottom > lane.bottom) failures.push(`clipped ${a.name}`)
          for (const b of controls) if (a !== b && a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top) failures.push(`overlap ${a.name} / ${b.name}`)
        }
        const crumb = header.querySelector('[aria-label="Breadcrumb"]').getBoundingClientRect()
        if (controls.some(r => r.left < crumb.right && r.right > crumb.left && r.top < crumb.bottom && r.bottom > crumb.top)) failures.push('breadcrumb overlaps controls')
        return { laneWidth: lane.width, height: lane.height, controls, failures }
      })
      assert.equal(facts.height, 32, `${scheme}/${width}: header height`)
      assert.deepEqual(facts.failures, [], `${scheme}/${width}: geometry ${JSON.stringify(facts)}`)
      assert.ok(facts.laneWidth >= 360)
      rows.push({ scheme, viewport: width, dock: Number(await dock.getAttribute('aria-valuenow')), ...facts })
      return facts
    }
    await inspect(900)
    assert.equal(await page.getByRole('button', { name: 'View KDL', exact: true }).isVisible(), false)
    await page.getByRole('button', { name: 'Conversation menu', exact: true }).click()
    await page.getByRole('menuitem', { name: 'View KDL', exact: true }).click()
    await page.getByRole('dialog').waitFor()
    await page.keyboard.press('Escape')
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    if (out) await page.screenshot({ path: `${out}/app-${scheme}-900-main-lane.png` })
    await inspect(1440)
    assert.equal(Number(await dock.getAttribute('aria-valuenow')), 720)
    if (out) await page.screenshot({ path: `${out}/app-${scheme}-1440-max-dock.png` })
    await inspect(1440, 440)
    if (out) {
      const image = await page.screenshot({ path: `${out}/app-${scheme}-1440.png` })
      await page.getByRole('button', { name: 'Captured changes information' }).click()
      await page.getByTestId('diff-panel-info').waitFor({ state: 'visible' })
      await page.screenshot({ path: `${out}/app-${scheme}-1440-provenance.png` })
      const reference = await readFile(`${out}/rm1-slice-${scheme}-1440.png`)
      await page.setViewportSize({ width: 2880, height: 940 })
      await page.setContent(`<html><body style="margin:0;background:${scheme === 'dark' ? '#111' : '#fff'};font:14px system-ui;color:${scheme === 'dark' ? '#fff' : '#111'}"><div style="display:flex"><figure style="margin:0"><figcaption style="height:40px;padding:10px;box-sizing:border-box">Live app · synthetic integration fixtures · ${scheme}</figcaption><img width="1440" src="data:image/png;base64,${image.toString('base64')}"></figure><figure style="margin:0"><figcaption style="height:40px;padding:10px;box-sizing:border-box">Accepted RM1 Slice reference · ${scheme}</figcaption><img width="1440" src="data:image/png;base64,${reference.toString('base64')}"></figure></div></body></html>`)
      await page.screenshot({ path: `${out}/${scheme}-1440-comparison.png` })
    }
    assert.deepEqual(errors, [], 'Page exceptions')
    await context.close()
  }
  const receipt = { pass: true, rows }
  console.log(JSON.stringify(receipt))
  if (out) await writeFile(`${out}/header-geometry-browser.json`, JSON.stringify(receipt, null, 2) + '\n')
} finally { await browser?.close(); await server.close() }
