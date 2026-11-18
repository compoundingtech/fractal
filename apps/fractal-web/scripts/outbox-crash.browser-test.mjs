import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { EventEmitter, once } from 'node:events'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'

// Real Chromium, IndexedDB, HTTP, senders and recovery UI; FAKE canonical protocol
// fixture server. A completed fixture receipt is NOT proof of a real daemon effect.
// Browser producers are unsigned: this proves nonce/signature ABSENCE is retained,
// not signing correctness. There is no action-outcome read (#6708), so even a valid
// receipt lost before local commit must restore unknown, without another POST.
// Intent/packet/dispatch-marker seams park fixture dependency wrappers AFTER real
// IndexedDB commits. Socket parks the real Node HTTP response after body receipt.
// Receipt parks a fixture repository wrapper AFTER the real HTTP completed result
// was decoded/correlated, BEFORE the real local outcome transaction. All kills
// close the page; no production module exposes any test seam.
// Run with the same pinned Node/Playwright/Vite environment as replay-e2e.mjs.
const app = fileURLToPath(new URL('..', import.meta.url))
const fixture = JSON.parse(await readFile(new URL('../src/data/subjectReadPort.gateway.fixtures.json', import.meta.url), 'utf8'))
const posts = []
const gets = []
const errors = []
const received = new EventEmitter()
const held = new Set()
const sockets = new Set()
const modes = new Map()
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
const digest = async bytes => new Uint8Array(createHash('sha256').update(bytes).digest())
let server, browser, Native, identities, origin, closing
const close = () => closing ??= (async () => {
  // Release held HTTP sockets even if the browser or a test failed mid-request.
  for (const response of held) response.destroy()
  for (const socket of sockets) socket.destroy()
  try { await browser?.close() } finally { await server?.close() }
})()
const interrupt = signal => {
  process.exitCode = signal === 'SIGINT' ? 130 : 143
  void close()
}
const onSigint = () => interrupt('SIGINT')
const onSigterm = () => interrupt('SIGTERM')
process.once('SIGINT', onSigint)
process.once('SIGTERM', onSigterm)
const deadline = setTimeout(() => {
  console.error('Outbox crash browser tests exceeded the 180-second deadline')
  process.exitCode = 1
  // A second finite deadline also bounds a stuck Chromium/Vite shutdown.
  setTimeout(() => process.exit(1), 5_000).unref()
  void close()
}, 180_000)
deadline.unref()

const base = { ...fixture.capabilities, value: { ...fixture.capabilities.value,
  issuer: 'person/example', issuer_key: 'issuer-key/crash-browser', transport: 'local-web',
  capabilities: [...fixture.capabilities.value.capabilities,
    { id: 'thread.start', version: 1, state: 'granted' },
    { id: 'thread.reply', version: 1, state: 'granted' }],
} }
const actor = base.value.session_actor
const startContext = { ...base, value: { kind: 'thread-start-context', source_id: 'source/crash-browser',
  episode: 'episode/crash-browser', start_recipient: { agent_id: fixture.cases.agent.ref, basis: { _tag: 'original' } } } }
