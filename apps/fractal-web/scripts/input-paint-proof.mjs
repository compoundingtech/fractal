#!/usr/bin/env node
// Run through gate-slot --class heavy. Uses the production capture/paint implementation,
// real trusted browser clicks and a test exporter; always closes its isolated browser.
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../../..', import.meta.url))
const session = `input-paint-${randomBytes(4).toString('hex')}`
const run = (args) => new Promise((resolve, reject) => {
  const child = spawn('playwright-cli', [`-s=${session}`, ...args], { cwd: root, stdio: 'inherit' })
  child.once('error', reject)
  child.once('exit', (code) => code === 0 ? resolve() : reject(new Error(`playwright-cli exited ${code}`)))
})
const vite = await createServer({ root, configFile: false, optimizeDeps: { entries: [] }, server: { host: '127.0.0.1', port: 0 }, plugins: [{
  name: 'input-paint-proof', configureServer(server) {
    server.middlewares.use('/input-paint-proof', async (req, res, next) => {
      if (req.url?.includes('html-proxy')) { next(); return }
      res.setHeader('Content-Type', 'text/html')
      const html = `<!doctype html><div id="react"></div><button id="hide">Hide before paint</button><output></output>
<script type="module">
import { createElement, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { flushSync } from 'react-dom'
import { installInputPaint } from '/apps/fractal-web/src/telemetry/inputPaint.ts'
import { makeFrameIngest } from '/apps/fractal-web/src/data/frameIngest.ts'
window.samples = []
window.streams = 0
const ingest = makeFrameIngest({ write: ({ value }) => {
  document.querySelector('output').textContent = value
  window.streams++
} })
// Registered before telemetry: this ingest rAF precedes capture's fallback rAF.
window.addEventListener('click', event => {
  if (event.target.id === 'before') ingest.accept({ key: 'stream', value: 'before' })
}, true)
const App = () => {
  const [count, setCount] = useState(0)
  return createElement('div', {},
    createElement('button', { id: 'paint', onClick: () => setCount(n => n + 1) }, 'Paint ' + count),
    createElement('button', { id: 'stopped', onClick: event => { event.stopPropagation(); setCount(n => n + 1) } }, 'Stopped ' + count),
    createElement('button', { id: 'before', onClick: event => event.stopPropagation() }, 'No-op pre-registered ingest'),
    createElement('button', { id: 'after', onClick: event => {
      event.stopPropagation()
      ingest.accept({ key: 'stream', value: 'after' })
    } }, 'No-op handler-registered ingest'),
  )
}
flushSync(() => createRoot(document.querySelector('#react')).render(createElement(App)))
window.stopCapture = installInputPaint({ emit: sample => window.samples.push(sample) })
document.querySelector('#hide').onclick = () => {
  document.querySelector('output').textContent = 'unpainted'
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' })
  document.dispatchEvent(new Event('visibilitychange'))
}
window.ready = true
</script>`
      res.end(await server.transformIndexHtml('/input-paint-proof.html', html))
    })
  },
}] })
try {
  await vite.listen()
  const address = vite.httpServer.address()
  if (typeof address !== 'object' || address === null) throw new Error('No listener')
  await run(['open', `http://127.0.0.1:${address.port}/input-paint-proof`])
  await run(['run-code', `async page => {
    await page.waitForFunction(() => window.ready)
    await page.locator('#paint').click()
    await page.waitForFunction(() => window.samples.length === 1)
    const painted = await page.evaluate(() => window.samples[0])
    if (painted.attributes.outcome !== 'painted' || painted.attributes['input.kind'] !== 'mouse' || painted.attributes.read_to_frame_ms <= 0 || painted.attributes.read_to_frame_ms > 2000) throw new Error(JSON.stringify(painted))
    await page.evaluate(() => new Promise(resolve => { let n = 0; const tick = () => ++n === 8 ? resolve() : requestAnimationFrame(tick); requestAnimationFrame(tick) }))
    if (await page.evaluate(() => window.samples.length) !== 1) throw new Error('Idle emitted input samples')
    await page.locator('#before').click()
    await page.waitForFunction(() => window.streams === 1)
    await page.locator('#after').click()
    await page.waitForFunction(() => window.streams === 2)
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    if (await page.evaluate(() => window.samples.length) !== 1) throw new Error('rAF ingest mutation was attributed to a no-op click')
    await page.locator('#stopped').click()
    await page.waitForFunction(() => window.samples.length === 2)
    if (await page.evaluate(() => window.samples[1].attributes.outcome) !== 'painted') throw new Error('Stopped propagation lost the React microtask commit')
    await page.locator('#hide').click()
    const samples = await page.evaluate(() => { window.stopCapture(); return window.samples })
    if (samples.length !== 3 || samples[2].attributes.outcome !== 'unacknowledged') throw new Error(JSON.stringify(samples))
    console.log(JSON.stringify({ status: 'pass', idleFrames: 8, rafIngestBeforeAndAfterCaptureExcluded: true, stoppedReactCommitPainted: true, samples }))
    return { status: 'pass', idleFrames: 8, rafIngestBeforeAndAfterCaptureExcluded: true, stoppedReactCommitPainted: true, samples }
  }`])
} finally {
  try { await run(['close']) } finally { await vite.close() }
}
