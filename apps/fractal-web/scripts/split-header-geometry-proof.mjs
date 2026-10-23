#!/usr/bin/env node
// Real app regression: lane-responsive header, not viewport-responsive chrome.
// Run through gate-slot with CI=1. Managed environments may set PLAYWRIGHT_MODULE and
// CHROMIUM_PATH. SPLIT_PROOF_DIR optionally records screenshots and the JSON receipt.
import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createServer, loadConfigFromFile } from 'vite'
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const app = fileURLToPath(new URL('..', import.meta.url))
const out = process.env.SPLIT_PROOF_DIR
if (out) await mkdir(out, { recursive: true })
const loaded = await loadConfigFromFile({ command: 'serve', mode: 'development' }, `${app}/vite.config.ts`, app)
const server = await createServer({ ...loaded.config, configFile: false, root: `${app}/src/web`,
  resolve: { ...loaded.config.resolve, dedupe: ['react', 'react-dom'] },
  optimizeDeps: { ...loaded.config.optimizeDeps, force: true, entries: ['split-header-geometry.browser.html'], include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', 'react/jsx-dev-runtime', '@assistant-ui/react', '@effect/atom-react'] },
  plugins: loaded.config.plugins.flat().filter(plugin => plugin?.name !== 'wf:shared-client-gateway'),
  server: { ...loaded.config.server, host: '127.0.0.1', port: 0, strictPort: false, hmr: false },
})
let browser
const rows = []
try {
  await server.listen()
  const port = server.httpServer.address().port
  browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
  for (const scheme of ['dark', 'light']) for (const failed of [false, true]) for (const [width, preferred] of [[900, 720], [1440, 720], [1440, 440]]) {
    // Each snapshot opens the real app with saved preferences, rather than inheriting focus
    // from the keyboard resize test. Pointer/keyboard resizing is covered in kit/app tests.
    const context = await browser.newContext({ viewport: { width, height: 900 }, colorScheme: scheme })
    await context.addInitScript(({ scheme, width, preferred }) => {
      localStorage.setItem('wf.ui@1', JSON.stringify({ version: 1, entries: {
        'round2.scheme': { version: 1, value: scheme, clock: 1, writer: 'split-header-proof' },
        'round2.sidebarRatio': { version: 1, value: width === 900 ? 208 / 900 : 256 / 1440, clock: 1, writer: 'split-header-proof' },
      } }))
      localStorage.setItem('composition.live.agent/split-0.changes', String(preferred))
    }, { scheme, width, preferred })
    const page = await context.newPage(), errors = []
    page.on('pageerror', error => errors.push(String(error)))
    await page.goto(`http://127.0.0.1:${port}/split-header-geometry.browser.html${failed ? '?sync-failed' : ''}`)
    await page.getByRole('button', { name: 'Toggle right panel', exact: true }).click()
    const inspector = page.getByRole('complementary', { name: 'Agent inspector', exact: true })
    await inspector.waitFor()
    await inspector.getByRole('tab', { name: 'Details', exact: true }).waitFor()
    if (out && !failed && width === 1440 && preferred === 440) {
      await page.evaluate(() => document.fonts.ready)
      await page.screenshot({ path: `${out}/app-${scheme}-1440-details.png` })
    }
    await inspector.getByRole('tab', { name: 'Changes', exact: true }).click()
    await page.getByTestId('diff-code').waitFor()
    await page.waitForFunction(failed => document.querySelector('[data-testid="transcript-header-status"] [data-testid="sync-line"]')?.getAttribute('data-sync-visible') === String(failed), failed)
    const dock = page.getByRole('separator', { name: 'Resize Changes' })
    if (width === 900) {
      await dock.focus(); await page.keyboard.press('End')
      assert.equal(Number(await dock.getAttribute('aria-valuenow')), Number(await dock.getAttribute('aria-valuemax')))
      assert.equal(await page.getByRole('button', { name: 'View KDL', exact: true }).isVisible(), false)
      await page.getByRole('button', { name: 'Conversation menu', exact: true }).click()
      assert.equal(await page.getByRole('menuitem', { name: 'Sync details', exact: true }).count(), failed ? 1 : 0, 'Diagnostics are offered only for retained failure')
      await page.getByRole('menuitem', { name: 'View KDL', exact: true }).click()
      await page.getByRole('dialog').waitFor()
      await page.keyboard.press('Escape')
      await page.getByRole('dialog').waitFor({ state: 'hidden' })
    }
    await page.mouse.move(800, 500)
    await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur())
    await page.getByRole('tooltip').waitFor({ state: 'hidden' })
    await page.evaluate(() => document.fonts.ready)
    const facts = await page.getByTestId('thread-header').evaluate(header => {
      const lane = header.getBoundingClientRect()
      const controls = [...header.querySelectorAll('button')].map(node => {
        const r = node.getBoundingClientRect(), style = getComputedStyle(node)
        return { name: node.getAttribute('aria-label') ?? node.textContent?.trim(), left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height, visible: style.visibility !== 'hidden' && style.display !== 'none' }
      }).filter(r => r.visible && r.width > 0 && r.height > 0)
      const failures = []
      for (const a of controls) {
        if (a.left < lane.left || a.right > lane.right || a.top < lane.top || a.bottom > lane.bottom) failures.push(`clipped ${a.name}`)
        for (const b of controls) if (a !== b && a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top) failures.push(`overlap ${a.name} / ${b.name}`)
      }
      const crumb = header.querySelector('[aria-label="Breadcrumb"]').getBoundingClientRect()
      if (controls.some(r => r.left < crumb.right && r.right > crumb.left && r.top < crumb.bottom && r.bottom > crumb.top)) failures.push('breadcrumb overlaps controls')
      const gear = controls.find(r => r.name === 'Conversation menu'), terminal = controls.find(r => r.name === 'Toggle terminal drawer')
      const clusterGap = gear && terminal ? terminal.left - gear.right : undefined
      if (clusterGap !== undefined && clusterGap > 16 && !header.querySelector('[data-sync-state="Failed"]')) failures.push(`empty native cluster gap ${clusterGap}`)
      return { laneWidth: lane.width, height: lane.height, controls, clusterGap, failures }
    })
    const rest = await dock.evaluate(node => {
      const style = getComputedStyle(node), header = document.querySelector('[data-testid="diff-panel-header"]')
      return { width: node.getBoundingClientRect().width, color: style.backgroundColor, neutral: header ? getComputedStyle(header).borderBottomColor : null, outline: style.outlineStyle, focused: node.matches(':focus-visible'), hovered: node.matches(':hover') }
    })
    assert.equal(facts.height, 32, `${scheme}/${width}: header height`)
    assert.deepEqual(facts.failures, [], `${scheme}/${width}: geometry ${JSON.stringify(facts)}`)
    assert.ok(facts.laneWidth >= 360)
    assert.equal(Number(await dock.getAttribute('aria-valuenow')), width === 900 ? 330 : preferred)
    assert.equal(rest.width, 1, 'Resting separator is one hairline')
    assert.equal(rest.color, rest.neutral, 'Resting separator uses the neutral pane border token')
    assert.equal(rest.focused, false); assert.equal(rest.hovered, false)
    if (failed) {
      const retry = page.getByRole('button', { name: 'Retry loading conversation', exact: true })
      assert.ok((await retry.boundingBox()).width >= 24, 'Retry keeps its target')
      const reason = page.getByTestId('thread-header').getByRole('button', { name: /subscription limit has been reached/ })
      assert.ok((await reason.getAttribute('aria-label')).includes('previously loaded thread and captured changes remain available'))
      const text = page.getByTestId('inline-sync-text')
      assert.equal(await text.evaluate(node => getComputedStyle(node).textOverflow), 'ellipsis')
      await reason.hover()
      await page.getByRole('tooltip').filter({ hasText: 'previously loaded thread and captured changes remain available' }).waitFor()
      await page.mouse.move(800, 500)
      await page.getByRole('tooltip').waitFor({ state: 'hidden' })
      if (width === 900) {
        assert.equal(await page.getByRole('button', { name: 'Sync details for conversation', exact: true }).isVisible(), false)
        assert.ok((await reason.innerText()).includes('Sync failed'))
        await page.getByRole('button', { name: 'Conversation menu', exact: true }).click()
        await page.getByRole('menuitem', { name: 'Sync details', exact: true }).click()
        const diagnostics = page.getByRole('dialog', { name: 'Sync details for conversation', exact: true })
        await diagnostics.waitFor()
        assert.ok((await diagnostics.innerText()).includes('previously loaded thread and captured changes remain available'))
        await page.getByRole('button', { name: 'Close sync details', exact: true }).click()
        await diagnostics.waitFor({ state: 'hidden' })
      }
    }
    rows.push({ scheme, failed, viewport: width, dock: Number(await dock.getAttribute('aria-valuenow')), ...facts, rest })
    if (out) {
      const suffix = failed ? width === 900 ? '900-sync-failed' : `1440-${preferred}-sync-failed` : width === 900 ? '900-main-lane' : preferred === 720 ? '1440-max-dock' : '1440'
      await page.screenshot({ path: `${out}/app-${scheme}-${suffix}.png` })
      if (!failed && width === 1440 && preferred === 440) {
        await page.getByRole('button', { name: 'Captured changes information' }).click()
        await page.getByTestId('diff-panel-info').waitFor({ state: 'visible' })
        await page.screenshot({ path: `${out}/app-${scheme}-1440-provenance.png` })
        await page.getByRole('button', { name: 'Close agent inspector', exact: true }).click()
        await page.setViewportSize({ width: 390, height: 844 })
        await page.getByRole('button', { name: 'Agent details', exact: true }).waitFor()
        await page.getByRole('tab', { name: 'Changes', exact: true }).waitFor()
        await page.screenshot({ path: `${out}/app-${scheme}-390-phone.png` })
        await page.getByRole('button', { name: 'Agent details', exact: true }).click()
        await page.getByRole('complementary', { name: 'Agent inspector', exact: true }).waitFor()
        await page.screenshot({ path: `${out}/app-${scheme}-390-details.png` })
        const detailsSheet = page.getByRole('dialog', { name: 'Agent details', exact: true })
        await detailsSheet.getByRole('button', { name: 'Close agent details', exact: true }).click()
        await detailsSheet.waitFor({ state: 'hidden' })
        await page.getByRole('button', { name: 'Conversation menu', exact: true }).click()
        for (const label of ['Resources', 'View KDL', 'Show all system events']) {
          const item = page.getByRole(label === 'Show all system events' ? 'menuitemcheckbox' : 'menuitem', { name: label === 'Resources' ? /^Resources/ : label, exact: label !== 'Resources' })
          assert.ok((await item.boundingBox()).height >= 44, `Phone menu touch target: ${label}`)
        }
        await page.keyboard.press('Escape')
      }
    }
    if (out && failed && width === 1440 && preferred === 440) {
      await page.getByRole('button', { name: 'Close agent inspector', exact: true }).click()
      await page.setViewportSize({ width: 390, height: 844 })
      await page.getByRole('button', { name: 'Agent details', exact: true }).waitFor()
      for (const label of ['Retry loading conversation', 'Sync details for conversation']) {
        const button = page.getByRole('button', { name: label, exact: true })
        const rect = await button.boundingBox()
        assert.ok(rect && rect.width >= 44 && rect.height >= 44, `Narrow SyncLine touch target: ${label}`)
      }
      await page.getByRole('button', { name: 'Sync details for conversation', exact: true }).click()
      const diagnostics = page.getByRole('dialog', { name: 'Sync details for conversation', exact: true })
      await diagnostics.waitFor()
      assert.ok((await diagnostics.innerText()).includes('previously loaded thread and captured changes remain available'))
      await page.keyboard.press('Escape')
      await diagnostics.waitFor({ state: 'hidden' })
      await page.screenshot({ path: `${out}/app-${scheme}-390-sync-failed.png` })
    }
    assert.deepEqual(errors, [], 'Page exceptions')
    await context.close()
  }
  const receipt = { pass: true, rows }
  console.log(JSON.stringify(receipt))
  if (out) await writeFile(`${out}/header-geometry-browser.json`, JSON.stringify(receipt, null, 2) + '\n')
} finally { await browser?.close(); await server.close() }