const agent = { request_id: base.request_id, ...fixture.cases.agent.payload }
const checked = (envelope, schema) => {
  // Envelope's generated closed value union does not cover every typed GET.
  // Reuse its canonical transport fields with the endpoint's generated codec.
  for (const key of ['api_version', 'request_id', 'snapshot']) {
    Native.decodeUnknownSync(Native.Envelope.fields[key])(envelope[key])
  }
  Native.decodeUnknownSync(schema)(envelope.value)
  return envelope
}
const json = (response, value) => {
  response.writeHead(200, { 'content-type': 'application/json' })
  response.end(JSON.stringify(value))
}
const middleware = async (request, response, next) => {
  const url = new URL(request.url, 'http://fixture.invalid')
  if (url.pathname === '/__outbox-crash') {
    // Leave headers writable so the app CSP plugin hashes the transformed preamble.
    response.setHeader('content-type', 'text/html')
    response.end(await server.transformIndexHtml(request.url, '<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module" src="/src/data/outbox/crash.browser.tsx"></script></body></html>'))
    return
  }
  if (!url.pathname.startsWith('/v1/client/')) { next(); return }
  try {
    if (request.method === 'GET') {
      gets.push(url.pathname + url.search)
      if (url.pathname === '/v1/client/capabilities') return json(response, checked(base, Native.Capabilities))
      if (url.pathname === `/v1/client/agents/${encodeURIComponent(fixture.cases.agent.ref)}`) return json(response, checked(agent, Native.Agent))
      if (url.pathname === '/v1/client/threads/start-context') {
        assert.equal(url.searchParams.get('source_id'), startContext.value.source_id)
        assert.equal(url.searchParams.get('episode'), startContext.value.episode)
        return json(response, checked(startContext, Native.ThreadStartContext))
      }
    }
    if (request.method === 'POST' && url.pathname === '/v1/client/actions') {
      const chunks = []
      for await (const chunk of request) chunks.push(chunk)
      const bytes = Buffer.concat(chunks)
      const packet = JSON.parse(bytes.toString('utf8'))
      Native.decodeUnknownSync(Native.ActionRequest)(packet)
      assert(['message.send', 'thread.start'].includes(packet.type))
      const expected = await identities(packet, actor, digest)
      const receipt = checked({ ...base, value: { kind: 'action-result', status: 'completed', action_id: packet.id,
        operation_id: `operation/${sha256(bytes).slice(0, 16)}`, snapshot_id: base.snapshot.id,
        affected_ids: [...(packet.type === 'thread.start' ? [packet.parameters.target_id] : []), ...expected],
      } }, Native.ActionResult)
      const post = { packet, bytes: [...bytes], fingerprint: sha256(bytes), receipt, responded: false }
      posts.push(post)
      received.emit('post')
      if (modes.get(packet.idempotency_key) === 'socket') {
        held.add(response)
        response.once('close', () => held.delete(response))
        return // Real HTTP request fully received; intentionally no response before page destruction.
      }
      json(response, receipt)
      post.responded = true
      return
    }
    throw new Error(`Unexpected fixture request: ${request.method} ${url.pathname}`)
  } catch (error) {
    errors.push(error)
    response.writeHead(500, { 'content-type': 'text/plain' })
    response.end(String(error))
  }
}
const count = key => posts.filter(post => post.packet.idempotency_key === key)
const waitForPost = async key => {
  while (count(key).length === 0) await once(received, 'post', { signal: AbortSignal.timeout(15_000) })
  return count(key)[0]
}
const snapshot = page => page.evaluate(() => window.outboxCrash.snapshot())
const phase = async (page, expected) => {
  await page.waitForFunction(expected => window.outboxCrash.phase === expected || window.outboxCrash.phase === 'error', expected)
  assert.equal(await page.evaluate(() => window.outboxCrash.error), undefined)
  assert.equal(await page.evaluate(() => window.outboxCrash.phase), expected)
}
const settled = async page => {
  await page.waitForFunction(() => window.outboxCrash.done || window.outboxCrash.phase === 'error')
  assert.equal(await page.evaluate(() => window.outboxCrash.error), undefined)
  await page.waitForLoadState('networkidle')
}
const unsigned = packet => {
  for (const object of [packet, packet.parameters]) {
    assert.equal(Object.hasOwn(object, 'nonce'), false)
    assert.equal(Object.hasOwn(object, 'signature'), false)
  }
}
const packetIdentity = (before, after, key) => {
  assert.deepEqual(after.intentBytes, before.intentBytes, 'Original serialized intent must survive page destruction')
  assert.deepEqual(after.record.intent, before.record.intent)
  assert.equal(after.record.id, key)
  assert.deepEqual(after.storedEnvelope.scope, before.storedEnvelope.scope)
  assert.equal(after.storedEnvelope.localAttemptId, before.storedEnvelope.localAttemptId)
  assert.deepEqual(after.storedEnvelope.intentBytes, before.storedEnvelope.intentBytes)
  assert.deepEqual(after.storedEnvelope.packetBytes, before.storedEnvelope.packetBytes)
  assert.deepEqual(after.storedEnvelope.fingerprint, before.storedEnvelope.fingerprint)
  if (before.packetBytes === undefined) {
    assert.equal(after.packetBytes, undefined)
    return
  }
  assert.deepEqual(after.record.packet, before.record.packet, 'Every original packet field must survive')
  assert.deepEqual(after.packetBytes, before.packetBytes, 'Stored wire bytes must be identical, not reconstructed')
  assert.equal(after.fingerprint, before.fingerprint)
  assert.equal(after.fingerprint, sha256(Buffer.from(before.packetBytes)))
  assert.deepEqual(JSON.parse(Buffer.from(before.packetBytes).toString('utf8')), before.record.packet)
  assert.equal(after.record.packet.idempotency_key, key)
  assert.deepEqual(after.record.expectedIdentities, before.record.expectedIdentities)
  unsigned(before.record.packet)
  unsigned(after.record.packet)
}
const delivered = async (page, key) => {
  try { await recoveryRow(page, key).waitFor({ state: 'hidden' }) }
  catch (cause) {
    throw new Error(`Recovery did not deliver ${key}: ${await page.locator('body').textContent()}; GETs=${JSON.stringify(gets)}; fixture errors=${JSON.stringify(errors.map(String))}`, { cause })
  }
  assert.equal((await snapshot(page)).record?.state, 'delivered')
}
// Attribute keys contain only generated ASCII identifiers; use exact DOM matching without CSS escaping.
const recoveryRow = (page, key) => page.locator(`[data-outbox-item="${key}"]`)
let sequence = 0
const testContext = async () => {
  const context = await browser.newContext({ serviceWorkers: 'block' })
  const networkPosts = []
  context.on('request', request => {
    const url = new URL(request.url())
    if (!['http:', 'https:'].includes(url.protocol)) return
    if (url.origin !== origin) errors.push(new Error(`Non-fixture request ${request.method()} ${url}`))
    if (request.method() === 'POST') {
      assert.equal(url.pathname, '/v1/client/actions')
      networkPosts.push(JSON.parse(request.postData()))
    }
  })
  return { context, networkPosts }
}
const open = async (context, params) => {
  const page = await context.newPage()
  page.setDefaultTimeout(15_000)
  page.on('pageerror', error => errors.push(error))
  await page.goto(`${origin}/__outbox-crash?${new URLSearchParams(params)}`)
  await page.waitForFunction(() => window.outboxCrash !== undefined)
  await page.evaluate(() => window.outboxCrash.ready)
  await page.waitForLoadState('networkidle')
  return page
}
const start = page => page.evaluate(() => window.outboxCrash.start())
const cases = [
  ['never_dispatched_restore_pending_explicit_revalidation', 'intent-commit', 'pending'],
  ['after_packet_commit_preserves_exact_packet', 'packet-commit', 'pending'],
  ['after_dispatch_marker_before_socket_is_unknown', 'dispatch-marker', 'unknown'],
  ['after_socket_before_commit_is_unknown', 'socket', 'unknown'],
  ['after_receipt_before_local_commit', 'receipt', 'unknown'],
]

