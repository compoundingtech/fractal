import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { createIsolatedKitWorker } from './kit-play-isolation.mjs'
import { strictStoryGuard } from './details-strict-story-guard.mjs'

const pageOptions = { viewport: { width: 440, height: 900 }, colorScheme: 'dark' }
const storyDocument = failed => `<!doctype html><meta charset="utf-8"><title>Story fixture</title>
<script>
  const listeners = new Map()
  window.__STORYBOOK_ADDONS_CHANNEL__ = {
    on(name, listener) {
      listeners.set(name, listener)
      // Emit only after the real strict guard has subscribed to all lifecycle
      // events. No wall-clock sleep or replacement for the guard is involved.
      if (name === 'storyErrored') queueMicrotask(() => {
        if (${failed}) listeners.get('playFunctionThrewException')({ message: 'deliberately broken play' })
        listeners.get('storyFinished')({ storyId: '${failed ? 'kit--broken' : 'kit--success'}', status: '${failed ? 'error' : 'success'}', reporters: [] })
      })
    },
  }
</script>`

const seedState = async page => page.evaluate(async () => {
  window.__previousKitCase = 'must not survive'
  localStorage.setItem('previous-case', 'local')
  sessionStorage.setItem('previous-case', 'session')
  document.cookie = 'previous-case=cookie; path=/; SameSite=Lax'
  await new Promise((resolve, reject) => {
    const request = indexedDB.open('previous-case', 1)
    request.onupgradeneeded = () => request.result.createObjectStore('values')
    request.onerror = () => reject(request.error)
    request.onsuccess = () => {
      const database = request.result
      const transaction = database.transaction('values', 'readwrite')
      transaction.objectStore('values').put('persisted', 'key')
      transaction.oncomplete = () => { database.close(); resolve() }
      transaction.onabort = () => { database.close(); reject(transaction.error) }
      transaction.onerror = () => { database.close(); reject(transaction.error) }
    }
  })
  const cache = await caches.open('previous-case')
  await cache.put('/previous-case', new Response('persisted'))
})

const readState = async page => page.evaluate(async () => ({
  global: window.__previousKitCase ?? null,
  local: localStorage.getItem('previous-case'),
  session: sessionStorage.getItem('previous-case'),
  cookie: document.cookie,
  databases: (await indexedDB.databases()).map(database => database.name),
  caches: await caches.keys(),
}))
const emptyState = { global: null, local: null, session: null, cookie: '', databases: [], caches: [] }

