#!/usr/bin/env node
// Run through gate-slot --class heavy. Uses the production capture/paint implementation,
// real trusted browser clicks and a test exporter; always closes its isolated browser.
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'

import { createStylexVitePlugins } from './stylex.mjs'
const root = fileURLToPath(new URL('../../..', import.meta.url))
const session = `input-paint-${randomBytes(4).toString('hex')}`
const run = (args) => new Promise((resolve, reject) => {
  const child = spawn('playwright-cli', [`-s=${session}`, ...args], { cwd: root, stdio: 'inherit' })
  child.once('error', reject)
  child.once('exit', (code) => code === 0 ? resolve() : reject(new Error(`playwright-cli exited ${code}`)))
})
const shellFixtures = {
  'shell-runtime': 'export const live={};export const telemetry={onCommit(){},ux:undefined}',
  'shell-data': 'export const DataSourceProvider=({children})=>children',
  'shell-workspace': `import {createElement} from 'react';export const LiveAgentWorkspace=({overlayHost})=>createElement('div',{ref:overlayHost,'data-app-overlay-host':''})`,
  'shell-meters': 'export const acquireMeasurementEngine=()=>()=>{};export const getDebug=()=>0',
  'shell-build': 'export const buildIdentity={machineVersion:"smoke",dirty:false};export const deploymentId=undefined',
}
const vite = await createServer({ root, configFile: false, optimizeDeps: { entries: [] }, server: { host: '127.0.0.1', port: 0 }, plugins: [{
  name: 'input-paint-shell-fixtures',
  enforce: 'pre',
  resolveId(id, importer) {
    if (id === 'virtual:build-identity') return '\0shell-build'
    if (id.endsWith('/meters.tsx')) return '\0shell-meters'
    if (!importer?.endsWith('/web/App.tsx')) return
    if (id === './liveRuntime.ts') return '\0shell-runtime'
    if (id === '../data/react.tsx') return '\0shell-data'
    if (id === './LiveAgentWorkspace.tsx') return '\0shell-workspace'
  },
  load(id) { return shellFixtures[id.slice(1)] },
}, ...createStylexVitePlugins({ externalPackages: ['@smalltalk/fractal-ui'], entries: [] }), {
  name: 'input-paint-proof', configureServer(server) {
    server.middlewares.use('/input-paint-proof', async (req, res, next) => {
      if (req.url?.includes('html-proxy')) { next(); return }
      res.setHeader('Content-Type', 'text/html')
      const html = `<!doctype html><div id="react"></div><div id="portal"></div><button id="hide">Hide before paint</button><output></output>
<script type="module">
import { createElement, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { flushSync } from 'react-dom'
import { installInputPaint } from '/apps/fractal-web/src/telemetry/inputPaint.ts'
import { makeFrameIngest } from '/apps/fractal-web/src/data/frameIngest.ts'
import { PortalContainerRegistrationContext, RegisteredPortal } from '/packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'
import { App as RealAppShell } from '/apps/fractal-web/src/web/App.tsx'
import 'virtual:overeng-stylex.css'
const portalContainer = document.querySelector('#portal')
const reactRoots = new Set()
const registerPortal = container => { reactRoots.add(container); return () => reactRoots.delete(container) }
window.samples = []
window.streams = 0
const ingest = makeFrameIngest({ write: ({ value }) => {
  document.querySelector('output').textContent = value
  window.streams++
} })
// Registered before telemetry to retain coverage of the explicit ingest boundary.
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
    createElement('button', { id: 'timer-dom', onClick: event => {
      event.stopPropagation()
      setTimeout(() => { document.querySelector('output').textContent = 'timer'; window.timerDomDone = true }, 0)
    } }, 'No-op DOM timer'),
    createElement('button', { id: 'timer-react', onClick: event => {
      event.stopPropagation()
      setTimeout(() => { flushSync(() => setCount(n => n + 1)); window.timerReactDone = true }, 0)
    } }, 'No-op React timer'),
    createElement(RegisteredPortal, { container: portalContainer, children: createElement('div', {},
      ...['bubble', 'capture'].flatMap(phase => {
        const handler = phase === 'capture' ? 'onClickCapture' : 'onClick'
        return [
          createElement('button', { id: 'portal-' + phase, [handler]: event => { event.stopPropagation(); setCount(n => n + 1) } }, 'Portal ' + phase + count),
          ...['dom', 'react'].map(kind => createElement('button', { id: 'portal-' + phase + '-' + kind, [handler]: event => {
            event.stopPropagation()
            setTimeout(() => {
              if (kind === 'dom') document.querySelector('output').textContent = 'portal timer'
              else flushSync(() => setCount(n => n + 1))
              window['portal-' + phase + '-' + kind] = true
            }, 0)
          } }, 'Portal timer ' + phase + kind)),
        ]
      }),
    ) }),
  )
}
const container = document.querySelector('#react')
const reactRoot = createRoot(container)
reactRoots.add(container)
flushSync(() => reactRoot.render(createElement(PortalContainerRegistrationContext.Provider, { value: registerPortal }, createElement(App))))
window.stopCapture = installInputPaint({ reactRoots, emit: sample => window.samples.push(sample) })
document.querySelector('#hide').onclick = () => {
  document.querySelector('output').textContent = 'unpainted'
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' })
  document.dispatchEvent(new Event('visibilitychange'))
}
window.startShell = () => {
  window.stopCapture()
  reactRoot.unmount()
  document.body.replaceChildren()
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' })
  const shell = document.createElement('div')
  document.body.append(shell)
  const shellRoot = createRoot(shell)
  const roots = new Set([shell])
  const register = element => { roots.add(element); return () => roots.delete(element) }
  const Fixture = () => {
    const [count, setCount] = useState(0)
    return createElement('div', { onClick: event => {
      if (event.target.closest('[data-changelog-popover]')) {
        event.stopPropagation()
        setTimeout(() => { flushSync(() => setCount(n => n + 1)); window.shellTimerDone = true }, 0)
      }
    } }, createElement(RealAppShell), createElement('output', { id: 'shell-timer' }, count))
  }
  flushSync(() => shellRoot.render(createElement(PortalContainerRegistrationContext.Provider, { value: register }, createElement(Fixture))))
  window.shellSamples = []
  window.stopCapture = installInputPaint({ reactRoots: roots, emit: sample => window.shellSamples.push(sample) })
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
    await page.locator('#timer-dom').click()
    await page.waitForFunction(() => window.timerDomDone)
    await page.locator('#timer-react').click()
    await page.waitForFunction(() => window.timerReactDone)
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    if (await page.evaluate(() => window.samples.length) !== 1) throw new Error('Intervening timer commit was attributed to a stopped no-op click')
    await page.locator('#stopped').click()
    await page.waitForFunction(() => window.samples.length === 2)
    if (await page.evaluate(() => window.samples[1].attributes.outcome) !== 'painted') throw new Error('Stopped propagation lost the React microtask commit')
    for (const phase of ['bubble', 'capture']) {
      for (const kind of ['dom', 'react']) {
        const id = 'portal-' + phase + '-' + kind
        await page.locator('#' + id).click()
        await page.waitForFunction(id => window[id], id)
      }
    }
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    if (await page.evaluate(() => window.samples.length) !== 2) throw new Error('External portal timer commit attributed to stopped no-op')
    for (const [phase, count] of [['bubble', 3], ['capture', 4]]) {
      await page.locator('#portal-' + phase).click()
      await page.waitForFunction(count => window.samples.length === count, count)
      if (await page.evaluate(() => window.samples.at(-1).attributes.outcome) !== 'painted') throw new Error('Lost stopped external portal commit')
    }
    await page.locator('#hide').click()
    const samples = await page.evaluate(() => { window.stopCapture(); return window.samples })
    if (samples.length !== 5 || samples[4].attributes.outcome !== 'unacknowledged') throw new Error(JSON.stringify(samples))
    await page.evaluate(() => window.startShell())
    await page.getByRole('button', { name: /^Changes/ }).click()
    await page.getByRole('dialog', { name: 'Recent changes' }).waitFor()
    const shellHost = await page.evaluate(() => document.querySelector('[data-app-overlay-host]').contains(document.querySelector('[data-changelog-popover]')))
    if (!shellHost) throw new Error('Status-bar changelog escaped registered app host')
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    const beforeNoop = await page.evaluate(() => window.shellSamples.length)
    if (beforeNoop < 1) throw new Error('Status-bar real opening produced no sample')
    await page.getByRole('heading', { name: \"What's new\" }).click()
    await page.waitForFunction(() => window.shellTimerDone)
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    if (await page.evaluate(() => window.shellSamples.length) !== beforeNoop) throw new Error('Status-bar no-op attributed a timer commit')
    await page.evaluate(() => window.stopCapture())
    console.log(JSON.stringify({ status: 'pass', idleFrames: 8, rafIngestBeforeAndAfterCaptureExcluded: true, timerCommitsExcluded: true, stoppedReactCommitPainted: true, externalPortalBubbleAndCapture: true, realShellChangelogHostAndTimerExclusion: true, samples }))
    return { status: 'pass', idleFrames: 8, rafIngestBeforeAndAfterCaptureExcluded: true, timerCommitsExcluded: true, stoppedReactCommitPainted: true, externalPortalBubbleAndCapture: true, realShellChangelogHostAndTimerExclusion: true, samples }
  }`])
} finally {
  try { await run(['close']) } finally { await vite.close() }
}
