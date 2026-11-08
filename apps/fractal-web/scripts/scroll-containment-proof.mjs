#!/usr/bin/env node
/**
 * Browser proof: the shell is a fixed-viewport layout that the document never scrolls. Renders
 * src/web/scrollContainment.browser.tsx (the real LiveAgentWorkspace with the right panel's long
 * Resources tab open) with the app's own Vite config in managed Chromium at a desktop, a tall and a
 * phone viewport. Each asserts in-page that no absolutely positioned shell descendant escapes to the
 * initial containing block, the document is exactly the viewport tall and a scroll request moves
 * nothing; then this runner wheels over the panel past its end, as the user did, and re-measures.
 *
 * `--without-fix` is the negative control: it reverts the shell's containing block (position:static,
 * overflow:hidden) and the Resources scroller's (position:static), and asserts the proof FAILS with
 * the document growing past the viewport and the shell scrolled up. The fixture loads app.css, not
 * index.html, so the root `overflow:hidden` lock cannot mask a containment leak.
 *
 *   node apps/fractal-web/scripts/scroll-containment-proof.mjs [--without-fix]
 */
import { execFileSync, spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createServer, loadConfigFromFile } from 'vite'

const app = fileURLToPath(new URL('..', import.meta.url))
const withoutFix = process.argv.includes('--without-fix')
const git = (...args) => execFileSync('git', ['-C', app, ...args], { encoding: 'utf8' }).trim()
const source = { rev: git('rev-parse', 'HEAD'), dirty: git('status', '--porcelain', '--', '.').length > 0 }
const viewports = [{ width: 1280, height: 800 }, { width: 2000, height: 1790 }, { width: 440, height: 900 }]
const scratchRoot = fileURLToPath(new URL('../../../tmp', import.meta.url))
await mkdir(scratchRoot, { recursive: true })
const scratch = await mkdtemp(`${scratchRoot}/scroll-containment-proof-`)
const session = `scroll-containment-${randomBytes(4).toString('hex')}`
const run = (args, capture = false) => {
  const { promise, resolve, reject } = Promise.withResolvers()
  const child = spawn('playwright-cli', [`-s=${session}`, ...args], { cwd: scratch, stdio: ['ignore', capture ? 'pipe' : 'inherit', 'inherit'] })
  let out = ''
  child.stdout?.on('data', (chunk) => { out += chunk })
  const timer = setTimeout(() => { child.kill('SIGTERM'); reject(new Error('Browser command deadline exceeded')) }, 180_000)
  child.on('error', (error) => { clearTimeout(timer); reject(error) })
  child.on('exit', (code) => { clearTimeout(timer); code === 0 ? resolve(out) : reject(new Error(`Browser command exited ${code}`)) })
  return promise
}
const loaded = await loadConfigFromFile({ command: 'serve', mode: 'development' }, `${app}/vite.config.ts`, app)
const server = await createServer({
  ...loaded.config,
  configFile: false,
  root: `${app}/src/web`,
  optimizeDeps: { ...loaded.config.optimizeDeps, entries: ['scroll-containment.browser.html'], include: ['react', 'react-dom/client', '@assistant-ui/react'] },
  // The fixture owns its data; never connect to an inherited live gateway.
  plugins: loaded.config.plugins.flat().filter((plugin) => plugin?.name !== 'wf:shared-client-gateway'),
  server: { ...loaded.config.server, host: '127.0.0.1', port: 0, strictPort: false, hmr: false },
})
try {
  await server.listen()
  const { port } = server.httpServer.address()
  await run(['open', 'about:blank'])
  const output = await run(['run-code', `async (page) => {
    const results = []
    for (const viewport of ${JSON.stringify(viewports)}) {
      const tab = await page.context().newPage()
      const errors = []
      tab.on('pageerror', (error) => errors.push(String(error)))
      await tab.setViewportSize(viewport)
      ${withoutFix ? `await tab.addInitScript(() => document.addEventListener('DOMContentLoaded', () => {
        const style = document.createElement('style')
        style.textContent = '[data-testid="live-agent-workspace"] { position: static !important; overflow: hidden !important } [data-agent-resources] { position: static !important }'
        document.head.append(style)
      }))` : ''}
      await tab.goto('http://127.0.0.1:${port}/scroll-containment.browser.html')
      await tab.waitForFunction(() => document.body.dataset.testResult !== undefined, { timeout: 60000 })
      const proof = await tab.evaluate(() => ({ result: document.body.dataset.testResult, proof: document.body.dataset.proof }))
      let wheeled
      const panel = tab.locator('[data-agent-resources]')
      if (await panel.count() === 1 && await panel.isVisible()) {
        const box = await panel.boundingBox()
        await tab.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
        for (let step = 0; step < 12; step++) { await tab.mouse.wheel(0, 800); await tab.waitForTimeout(80) }
        await tab.waitForTimeout(300)
        wheeled = await tab.evaluate(() => window.scrollContainmentMeasure())
      }
      results.push({ viewport, ...proof, wheeled, errors })
      await tab.close()
    }
    return 'PROOF ' + JSON.stringify(results)
  }`], true)
  const line = output.split('\n').find((text) => text.includes('PROOF '))
  if (line === undefined) throw new Error(`No proof result in browser output: ${output}`)
  // playwright-cli prints the returned string JSON-quoted.
  const raw = line.slice(line.indexOf('PROOF '))
  const results = JSON.parse(JSON.parse(`"${raw.replace(/"\s*$/, '').slice('PROOF '.length)}"`))
  const receipts = results.map(({ viewport, result, proof, wheeled, errors }) => {
    const facts = (() => { try { return JSON.parse(proof) } catch { return { detail: proof } } })()
    return { viewport, result, opened: facts.opened, requested: facts.requested, wheeled, failures: facts.failures ?? [facts.detail], browserErrors: errors.length }
  })
  console.log(JSON.stringify({ mode: withoutFix ? 'without-fix' : 'with-fix', ...source, receipts }))
  if (withoutFix) {
    // The control must fail for the reason the fix addresses: the document grows and the shell scrolls up.
    const failed = receipts.filter(({ result, requested, wheeled }) => result === 'fail' && requested?.scrollHeight > requested?.innerHeight && (requested?.scrollY > 0 || wheeled?.scrollY > 0))
    if (failed.length === 0) throw new Error('Negative control did not fail as expected')
    for (const { viewport, requested, wheeled } of failed) console.log(`PASS negative control at ${viewport.width}x${viewport.height}: without the fix the document grows to ${requested.scrollHeight}px and the shell scrolls to ${wheeled?.shellTop ?? requested.shellTop}px`)
  } else {
    for (const receipt of receipts) {
      const { viewport, result, wheeled, failures, browserErrors } = receipt
      if (result !== 'pass') throw new Error(`${viewport.width}x${viewport.height}: ${JSON.stringify(failures)}`)
      if (wheeled !== undefined && (wheeled.scrollY !== 0 || wheeled.shellTop !== 0 || wheeled.scrollHeight !== wheeled.innerHeight)) throw new Error(`${viewport.width}x${viewport.height}: wheeling moved the document ${JSON.stringify(wheeled)}`)
      if (browserErrors > 0) throw new Error(`${viewport.width}x${viewport.height}: browser errors`)
    }
    console.log('PASS the document never scrolls: the shell contains its absolutely positioned descendants at every viewport, and wheeling past the Resources panel end moves nothing')
  }
} finally {
  try { await run(['close']) } finally { await server.close(); await rm(scratch, { recursive: true, force: true }) }
}