try {
  for (const key of ['WF_ST_GATEWAY', 'WF_ST_AUTHORIZATION', 'WF_ST_GATEWAY_HOST']) delete process.env[key]
  process.chdir(app)
  server = await createServer({ root: app, configFile: `${app}/vite.config.ts`, configLoader: 'runner',
    server: { host: '127.0.0.1', port: 0, open: false },
    plugins: [{ name: 'outbox-crash-canonical-fixture', configureServer(vite) {
      vite.middlewares.use((request, response, next) => {
        void middleware(request, response, next).catch(error => { errors.push(error); next(error) })
      })
    } }],
  })
  server.httpServer.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)) })
  Native = await server.ssrLoadModule('@fractal/client/schema')
  ;({ expectedPreparedPacketIdentities: identities } = await server.ssrLoadModule('@fractal/client/prepared-packet'))
  checked(base, Native.Capabilities)
  checked(agent, Native.Agent)
  checked(startContext, Native.ThreadStartContext)
  await server.listen()
  const address = server.httpServer.address()
  assert(address && typeof address === 'object')
  origin = `http://127.0.0.1:${address.port}`
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
  browser = await chromium.launch({ ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}) })
  for (const flow of ['chat', 'thread.start']) {
    for (const [name, seam, restoredState] of cases) {
      const key = `attempt/browser-crash-${++sequence}`
      const params = { flow, seam, key, database: `outbox-crash-${sequence}` }
      const { context, networkPosts } = await testContext()
      try {
        modes.set(key, seam)
        let page = await open(context, params)
        await start(page)
        let post
        if (seam === 'socket') post = await waitForPost(key)
        else await phase(page, seam)
        const before = await snapshot(page)
        assert.equal(before.record.state, 'pending')
        assert.equal(before.record.mayHaveDispatched, restoredState === 'unknown')
        if (seam === 'intent-commit') assert.equal(before.record.phase, 'preparing')
        if (seam === 'packet-commit') assert.equal(before.record.phase, 'ready')
        const expectedPosts = ['socket', 'receipt'].includes(seam) ? 1 : 0
        assert.equal(count(key).length, expectedPosts)
        assert.equal(networkPosts.length, expectedPosts)
        if (seam === 'receipt') {
          post = count(key)[0]
          assert.equal(post.responded, true, 'Actual canonical completed HTTP receipt must reach sender before parking')
          assert.equal(post.receipt.value.status, 'completed')
          assert.equal(post.receipt.value.action_id, before.record.packet.id)
        }
        if (post) {
          assert.deepEqual(post.bytes, before.packetBytes)
          assert.equal(post.fingerprint, before.fingerprint)
          assert.deepEqual(post.packet, before.record.packet)
          assert.deepEqual(await identities(post.packet, actor, digest), before.record.expectedIdentities)
        }
        await page.close() // Destroy the sender closure; preserve real IndexedDB in this BrowserContext.
        page = await open(context, { ...params, seam: 'none' })
        const after = await snapshot(page)
        assert.equal(after.record.state, restoredState)
        packetIdentity(before, after, key)
        const row = recoveryRow(page, key)
        await row.waitFor({ state: 'visible' })
        assert.equal(await row.getAttribute('data-outbox-kind'), restoredState)
        assert.equal(count(key).length, expectedPosts, 'Startup must never POST a restored attempt')
        assert.equal(networkPosts.length, expectedPosts, 'Browser network must agree with HTTP server')
        if (restoredState === 'unknown') {
          assert.match(await row.locator('[data-outbox-headline]').textContent(), /has not confirmed.*Do not send it again/)
          assert.equal(await row.locator('[data-outbox-action="send"], [data-outbox-action="retry"]').count(), 0)
        } else {
          assert.match(await row.locator('[data-outbox-headline]').textContent(), /has not been sent/)
          const readsBefore = gets.length
          modes.set(key, 'none')
          await row.locator('[data-outbox-action="send"]').click()
          await delivered(page, key)
          await page.waitForLoadState('networkidle')
          assert.equal(count(key).length, 1, `Recovery ${key}: observed POST keys ${JSON.stringify(posts.map(post => post.packet.idempotency_key))}`)
          assert.equal(networkPosts.length, 1)
          const revalidated = gets.slice(readsBefore)
          assert(revalidated.some(path => path.startsWith(flow === 'chat' ? '/v1/client/agents/' : '/v1/client/threads/start-context')),
            'Explicit recovery send must revalidate the original target through real HTTP')
          const sent = await snapshot(page)
          if (before.packetBytes !== undefined) packetIdentity(before, sent, key)
          assert.equal(sent.record.state, 'delivered')
          assert.equal(sent.record.evidence.kind, 'receipt')
          assert.deepEqual(count(key)[0].bytes, sent.packetBytes)
          unsigned(sent.record.packet)
        }
        assert.deepEqual(errors, [])
        console.log(`PASS ${flow} ${name}${seam === 'receipt' ? ' (unknown: outcome-read unavailable #6708)' : ''}`)
      } finally { await context.close() }
    }
    const key = `attempt/browser-crash-${++sequence}`
    const params = { flow, seam: 'none', key, database: `outbox-crash-${sequence}` }
    const { context, networkPosts } = await testContext()
    try {
      // Both owners finish startup before either accepts intent: recovery cannot
      // mistake a sibling's live dispatch for a crashed previous session.
      const pages = await Promise.all([open(context, params), open(context, params)])
      await Promise.all(pages.map(start))
      await Promise.all(pages.map(settled))
      const records = await Promise.all(pages.map(snapshot))
      assert.equal(count(key).length, 1, 'Actual IndexedDB dispatch permit must admit only one tab')
      assert.equal(networkPosts.length, 1)
      assert.equal(records[0].record.state, 'delivered')
      packetIdentity(records[0], records[1], key)
      assert.deepEqual(records[0].packetBytes, count(key)[0].bytes)
      assert.deepEqual(errors, [])
      console.log(`PASS ${flow} two_tabs_one_dispatch`)
    } finally { await context.close() }
  }
  console.log('PASS outbox crash browser matrix: real browser/storage/HTTP; fake canonical protocol fixture, not real daemon effect proof; unsigned packet identity only; receipt-gap remains unknown (#6708).')
} catch (error) {
  process.exitCode = 1
  console.error(error)
  for (const fixtureError of errors) console.error(fixtureError)
} finally {
  clearTimeout(deadline)
  process.off('SIGINT', onSigint)
  process.off('SIGTERM', onSigterm)
  const shutdownDeadline = setTimeout(() => process.exit(1), 5_000)
  try { await close() } finally { clearTimeout(shutdownDeadline) }
}
