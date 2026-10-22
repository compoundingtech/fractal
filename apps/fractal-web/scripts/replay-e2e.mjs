import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { once } from 'node:events'
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
let server, browser, devtools
let negativePassed = false
let closing
const close = () => closing ??= (async () => {
  try { await browser?.close() } finally {
    devtools?.close()
    await server?.close()
  }
})()
const interrupt = (signal) => {
  console.error(`Replay browser e2e interrupted by ${signal}`)
  void close().finally(() => process.exit(signal === 'SIGINT' ? 130 : 143))
}
const onSigint = () => interrupt('SIGINT')
const onSigterm = () => interrupt('SIGTERM')
process.once('SIGINT', onSigint)
process.once('SIGTERM', onSigterm)
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
  // A separate loopback CDP connection gives us flattened worker sessions, which the
  // public Playwright CDPSession.send API cannot address with a sessionId.
  browser = await chromium.launch({
    ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}),
    args: ['--remote-debugging-port=0', '--enable-automation'],
  })
  const browserSession = await browser.newBrowserCDPSession()
  const { arguments: arguments_ } = await browserSession.send('Browser.getBrowserCommandLine')
  const profile = arguments_.find(argument => argument.startsWith('--user-data-dir='))?.slice('--user-data-dir='.length)
  assert(profile, 'Chromium must report its temporary profile for the loopback CDP endpoint')
  const [port, socketPath] = (await readFile(`${profile}/DevToolsActivePort`, 'utf8')).trim().split('\n')
  await browserSession.detach()
  devtools = new WebSocket(`ws://127.0.0.1:${port}${socketPath}`)
  await once(devtools, 'open')
  let nextCommand = 0
  const commands = new Map()
  const sockets = new Map()
  const attaching = new Set()
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++nextCommand
    commands.set(id, { resolve, reject })
    devtools.send(JSON.stringify({ id, method, params, ...(sessionId === undefined ? {} : { sessionId }) }))
  })
  const autoAttach = { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }
  devtools.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data)
    if (message.id !== undefined) {
      const command = commands.get(message.id)
      commands.delete(message.id)
      if (message.error) command?.reject(new Error(JSON.stringify(message.error)))
      else command?.resolve(message.result)
      return
    }
    const { method, params, sessionId } = message
    if (method === 'Target.attachedToTarget') {
      // Auto-attachment is recursive: page targets own dedicated/shared worker targets.
      // Every target stays paused until Network observation is installed.
      const setup = (async () => {
        await send('Network.enable', {}, params.sessionId)
        await send('Target.setAutoAttach', autoAttach, params.sessionId)
        await send('Runtime.runIfWaitingForDebugger', {}, params.sessionId)
      })().catch(error => errors.push(`CDP target observation: ${error.message}`))
      attaching.add(setup)
      void setup.finally(() => attaching.delete(setup))
    } else if (method === 'Network.webSocketCreated') {
      const url = new URL(params.url)
      sockets.set(`${sessionId}:${params.requestId}`, { url: params.url, verified: false })
      if (url.protocol !== 'ws:' || url.host !== authority || /^\/v1(?:\/|$)/.test(url.pathname)) {
        violations.push(`WebSocket ${url}`)
      }
    } else if (method === 'Network.webSocketWillSendHandshakeRequest') {
      const socket = sockets.get(`${sessionId}:${params.requestId}`)
      if (socket === undefined) {
        errors.push('CDP observed a WebSocket handshake without its creation event')
        return
      }
      socket.verified = true
      const url = new URL(socket.url)
      const protocol = Object.entries(params.request.headers).find(([key]) => key.toLowerCase() === 'sec-websocket-protocol')?.[1]
      if (url.protocol !== 'ws:' || url.host !== authority || protocol !== 'vite-hmr') {
        violations.push(`WebSocket ${url} protocol=${JSON.stringify(protocol)}`)
      }
    }
  })
  await send('Target.setAutoAttach', autoAttach)

  const context = async (negative = false) => {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, serviceWorkers: 'block' })
    context.on('request', request => {
      const url = new URL(request.url())
      if (!['http:', 'https:'].includes(url.protocol)) return
      if (url.origin !== origin || /^\/v1(?:\/|$)/.test(url.pathname)) violations.push(`request ${request.method()} ${url}`)
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
  const noUserFolders = async () => {
    // Project and Unfiled groups are generated views, not user-created folders.
    // User folders keep their arrangement IDs; they have no separate row-key prefix.
    const folders = await roster.getByRole('row').filter({ hasNot: page.locator('[data-wf-agent-ref]') })
      .evaluateAll(rows => rows.map(row => row.getAttribute('data-sidebar-tree-key'))
        .filter(key => key !== 'wf/unfiled' && !key?.startsWith('wf/project/')))
    assert.deepEqual(folders, [], `Unexpected user folder rows: ${JSON.stringify(folders)}`)
  }
  const agentInProject = async () => {
    const projectKey = `wf/project/${encodeURIComponent('~/src/sprout')}`
    const project = roster.locator(`[data-sidebar-tree-key="${projectKey}"]`)
    await project.getByText('~/src/sprout', { exact: true }).waitFor({ state: 'visible' })
    const parentKey = await agent.evaluate(row => {
      const level = Number(row.getAttribute('aria-level'))
      const rows = [...row.closest('[role="treegrid"]').querySelectorAll('[role="row"]')]
      for (let index = rows.indexOf(row) - 1; index >= 0; index--) {
        if (Number(rows[index].getAttribute('aria-level')) < level) return rows[index].getAttribute('data-sidebar-tree-key')
      }
      return null
    })
    assert.equal(parentKey, projectKey, 'Sprout Builder must appear under its project group')
  }
  // Onboarding creates its conversation at +5s. Keep it unopened until then, without a retry.
  await page.goto(`${origin}/?scenario=${scenario}&open=missions:detail`)
  // Open replay sockets and publish the initial RAF batch, still well before the +2s frame.
  await page.clock.runFor(100)
  await roster.waitFor({ state: 'visible' })
  assert.equal(await agent.count(), 0, 'before +2s: Sprout Builder must not exist')
  assert.equal(await page.getByText(sentence, { exact: true }).count(), 0, 'before timed conversation: sentence must not exist')
  await noUserFolders()
  await page.clock.runFor(2000)
  await page.clock.runFor(32) // Publish the SDK/React animation-frame batch after frame delivery.
  await agent.waitFor({ state: 'visible' })
  assert.equal(await agent.count(), 1, 'after +2s: replay inserts Sprout Builder')
  await noUserFolders()
  await agentInProject()
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
  await noUserFolders()
  await agentInProject()
  console.log('PASS in-app navigation and reload preserve scenario; onboarding has no user folders and groups its agent by project')
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
  await send('Browser.getVersion') // Drain protocol events after the contexts and their workers close.
  await Promise.all(attaching)
  for (const socket of sockets.values()) if (!socket.verified) violations.push(`WebSocket without an observed HMR handshake: ${socket.url}`)
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
  process.removeListener('SIGINT', onSigint)
  process.removeListener('SIGTERM', onSigterm)
  await close()
  console.log(`REPLAY_E2E ${JSON.stringify({ seconds: Number(((performance.now() - started) / 1000).toFixed(2)), negativePassed, violations, errors })}`)
}
