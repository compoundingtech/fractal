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
const vite = await createServer({ root, configFile: false, server: { host: '127.0.0.1', port: 0 }, plugins: [{
  name: 'input-paint-proof', configureServer(server) {
    server.middlewares.use('/input-paint-proof', (_req, res) => {
      res.setHeader('Content-Type', 'text/html')
      res.end(`<!doctype html><button id="paint">Paint</button><button id="hide">Hide before paint</button><output></output>
<script type="module">
import { installInputPaint } from '/apps/fractal-web/src/telemetry/inputPaint.ts'
window.samples = []
window.stopCapture = installInputPaint({ emit: sample => window.samples.push(sample) })
document.querySelector('#paint').onclick = () => { document.querySelector('output').textContent = 'painted' }
document.querySelector('#hide').onclick = () => {
  document.querySelector('output').textContent = 'unpainted'
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' })
  document.dispatchEvent(new Event('visibilitychange'))
}
window.ready = true
</script>`)
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
    await page.locator('#hide').click()
    const samples = await page.evaluate(() => { window.stopCapture(); return window.samples })
    if (samples.length !== 2 || samples[1].attributes.outcome !== 'unacknowledged') throw new Error(JSON.stringify(samples))
    console.log(JSON.stringify({ status: 'pass', idleFrames: 8, samples }))
    return { status: 'pass', idleFrames: 8, samples }
  }`])
} finally {
  try { await run(['close']) } finally { await vite.close() }
}