test('reused Chromium worker isolates cases and rejects contamination and broken plays', { timeout: 30000 }, async t => {
  const servers = []
  let browser, worker, workerB, assetRequests = 0, retainWebStorage = false
  t.after(async () => {
    // Run every cleanup even when another cleanup fails; node:test preserves
    // these hook errors alongside the original assertion or protocol failure.
    const results = await Promise.allSettled([
      (async () => { await worker?.close() })(),
      (async () => { await workerB?.close() })(),
      ...servers.map(server => new Promise((resolve, reject) => {
        server.close(error => error ? reject(error) : resolve())
        server.closeAllConnections()
      })),
    ])
    try {
      await browser?.close()
    } catch (error) {
      results.push({ status: 'rejected', reason: error })
    }
    const failures = results.filter(result => result.status === 'rejected')
    if (failures.length) throw new AggregateError(failures.map(result => result.reason), 'Browser isolation test cleanup failed')
  })
  const serve = async () => {
    const server = createServer((request, response) => {
      const { pathname } = new URL(request.url, 'http://localhost')
      if (pathname === '/asset') {
        assetRequests++
        response.writeHead(200, { 'Content-Type': 'text/plain', 'Cache-Control': 'public, max-age=3600' }).end('immutable asset')
      } else if (pathname === '/service-worker.js') {
        response.writeHead(200, { 'Content-Type': 'text/javascript', 'Cache-Control': 'no-store' }).end(`
          self.addEventListener('activate', event => event.waitUntil(self.clients.claim()))
          self.addEventListener('fetch', event => {
            if (new URL(event.request.url).pathname === '/sw-proof') {
              event.respondWith(Promise.resolve(new Response('controlled by previous case')))
            }
            if (new URL(event.request.url).pathname === '/audit-attack') {
              event.respondWith(Promise.resolve(new Response('<!doctype html><script>window.__auditAttack = true; setInterval(() => localStorage.setItem("audit-attack", "survived"), 0)</script>', { headers: { 'Content-Type': 'text/html' } })))
            }
          })
          self.addEventListener('message', event => {
            if (event.data?.type !== 'navigate-audit') return
            event.waitUntil((async () => {
              const audit = (await self.clients.matchAll({ type: 'window' })).find(client => client.id !== event.source.id && client.url === event.data.from)
              await audit.navigate(event.data.url)
              event.ports[0].postMessage('navigated')
            })())
          })
        `)
      } else {
        response.writeHead(200, { 'Content-Type': 'text/html' }).end(pathname.endsWith('/iframe.html') ? storyDocument(pathname !== '/success/iframe.html') : '<!doctype html><title>Isolation fixture</title>')
      }
    })
    servers.push(server)
    await new Promise((resolve, reject) => {
      server.once('error', reject)
      server.listen(0, '127.0.0.1', resolve)
    })
    return `http://127.0.0.1:${server.address().port}`
  }
  const base = await serve(), otherOrigin = await serve()
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
  browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
  // Fault injection affects only CDP clearing, not storage reads or the real
  // browser lifecycle. It proves a failed reset is caught before the next case.
  const instrumentedBrowser = {
    async newContext(options) {
      const context = await browser.newContext(options)
      const newCDPSession = context.newCDPSession.bind(context)
      context.newCDPSession = async page => {
        const client = await newCDPSession(page)
        const send = client.send.bind(client)
        client.send = (method, params) => send(method,
          retainWebStorage && method === 'Storage.clearDataForOrigin'
            ? { ...params, storageTypes: 'indexeddb,cache_storage,service_workers' }
            : params)
        return client
      }
      return context
    },
  }
  worker = await createIsolatedKitWorker(instrumentedBrowser, base, { workerId: 7 })
  await worker.reset()
  const control = worker.context.pages()[0]
  const baseAudit = worker.context.pages().find(page => page.url() === `${base}/`)
  assert.ok(baseAudit, 'reset creates an inert origin audit document')
  await worker.reset()
  assert.ok(worker.context.pages().includes(baseAudit), 'an unchanged inert target is reused')
  const previous = await worker.newPage(pageOptions)
  await previous.goto(base)
  await seedState(previous)
  assert.deepEqual(await readState(previous), {
    global: 'must not survive', local: 'local', session: 'session', cookie: 'previous-case=cookie',
    databases: ['previous-case'], caches: ['previous-case'],
  })
  assert.equal(await previous.evaluate(() => fetch('/asset').then(response => response.text())), 'immutable asset')
  assert.equal(assetRequests, 1)
  await previous.evaluate(async () => {
    const controlled = new Promise(resolve => {
      if (navigator.serviceWorker.controller) resolve()
      else navigator.serviceWorker.addEventListener('controllerchange', resolve, { once: true })
    })
    await navigator.serviceWorker.register('/service-worker.js')
    await navigator.serviceWorker.ready
    await controlled
  })
  assert.equal(await previous.evaluate(() => fetch('/sw-proof').then(response => response.text())), 'controlled by previous case')
  assert.equal(await baseAudit.evaluate(() => navigator.serviceWorker.controller !== null), true, 'case service worker claims the retained audit client')
  assert.match(await previous.evaluate(() => fetch('/audit-attack').then(response => response.text())), /window.__auditAttack/, 'negative control: service worker supplies executable attack HTML')
  const navigateAudit = async url => previous.evaluate(({ url, from }) => new Promise(resolve => {
    const channel = new MessageChannel()
    channel.port1.onmessage = () => { channel.port1.close(); resolve() }
    navigator.serviceWorker.controller.postMessage({ type: 'navigate-audit', url, from }, [channel.port2])
  }), { url, from: baseAudit.url() })
  await navigateAudit(`${base}/audit-attack`)
  assert.equal(new URL(baseAudit.url()).pathname, '/audit-attack', 'service worker navigated the retained audit client')
  assert.equal(await baseAudit.evaluate(() => window.__auditAttack ?? false), false, 'SW-served document scripts cannot run on audit targets')
  const attackBlobURL = await previous.evaluate(() => URL.createObjectURL(new Blob([
    '<!doctype html><script>window.__auditBlobAttack = true; setInterval(() => localStorage.setItem("audit-blob-attack", "survived"), 0)</script>',
  ], { type: 'text/html' })))
  await navigateAudit(attackBlobURL)
  assert.equal(baseAudit.url(), attackBlobURL, 'case-made same-origin blob document reached the audit target')
  assert.equal(await baseAudit.evaluate(() => window.__auditBlobAttack ?? false), false, 'blob document scripts cannot run on audit targets')
  await worker.context.grantPermissions(['geolocation'], { origin: base })
  assert.equal(await previous.evaluate(async () => (await navigator.permissions.query({ name: 'geolocation' })).state), 'granted')
  const popupEvent = previous.waitForEvent('popup')
  await previous.evaluate(url => { window.open(url) }, otherOrigin)
  const popup = await popupEvent
  await popup.waitForLoadState('load')
  await seedState(popup)
  assert.deepEqual(await readState(popup), {
    global: 'must not survive', local: 'local', session: 'session', cookie: 'previous-case=cookie',
    databases: ['previous-case'], caches: ['previous-case'],
  })
  // These request URLs must never be passed to CDP as HTTP storage origins.
  const opaque = await worker.newPage(pageOptions)
  await opaque.goto('data:text/html,<title>Opaque case</title>')
  const blobURL = await previous.evaluate(() => URL.createObjectURL(new Blob(['<title>Blob case</title>'], { type: 'text/html' })))
  const blob = await worker.newPage(pageOptions)
  await blob.goto(blobURL)

  // CDP storage operations must target the worker's own browser context. The
  // same HTTP origin in another live worker must remain entirely untouched.
  workerB = await createIsolatedKitWorker(browser, base)
  const pageB = await workerB.newPage(pageOptions)
  await pageB.goto(base)
  assert.deepEqual(await readState(pageB), emptyState)
  await seedState(pageB)
  await workerB.reset()
  assert.ok(pageB.isClosed())
  assert.deepEqual(await readState(previous), {
    global: 'must not survive', local: 'local', session: 'session', cookie: 'previous-case=cookie',
    databases: ['previous-case'], caches: ['previous-case'],
  })
  assert.deepEqual(await readState(popup), {
    global: 'must not survive', local: 'local', session: 'session', cookie: 'previous-case=cookie',
    databases: ['previous-case'], caches: ['previous-case'],
  })
  assert.equal(await previous.evaluate(() => fetch('/sw-proof').then(response => response.text())), 'controlled by previous case')
  const cleanB = await workerB.newPage(pageOptions)
  await cleanB.goto(base)
  assert.deepEqual(await readState(cleanB), emptyState)

  const { resetMilliseconds } = await worker.reset()
  assert.ok(Number.isFinite(resetMilliseconds) && resetMilliseconds >= 0)
  assert.ok(previous.isClosed() && popup.isClosed() && opaque.isClosed() && blob.isClosed())
  const auditPages = worker.context.pages()
  assert.equal(auditPages.length, 3, 'only blank control and two inert origin audits survive')
  assert.ok(auditPages.includes(control) && baseAudit.isClosed(), 'reset replaces a navigated audit target before clearing storage')
  const cleanAudit = auditPages.find(page => page.url() === `${base}/`)
  assert.ok(cleanAudit, 'replacement is at the exact inert URL')
  assert.equal(await cleanAudit.evaluate(() => window.__auditAttack ?? window.__auditBlobAttack ?? false), false)
  assert.deepEqual(auditPages.filter(page => page !== control).map(page => new URL(page.url()).origin).sort(), [base, otherOrigin].sort())
  await worker.reset()
  assert.deepEqual(worker.context.pages(), auditPages, 'repeated resets do not create more audit documents')
  assert.equal(control.url(), 'about:blank')
  assert.deepEqual(await worker.context.cookies(), [])
  await worker.audit()
  // An exact-URL audit may still acquire a cross-site child target, where the
  // parent's script-disable flag is not a sufficient boundary.
  const childAttached = cleanAudit.waitForEvent('frameattached')
  await cleanAudit.evaluate(() => {
    const frame = document.createElement('iframe')
    frame.src = 'https://audit-child.invalid/'
    document.body.append(frame)
  })
  await childAttached
  assert.equal(cleanAudit.url(), `${base}/`)
  assert.equal(cleanAudit.frames().length, 2)
  await assert.rejects(() => worker.audit(), /Kit isolation audit page changed/)
  await worker.reset()
  assert.ok(cleanAudit.isClosed(), 'reset replaces an exact-URL audit containing a child frame')
  const singleFrameAudit = worker.context.pages().find(page => page.url() === `${base}/`)
  assert.ok(singleFrameAudit)
  assert.equal(singleFrameAudit.frames().length, 1)
  assert.equal(singleFrameAudit.workers().length, 0)

  const next = await worker.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: 'light' })
  assert.notEqual(next, previous)
  await next.goto(base)
  assert.deepEqual(await readState(next), emptyState)
  assert.match(await next.evaluate(() => navigator.userAgent), / StorybookTestRunner$/)
  assert.equal(await next.evaluate(() => navigator.serviceWorker.controller === null), true)
  assert.equal(await next.evaluate(async () => (await navigator.serviceWorker.getRegistrations()).length), 0)
  assert.notEqual(await next.evaluate(() => fetch('/sw-proof').then(response => response.text())), 'controlled by previous case')
  assert.deepEqual(next.viewportSize(), { width: 1440, height: 900 })
  assert.equal(await next.evaluate(() => matchMedia('(prefers-color-scheme: light)').matches), true)
  assert.equal(await next.evaluate(async () => (await navigator.permissions.query({ name: 'geolocation' })).state), 'prompt')
  assert.equal(await next.evaluate(() => fetch('/asset').then(response => response.text())), 'immutable asset')
  assert.equal(assetRequests, 1, 'HTTP asset cache survives storage reset')
  const nextOtherOrigin = await worker.newPage(pageOptions)
  await nextOtherOrigin.goto(otherOrigin)
  assert.deepEqual(await readState(nextOtherOrigin), emptyState)
  assert.equal(await nextOtherOrigin.evaluate(() => matchMedia('(prefers-color-scheme: dark)').matches), true)

  // Deliberately contaminate after a successful reset: the public audit must
  // observe real quota-backed storage, not merely trust reset's completion.
  await seedState(nextOtherOrigin)
  await worker.context.clearCookies()
  await assert.rejects(() => worker.audit(), /Kit isolation audit: residual storage/)
  await worker.context.addCookies([{ name: 'negative-control', value: 'present', url: base }])
  await assert.rejects(() => worker.audit(), /Kit isolation audit: .* residual cookies/)
  await worker.reset()

  // A case may legitimately write storage and navigate again. Only its first
  // HTTP(S) top-level document must start empty, matching fresh-context cases.
  const navigating = await worker.newPage(pageOptions)
  await navigating.goto(base)
  await navigating.evaluate(() => {
    localStorage.setItem('within-case', 'local')
    sessionStorage.setItem('within-case', 'session')
  })
  await navigating.goto(`${base}/another-document`)
  assert.deepEqual(await navigating.evaluate(() => ({
    local: localStorage.getItem('within-case'),
    session: sessionStorage.getItem('within-case'),
  })), { local: 'local', session: 'session' })
  await worker.reset()

  // localStorage may not contribute to CDP quota usage. The real strict
  // guard wraps the helper's goto, whose first-document audit must reject.
  const writer = await worker.newPage(pageOptions)
  await writer.goto(base)
  await writer.evaluate(() => localStorage.setItem('local-only-negative-control', 'present'))
  const previousCase = { id: 'kit--previous', scheme: 'dark', width: 440 }
  worker.setPreviousCase(previousCase)
  await assert.rejects(() => worker.audit(), error => {
    assert.deepEqual(error.isolation, {
      boundary: 'reset', origin: base,
      storage: [{ kind: 'localStorage', keys: ['local-only-negative-control'] }],
      workerId: 7, previousCase,
    })
    assert.match(error.message, /localStorage.*local-only-negative-control/)
    assert.ok(!error.message.includes('present'), 'storage values are never reported')
    return true
  })
  assert.deepEqual(await worker.context.cookies(), [])
  const contaminated = await worker.newPage(pageOptions)
  const contaminatedGuard = await strictStoryGuard(contaminated, base)
  await assert.rejects(() => contaminated.goto(`${base}/success/iframe.html?id=kit--success`, { waitUntil: 'domcontentloaded' }), error => {
    // The helper rejects before strictStoryGuard can accept storyFinished.
    assert.match(error.message, /Kit isolation document-start audit: residual Web Storage/)
    assert.deepEqual(error.isolation, {
      boundary: 'document-start', origin: base,
      storage: [{ kind: 'localStorage', keys: ['local-only-negative-control'] }],
      workerId: 7, previousCase,
    })
    return true
  })
  assert.equal(contaminatedGuard.records.length, 0)
  retainWebStorage = true
  try {
    await assert.rejects(() => worker.reset(), error => {
      assert.deepEqual(error.isolation, {
        boundary: 'reset', origin: base,
        storage: [{ kind: 'localStorage', keys: ['local-only-negative-control'] }],
        workerId: 7, previousCase,
      })
      return true
    })
  } finally {
    retainWebStorage = false
  }
  await worker.reset()
  const cleanStory = await worker.newPage(pageOptions)
  const cleanGuard = await strictStoryGuard(cleanStory, base)
  await cleanStory.goto(`${base}/success/iframe.html?id=kit--success`, { waitUntil: 'domcontentloaded' })
  await cleanGuard.check()
  assert.ok(cleanGuard.records.length > 0)
  await worker.reset()
  const sessionWriter = await worker.newPage(pageOptions)
  await sessionWriter.goto(otherOrigin)
  await sessionWriter.evaluate(() => sessionStorage.setItem('session-only-negative-control', 'private-value'))
  await assert.rejects(() => worker.audit(), error => {
    assert.deepEqual(error.isolation.storage, [{ kind: 'sessionStorage', keys: ['session-only-negative-control'] }])
    assert.equal(error.isolation.origin, otherOrigin)
    assert.ok(!error.message.includes('private-value'))
    return true
  })
  await worker.reset()

  // A pending composer-style debounce and a synchronous pagehide write must
  // not cross reset. No sleep: reset destroys the writer before storage clear.
  const pendingWriter = await worker.newPage(pageOptions)
  await pendingWriter.goto(base)
  await pendingWriter.evaluate(() => {
    setTimeout(() => localStorage.setItem('pending-debounce', 'private-value'), 300)
    addEventListener('pagehide', () => localStorage.setItem('pagehide-write', 'private-value'))
  })
  await worker.reset()
  assert.ok(pendingWriter.isClosed())
  await worker.audit()

  const broken = await worker.newPage(pageOptions)
  const guard = await strictStoryGuard(broken, base)
  await assert.rejects(() => broken.goto(`${base}/iframe.html?id=kit--broken`, { waitUntil: 'domcontentloaded' }), error => {
    assert.match(error.message, /strict-storybook-readiness/)
    assert.match(error.message, /deliberately broken play/)
    assert.match(error.message, /"status":"error"/)
    return true
  })
  assert.equal(guard.records.length, 0, 'broken play must not produce successful strict evidence')
  await worker.reset()
})
