import assert from 'node:assert/strict'
import { performance } from 'node:perf_hooks'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'

// Run in the web Nix shell: its Playwright driver and Chromium are also used by the kit gate.
const started = performance.now()
const app = fileURLToPath(new URL('..', import.meta.url))
const scenario = 'first-run-onboarding'
const sentence = 'The first mission is running. Checking sprout now.'
const epoch = new Date('2026-10-01T12:00:00.000Z')
const violations = []
const errors = []
let server, browser
let negativePassed = false
const close = async () => {
  await browser?.close()
  await server?.close()
}
const deadline = setTimeout(() => {
  console.error('Replay browser e2e exceeded its 90-second deadline')
  process.exitCode = 1
  void close()
}, 90_000)
deadline.unref()

try {
  // Replay must work with no live gateway or bearer, even in a developer's configured shell.
  for (const key of ['WF_ST_GATEWAY', 'WF_ST_AUTHORIZATION', 'WF_ST_GATEWAY_HOST']) delete process.env[key]
  process.chdir(app)
  server = await createServer({
    configFile: `${app}/vite.config.ts`,
    configLoader: 'runner',
    server: { host: '127.0.0.1', port: 0, open: false },
  })
  await server.listen()
  const address = server.httpServer.address()
  assert(address && typeof address === 'object')
  const origin = `http://127.0.0.1:${address.port}`
  const authority = new URL(origin).host
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
  browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})

  const context = async (negative = false) => {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, serviceWorkers: 'block' })
    context.on('request', request => {
      const url = new URL(request.url())
      if (!['http:', 'https:'].includes(url.protocol)) return
      if (url.origin !== origin || /^\/v1(?:\/|$)/.test(url.pathname)) violations.push(`request ${request.method()} ${url}`)
    })
    // The constructor binding records both authority and subprotocol before any app module runs.
    // Retain native construction/prototype behavior; the observer never supplies replay data.
    await context.exposeBinding('recordReplaySocket', (_source, url, protocols) => {
      const socket = new URL(url, origin)
      if (socket.protocol !== 'ws:' || socket.host !== authority || protocols.length !== 1 || protocols[0] !== 'vite-hmr') {
        violations.push(`WebSocket ${socket} protocols=${JSON.stringify(protocols)}`)
      }
    })
    await context.addInitScript(() => {
      const NativeWebSocket = window.WebSocket
      window.WebSocket = new Proxy(NativeWebSocket, {
        construct(target, args) {
          const [url, protocols] = args
          void window.recordReplaySocket(String(url), typeof protocols === 'string' ? [protocols] : protocols ?? [])
          return Reflect.construct(target, args)
        },
      })
    })
    const page = await context.newPage()
    page.setDefaultTimeout(10_000)
    page.on('pageerror', error => {
      if (negative && error.message.startsWith('Unknown Fractal replay scenario "__missing_replay_world__". Choose one of:') && error.message.includes(scenario)) {
        negativePassed = true
      } else errors.push(error.message)
    })
    page.on('console', message => {
      if (message.type() === 'error') errors.push(message.text())
    })
    assert(page.clock && typeof page.clock.install === 'function', 'The Nix Playwright driver must provide browser-clock control')
    await page.clock.install({ time: epoch })
    await page.clock.pauseAt(new Date(epoch.getTime() + 1000))
    return { context, page }
  }

  const positive = await context()
  const page = positive.page
  const roster = page.getByRole('navigation', { name: 'Agent roster', exact: true })
  const agent = roster.getByRole('row').filter({ has: page.locator('[data-wf-agent-ref]'), hasText: 'Sprout Builder' })
  const noFolders = async () => {
    // Only the adapter's Unfiled grouping row may lack an agent identity in this empty world.
    const groups = await roster.getByRole('row').filter({ hasNot: page.locator('[data-wf-agent-ref]') })
      .evaluateAll(rows => rows.map(row => row.getAttribute('data-sidebar-tree-key')))
    assert(groups.every(key => key === 'wf/unfiled'), `Unexpected folder rows: ${JSON.stringify(groups)}`)
  }
  // Onboarding creates its conversation at +5s. Keep it unopened until then, without a retry.
  await page.goto(`${origin}/?scenario=${scenario}&open=missions:detail`)
  // Open replay sockets and publish the initial RAF batch, still well before the +2s frame.
  await page.clock.runFor(100)
  await roster.waitFor({ state: 'visible' })
  assert.equal(await agent.count(), 0, 'before +2s: Sprout Builder must not exist')
  assert.equal(await page.getByText(sentence, { exact: true }).count(), 0, 'before timed conversation: sentence must not exist')
  await noFolders()
  await page.clock.runFor(2000)
  await page.clock.runFor(32) // Publish the SDK/React animation-frame batch after frame delivery.
  await agent.waitFor({ state: 'visible' })
  assert.equal(await agent.count(), 1, 'after +2s: replay inserts Sprout Builder')
  await noFolders()
  await page.clock.runFor(3000)
  await page.clock.runFor(32)
  await agent.click()
  assert(new URL(page.url()).pathname.startsWith('/w/agent/'), 'agent selection must navigate in-app')
  assert.equal(new URL(page.url()).searchParams.get('scenario'), scenario)
  assert.equal(await page.getByText(sentence, { exact: true }).count(), 0)
  await page.clock.runFor(2000)
  await page.clock.runFor(32)
  await page.getByTestId('transcript-scroll').getByText(sentence, { exact: true }).waitFor({ state: 'visible' })
  console.log('PASS timed replay: empty roster → Sprout Builder (+2s) → conversation sentence (+7s)')
  await page.reload()
  await page.clock.runFor(100)
  await roster.waitFor({ state: 'visible' })
  assert.equal(new URL(page.url()).searchParams.get('scenario'), scenario)
  assert.equal(await agent.count(), 0, 'reload restarts the replay clock')
  await page.clock.runFor(2000)
  await page.clock.runFor(32)
  await agent.waitFor({ state: 'visible' })
  await noFolders()
  console.log('PASS in-app navigation and reload preserve scenario; onboarding has no folders')
  await positive.context.close()

  const negative = await context(true)
  const catalogError = negative.page.waitForEvent('pageerror', {
    predicate: error => error.message.startsWith('Unknown Fractal replay scenario "__missing_replay_world__". Choose one of:'),
  })
  await negative.page.goto(`${origin}/?scenario=__missing_replay_world__`)
  await catalogError
  assert(negativePassed, 'unknown scenario must report the explicit catalog error with valid choices')
  assert.equal(await negative.page.getByTestId('live-agent-workspace').count(), 0)
  await negative.context.close()
  console.log('PASS unknown-world negative control')
  assert.deepEqual(violations, [], 'Replay network invariant: no real /v1/, non-local requests, or non-HMR WebSockets')
  assert.deepEqual(errors, [], 'Unexpected browser errors')
  console.log('PASS network: zero real /v1/ requests, zero non-local requests, only local vite-hmr WebSockets')
} catch (error) {
  for (const context of browser?.contexts() ?? []) for (const page of context.pages()) {
    console.error(`Replay failure DOM (${page.url()}): ${await page.locator('body').innerText()}`)
  }
  throw error
} finally {
  clearTimeout(deadline)
  await close()
  console.log(`REPLAY_E2E ${JSON.stringify({ seconds: Number(((performance.now() - started) / 1000).toFixed(2)), negativePassed, violations, errors })}`)
}
