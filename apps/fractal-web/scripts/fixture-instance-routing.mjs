import assert from 'node:assert/strict'
import { once } from 'node:events'
import { createServer } from 'node:http'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer as createViteServer } from 'vite'

// Exercise the real web app and its production gateway middleware, not fixtureSource.
// The two Unix HTTP servers are transport test doubles, not initialized daemon instances.
const app = fileURLToPath(new URL('..', import.meta.url))
const scratch = fileURLToPath(new URL('../../../tmp/fixture-instance-routing/', import.meta.url))
await mkdir(scratch, { recursive: true })
const root = await mkdtemp(join(scratch, 'routing-'))
const authorization = ['Bearer', 'fixture-routing-test'].join(' ')
const calls = { fixture: [], normal: [] }
const gateways = []
let web, browser
const saved = Object.fromEntries(['WF_ST_GATEWAY', 'WF_ST_AUTHORIZATION', 'WF_ST_GATEWAY_HOST'].map(key => [key, process.env[key]]))
const previousCwd = process.cwd()

async function gateway(kind) {
  const server = createServer((request, response) => {
    calls[kind].push({ path: request.url, authorization: request.headers.authorization })
    response.setHeader('content-type', 'application/json')
    if (kind === 'normal' || request.headers.authorization !== authorization) {
      response.writeHead(403)
      response.end(JSON.stringify({ error: 'fixture-credential-refused' }))
    } else {
      response.end(JSON.stringify({ instance: 'client-control-fixture', capabilities: [] }))
    }
  })
  gateways.push(server)
  const socket = join(root, `${kind}-client.sock`)
  server.listen(socket)
  await once(server, 'listening')
  return socket
}

async function serve(socket) {
  process.env.WF_ST_GATEWAY = `unix:${socket}`
  process.env.WF_ST_AUTHORIZATION = authorization
  delete process.env.WF_ST_GATEWAY_HOST
  web = await createViteServer({
    configFile: `${app}/vite.config.ts`, configLoader: 'runner',
    server: { host: '127.0.0.1', port: 0, open: false },
  })
  await web.listen()
  const address = web.httpServer.address()
  assert(address && typeof address === 'object')
  return `http://127.0.0.1:${address.port}`
}

try {
  const fixtureSocket = await gateway('fixture')
  const normalSocket = await gateway('normal')
  process.chdir(app)
  const origin = await serve(fixtureSocket)
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
  browser = await chromium.launch({ ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}) })
  // Fresh, nonpersistent context: never borrow a normal paired browser profile.
  const context = await browser.newContext()
  const page = await context.newPage()
  await page.goto(origin, { waitUntil: 'domcontentloaded' })
  assert.equal(new URL(page.url()).origin, origin)
  assert.equal(await page.locator('#root').count(), 1)
  const routed = await page.evaluate(async () => {
    const response = await fetch('/v1/client/capabilities')
    return { status: response.status, body: await response.json() }
  })
  assert.deepEqual(routed, { status: 200, body: { instance: 'client-control-fixture', capabilities: [] } })
  assert(calls.fixture.some(call => call.path === '/v1/client/capabilities' && call.authorization === authorization))
  assert.deepEqual(calls.normal, [], 'Fixture web origin must never reach the normal client service')
  const privileged = await page.evaluate(async () => (await fetch('/v1/status')).status)
  assert.equal(privileged, 404, 'The web boundary must not proxy privileged API routes')
  await context.close()
  await web.close()
  web = undefined

  // Negative control: the same fixture credential routed to the normal service is refused.
  const normalOrigin = await serve(normalSocket)
  const refusedContext = await browser.newContext()
  const refusedPage = await refusedContext.newPage()
  await refusedPage.goto(normalOrigin, { waitUntil: 'domcontentloaded' })
  const refused = await refusedPage.evaluate(async () => {
    const response = await fetch('/v1/client/capabilities')
    return { status: response.status, body: await response.json() }
  })
  assert.deepEqual(refused, { status: 403, body: { error: 'fixture-credential-refused' } })
  assert(calls.normal.some(call => call.path === '/v1/client/capabilities' && call.authorization === authorization))
  await refusedContext.close()
  console.log(JSON.stringify({ check: 'fixture-instance-real-web-routing', fixtureOrigin: origin, normalOrigin, fixtureRouted: true, normalRefused: true, privilegedRouteExcluded: true, transport: 'unix-test-double', embeddedFixtureSource: false }))
} finally {
  await browser?.close()
  await web?.close()
  for (const gateway of gateways) {
    gateway.closeAllConnections()
    await new Promise(resolve => gateway.close(resolve))
  }
  for (const [key, value] of Object.entries(saved)) {
    if (value === undefined) delete process.env[key]
    else process.env[key] = value
  }
  process.chdir(previousCwd)
  await rm(root, { recursive: true, force: true })
}
