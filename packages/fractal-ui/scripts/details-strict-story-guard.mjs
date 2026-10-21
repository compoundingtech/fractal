// dt10 strict lifecycle: only the two exact resource 404s are tolerated.
export async function strictStoryGuard(page, base) {
  const errors = [], records = [], ignored = []
  const allow404 = message => message.type() === 'error' && message.text() === 'Failed to load resource: the server responded with a status of 404 (Not Found)' && [`${base}/vite-inject-mocker-entry.js`, `${base}/favicon.ico`].includes(message.location().url)
  const originalOn = page.on.bind(page), originalOff = page.off.bind(page), listeners = new WeakMap()
  originalOn('console', message => { if (message.type() === 'error') { const fact = { text: message.text(), location: message.location() }; (allow404(message) ? ignored : errors).push(fact) } })
  page.on = (event, listener) => { if (event !== 'console') return originalOn(event, listener); const filtered = message => { if (!allow404(message)) listener(message) }; listeners.set(listener, filtered); return originalOn(event, filtered) }
  page.off = (event, listener) => originalOff(event, event === 'console' ? listeners.get(listener) ?? listener : listener)
  originalOn('pageerror', error => errors.push({ kind: 'page', message: error.message }))
  await page.addInitScript(() => {
    window.__dt10StrictEvents = []
    // A failed a11y report names its axe rules and the nodes that broke them (or the error that stopped axe),
    // so the failure message says what to fix.
    const summarize = result => result?.error ? { error: String(result.error?.message ?? result.error) } : { violations: (result?.violations ?? []).map(({ id, impact, nodes }) => ({ id, impact, nodes: nodes.map(({ target, html, failureSummary }) => ({ target, html: html.slice(0, 200), failureSummary })) })) }
    const timer = setInterval(() => {
      const channel = window.__STORYBOOK_ADDONS_CHANNEL__
      if (!channel) return
      clearInterval(timer)
      for (const name of ['storyFinished', 'playFunctionThrewException', 'storyThrewException', 'storyErrored']) channel.on(name, payload => window.__dt10StrictEvents.push({ name, payload: name === 'storyFinished' ? { storyId: payload?.storyId, status: payload?.status, reporters: payload?.reporters?.map(({ type, status, result }) => ({ type, status, ...(status === 'passed' ? {} : { failure: summarize(result) }) })) } : { message: typeof payload === 'string' ? payload : payload?.message, stack: payload?.stack } }))
    }, 5)
  })
  const check = async () => {
    if (!page.url().includes('/iframe.html')) return
    const events = await page.evaluate(() => window.__dt10StrictEvents ?? [])
    const finished = events.filter(event => event.name === 'storyFinished'), exceptions = events.filter(event => event.name !== 'storyFinished')
    const record = { url: page.url(), finished, exceptions, errors: [...errors], allowed404s: [...ignored] }
    if (errors.length || exceptions.length || !finished.length || finished.some(event => event.payload.status !== 'success')) throw new Error(JSON.stringify({ rule: 'strict-storybook-readiness', ...record }))
    records.push(record)
  }
  const goto = page.goto.bind(page)
  page.goto = async (...args) => {
    await check()
    errors.length = 0; ignored.length = 0
    const result = await goto(...args)
    if (page.url().includes('/iframe.html')) {
      await page.waitForFunction(() => window.__dt10StrictEvents?.some(event => ['storyFinished', 'playFunctionThrewException', 'storyThrewException', 'storyErrored'].includes(event.name)), undefined, { timeout: 600000 })
      await check()
    }
    return result
  }
  return { check, records }
}
