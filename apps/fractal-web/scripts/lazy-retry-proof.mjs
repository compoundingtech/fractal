#!/usr/bin/env node
/** Run on the designated browser-proof host, in a named PTY:
 * nix develop .#web --command node apps/fractal-web/scripts/lazy-retry-proof.mjs
 * Uses Nix's PLAYWRIGHT_MODULE and CHROMIUM_PATH; FIREFOX_PATH / WEBKIT_PATH
 * optionally select matching Nix executables. No browser downloads or browser-path override.
 * WF_PROOF_ENGINES=chromium,firefox,webkit selects engines (default: all three).
 * WF_PROOF_DIR optionally retains JSON evidence. WebKit's software EGL environment
 * (LIBGL_ALWAYS_SOFTWARE, EGL_PLATFORM, __EGL_VENDOR_LIBRARY_FILENAMES) is inherited.
 */
import assert from 'node:assert/strict'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { build, preview } from 'vite'
import { fractalLazyChunks } from './vite-lazy-chunks.mjs'

const app = fileURLToPath(new URL('..', import.meta.url))
const fixture = fileURLToPath(new URL('./lazy-retry-fixture/', import.meta.url))
const scratchRoot = `${app}/../../tmp/lazy-retry-proof`
await mkdir(scratchRoot, { recursive: true })
const outDir = await mkdtemp(`${scratchRoot}/build-`)
const module = process.env.PLAYWRIGHT_MODULE
const playwright = await import(module?.startsWith('/') ? pathToFileURL(module).href : module ?? 'playwright')
const engines = (process.env.WF_PROOF_ENGINES ?? 'chromium,firefox,webkit').split(',')
for (const engine of engines) assert(['chromium', 'firefox', 'webkit'].includes(engine), `Unknown engine: ${engine}`)
const receipts = []
let server
try {
  const result = await build({
    configFile: false, root: fixture, base: '/proof/',
    plugins: [fractalLazyChunks({ conversation: `${fixture}feature.js` })],
    build: {
      outDir, emptyOutDir: true,
      rolldownOptions: { output: { codeSplitting: {
        includeDependenciesRecursively: false,
        groups: [{ name: 'static-dependency', test: /[/\\]lazy-retry-fixture[/\\]dependency\.js$/ }],
      } } },
    },
  })
  assert(!Array.isArray(result) && 'output' in result, 'Expected one production bundle')
  const root = result.output.find(asset => asset.type === 'chunk' && asset.facadeModuleId === `${fixture}feature.js`)
  const dependency = result.output.find(asset => asset.type === 'chunk' && Object.keys(asset.modules).includes(`${fixture}dependency.js`))
  assert(root && dependency && root.fileName !== dependency.fileName, 'Fixture needs a separately emitted static dependency')
  assert(root.imports.includes(dependency.fileName), 'Lazy root must statically import the separate dependency')
  const html = await readFile(`${outDir}/index.html`, 'utf8')
  const manifest = html.match(/<script\b[^>]*\bid=["']fractal-lazy-chunk-assets["'][^>]*>([\s\S]*?)<\/script>/)
  assert(manifest, 'Real plugin must inject the recovery manifest')
  const assets = JSON.parse(manifest[1]).conversation
  assert(Array.isArray(assets) && assets.includes(`/proof/${root.fileName}`) && assets.includes(`/proof/${dependency.fileName}`))
  const css = assets.filter(asset => asset.endsWith('.css'))
  assert.equal(css.length, 1, 'Fixture must include its lazy CSS in the recursive closure')
  server = await preview({ configFile: false, root: fixture, base: '/proof/', build: { outDir }, preview: { host: '127.0.0.1', port: 0, strictPort: false } })
  const origin = `http://127.0.0.1:${server.httpServer.address().port}`
  for (const engine of engines) {
    const executablePath = process.env[`${engine.toUpperCase()}_PATH`]
    const browser = await playwright[engine].launch({ ...(executablePath ? { executablePath } : {}) })
    try {
      for (const [target, path] of Object.entries({ root: `/proof/${root.fileName}`, dependency: `/proof/${dependency.fileName}`, css: css[0] })) {
        const context = await browser.newContext()
        try {
          const page = await context.newPage()
          page.setDefaultTimeout(15_000)
          let blocked = true
          let aborted = 0
          let phase = 'first-load'
          const requests = []
          const stylesheetState = () => page.evaluate(() => [...document.querySelectorAll('link')].map(link => {
            let rules
            try { rules = link.sheet ? [...link.sheet.cssRules].map(rule => rule.cssText) : null }
            catch (error) { rules = String(error) }
            return { href: link.href, rel: link.rel, media: link.media, disabled: link.disabled, sheet: Boolean(link.sheet), rules }
          }))
          const responses = []
          page.on('response', response => responses.push({ phase, url: response.url(), status: response.status() }))
          page.on('requestfailed', request => responses.push({ phase, url: request.url(), failure: request.failure() }))
          await page.route(`${origin}${path}`, route => {
            if (blocked) { aborted++; return route.abort('failed') }
            return route.continue()
          })
          page.on('request', request => requests.push({ phase, path: new URL(request.url()).pathname, type: request.resourceType() }))
          await page.goto(`${origin}/proof/`)
          const draft = page.getByRole('textbox', { name: 'Draft' })
          await draft.fill('Keep this unsent draft')
          await page.getByRole('combobox', { name: 'Conversation' }).selectOption('Beta')
          await draft.evaluate(element => {
            element.setSelectionRange(5, 9, 'backward')
            element.dispatchEvent(new Event('select'))
          })
          const documentId = await page.evaluate(() => window.lazyProof.document)
          await page.getByRole('button', { name: 'Load conversation', exact: true }).click()
          await page.getByRole('button', { name: 'Try again', exact: true }).waitFor()
          assert(aborted > 0, `${engine}/${target}: first import must hit the aborted real asset`)
          assert.equal(await page.evaluate(() => window.lazyProof.attempts), 1)
          assert.equal(await page.getByRole('button', { name: 'Reload and retry', exact: true }).count(), 0)
          const stylesBeforeRetry = target === 'css' ? await stylesheetState() : undefined
          blocked = false
          phase = 'same-document-retry'
          await page.getByRole('button', { name: 'Try again', exact: true }).click()
          await page.waitForFunction(() => ['ready', 'failed'].includes(document.querySelector('#status').textContent))
          assert.equal(await page.evaluate(() => window.lazyProof.document), documentId, 'First retry must not navigate')
          assert.equal(await page.evaluate(() => window.lazyProof.attempts), 2, 'First retry must call the real importer again')
          const reloaded = await page.locator('#status').textContent() === 'failed'
          if (target === 'css') assert.equal(reloaded, false, 'Failed CSS preload must recover in the same document')
          if (reloaded) {
            await page.getByRole('button', { name: 'Reload and retry', exact: true }).waitFor()
            assert.equal(await page.getByRole('button', { name: 'Try again', exact: true }).count(), 0)
            phase = 'reload'
            await Promise.all([
              page.waitForEvent('framenavigated', frame => frame === page.mainFrame()),
              page.getByRole('button', { name: 'Reload and retry', exact: true }).click(),
            ])
            await page.waitForFunction(() => document.querySelector('#status')?.textContent === 'ready')
            assert.notEqual(await page.evaluate(() => window.lazyProof.document), documentId)
            for (const asset of assets) assert(requests.some(request => request.phase === 'reload' && request.path === asset && request.type === 'fetch'), `Reload must fetch closure asset ${asset}`)
          }
          await page.getByText('Conversation loaded', { exact: true }).waitFor()
          const facts = await page.evaluate(() => {
            const draft = document.querySelector('#draft')
            return {
              draft: draft.value, selection: document.querySelector('#selection').value,
              start: draft.selectionStart, end: draft.selectionEnd, direction: draft.selectionDirection,
              color: getComputedStyle(document.querySelector('#feature')).color,
              rootExecutions: Number(sessionStorage.getItem('lazy-proof-root-executions')),
              dependencyExecutions: Number(sessionStorage.getItem('lazy-proof-dependency-executions')),
            }
          })
          if (facts.color !== 'rgb(10, 80, 170)') {
            const diagnostics = {
              engine, target, facts, stylesBeforeRetry, stylesAfterRetry: await stylesheetState(), requests, responses,
              feature: await page.locator('#feature').evaluate(element => element.outerHTML),
            }
            const report = JSON.stringify(diagnostics, null, 2)
            console.error(report)
            if (process.env.WF_PROOF_DIR) {
              await mkdir(process.env.WF_PROOF_DIR, { recursive: true })
              await writeFile(`${process.env.WF_PROOF_DIR}/lazy-retry-${engine}-${target}-failure.json`, report)
            }
          }
          assert.deepEqual(facts, {
            draft: 'Keep this unsent draft', selection: 'Beta', start: 5, end: 9, direction: 'backward',
            color: 'rgb(10, 80, 170)', rootExecutions: 1, dependencyExecutions: 1,
          }, `${engine}/${target}: recovery must preserve draft/selection, apply CSS and execute each module once`)
          receipts.push({ engine, target, aborted, recovery: reloaded ? 'reload' : 'same-document', facts, requests })
        } finally { await context.close() }
      }
    } finally { await browser.close() }
  }
  const report = JSON.stringify({ status: 'passed', mode: 'production-vite', receipts }, null, 2)
  if (process.env.WF_PROOF_DIR) {
    await mkdir(process.env.WF_PROOF_DIR, { recursive: true })
    await writeFile(`${process.env.WF_PROOF_DIR}/lazy-retry.json`, report)
  }
  console.log(report)
} finally {
  if (server) await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()))
  await rm(outDir, { recursive: true, force: true })
}
