// dt10 strict lifecycle: only the two exact resource 404s are tolerated.
export async function strictStoryGuard(page, base, { finishFiniteAnimations = false } = {}) {
  const errors = [], records = [], ignored = []
  const allow404 = message => message.type() === 'error' && message.text() === 'Failed to load resource: the server responded with a status of 404 (Not Found)' && [`${base}/vite-inject-mocker-entry.js`, `${base}/favicon.ico`].includes(message.location().url)
  const originalOn = page.on.bind(page), originalOff = page.off.bind(page), listeners = new WeakMap()
  originalOn('console', message => { if (message.type() === 'error') { const fact = { text: message.text(), location: message.location() }; (allow404(message) ? ignored : errors).push(fact) } })
  page.on = (event, listener) => { if (event !== 'console') return originalOn(event, listener); const filtered = message => { if (!allow404(message)) listener(message) }; listeners.set(listener, filtered); return originalOn(event, filtered) }
  page.off = (event, listener) => originalOff(event, event === 'console' ? listeners.get(listener) ?? listener : listener)
  originalOn('pageerror', error => errors.push({ kind: 'page', message: error.message }))
  await page.addInitScript(({ finishFiniteAnimations }) => {
    window.__dt10StrictEvents = []
    window.__dt10Readiness = { phases: [], finiteAnimationsFinished: 0, animationErrors: [] }
    document.addEventListener('DOMContentLoaded', () => {
      window.__dt10Readiness.domContentLoaded = performance.now()
      void document.fonts.ready.then(() => { window.__dt10Readiness.fontsReady = performance.now() })
      window.__dt10Readiness.stylesheets = [...document.styleSheets].map(sheet => ({ href: sheet.href }))
      window.__dt10Readiness.cssResources = performance.getEntriesByType('resource')
        .filter(entry => new URL(entry.name).pathname.endsWith('.css'))
        .map(({ name, startTime, responseEnd }) => ({ name, startTime, responseEnd }))
    }, { once: true })
    const timer = setInterval(() => {
      const channel = window.__STORYBOOK_ADDONS_CHANNEL__
      if (!channel) return
      clearInterval(timer)
      const readiness = window.__dt10Readiness
      readiness.channelReady = performance.now()
      channel.on('storyRenderPhaseChanged', ({ storyId, newPhase, renderId }) => {
        readiness.phases.push({ storyId, phase: newPhase, renderId, milliseconds: performance.now() })
        // Storybook emits completing after play and before its animation wait
        // and axe afterEach. Finish, never pause, so axe sees the same endpoint.
        if (!finishFiniteAnimations || newPhase !== 'completing') return
        const roots = [document]
        for (let index = 0; index < roots.length; index++) {
          for (const element of roots[index].querySelectorAll('*')) {
            if (element.shadowRoot) roots.push(element.shadowRoot)
          }
        }
        for (const animation of new Set(roots.flatMap(root => root.getAnimations()))) {
          if (animation.playState !== 'running') continue
          // Finite endTime includes iterations and delays. Leave infinite
          // animations entirely alone, including Storybook's existing timeout.
          if (!Number.isFinite(animation.effect?.getComputedTiming().endTime)) continue
          try {
            animation.finish()
            readiness.finiteAnimationsFinished++
          } catch (error) {
            readiness.animationErrors.push(String(error))
          }
        }
      })
      for (const name of ['storyFinished', 'playFunctionThrewException', 'storyThrewException', 'storyErrored']) channel.on(name, payload => window.__dt10StrictEvents.push({ name, payload: name === 'storyFinished' ? { storyId: payload?.storyId, status: payload?.status, reporters: payload?.reporters?.map(({ type, status }) => ({ type, status })) } : { message: typeof payload === 'string' ? payload : payload?.message, stack: payload?.stack } }))
    }, 5)
  }, { finishFiniteAnimations })
  const check = async () => {
    if (!page.url().includes('/iframe.html')) return
    const { events, readiness } = await page.evaluate(() => ({
      events: window.__dt10StrictEvents ?? [], readiness: window.__dt10Readiness,
    }))
    const finished = events.filter(event => event.name === 'storyFinished'), exceptions = events.filter(event => event.name !== 'storyFinished')
    const record = { url: page.url(), finished, exceptions, errors: [...errors], allowed404s: [...ignored], readiness }
    if (errors.length || exceptions.length || readiness?.animationErrors.length || !finished.length || finished.some(event => event.payload.status !== 'success')) throw new Error(JSON.stringify({ rule: 'strict-storybook-readiness', ...record }))
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
