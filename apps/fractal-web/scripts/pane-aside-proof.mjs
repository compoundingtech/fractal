#!/usr/bin/env node
/**
 * Browser proof: every pane owns its side panel. Renders src/web/paneAside.browser.tsx (the real
 * LiveAgentWorkspace) with the app's own Vite config in managed Chromium, at 1280x800 and 440x900, in the
 * light and the dark scheme. The fixture asserts in-page: no global dock; a wide pane opens its aside by
 * default and an explicit toggle holds in its band; a split gives each pane its own aside, closed by default
 * when too narrow; the panel tab is remembered per kind; a phone opens the panel as a bottom sheet.
 *
 *   node apps/fractal-web/scripts/pane-aside-proof.mjs
 */
import { execFileSync, spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createServer, loadConfigFromFile } from 'vite'

const app = fileURLToPath(new URL('..', import.meta.url))
const git = (...args) => execFileSync('git', ['-C', app, ...args], { encoding: 'utf8' }).trim()
const source = { rev: git('rev-parse', 'HEAD'), dirty: git('status', '--porcelain', '--', '.').length > 0 }
const cases = [1280, 440].flatMap(width => ['light', 'dark'].map(scheme => ({ viewport: { width, height: width === 440 ? 900 : 800 }, scheme })))
const scratchRoot = fileURLToPath(new URL('../../../tmp', import.meta.url))
await mkdir(scratchRoot, { recursive: true })
const scratch = await mkdtemp(`${scratchRoot}/pane-aside-proof-`)
const session = `pane-aside-${randomBytes(4).toString('hex')}`
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
  optimizeDeps: { ...loaded.config.optimizeDeps, entries: ['pane-aside.browser.html'], include: ['react', 'react-dom/client', '@assistant-ui/react'] },
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
    for (const { viewport, scheme } of ${JSON.stringify(cases)}) {
      const tab = await page.context().newPage()
      const errors = []
      tab.on('pageerror', (error) => errors.push(String(error)))
      await tab.setViewportSize(viewport)
      await tab.goto('http://127.0.0.1:${port}/pane-aside.browser.html?scheme=' + scheme)
      await tab.waitForFunction(() => document.body.dataset.testResult !== undefined, { timeout: 90000 })
      const proof = await tab.evaluate(() => ({ result: document.body.dataset.testResult, proof: document.body.dataset.proof }))
      results.push({ viewport, scheme, ...proof, errors })
      await tab.close()
    }
    return 'PROOF ' + JSON.stringify(results)
  }`], true)
  const line = output.split('\n').find((text) => text.includes('PROOF '))
  if (line === undefined) throw new Error(`No proof result in browser output: ${output}`)
  // playwright-cli prints the returned string JSON-quoted.
  const raw = line.slice(line.indexOf('PROOF '))
  const results = JSON.parse(JSON.parse(`"${raw.replace(/"\s*$/, '').slice('PROOF '.length)}"`))
  const receipts = results.map(({ viewport, scheme, result, proof, errors }) => ({ viewport, scheme, result, proof: (() => { try { return JSON.parse(proof) } catch { return { detail: proof } } })(), browserErrors: errors }))
  console.log(JSON.stringify({ ...source, receipts }, null, 1))
  for (const { viewport, scheme, result, proof, browserErrors } of receipts) {
    if (result !== 'pass') throw new Error(`${viewport.width}x${viewport.height} ${scheme}: ${JSON.stringify(proof.failures ?? [])} ${proof.error ?? proof.detail ?? ''}`)
    if (browserErrors.length > 0) throw new Error(`${viewport.width}x${viewport.height} ${scheme}: browser errors ${browserErrors.join(' | ')}`)
  }
  console.log('PASS each pane owns its side panel at 1280 and 440, light and dark')
} finally {
  try { await run(['close']) } finally { await server.close(); await rm(scratch, { recursive: true, force: true }) }
}
