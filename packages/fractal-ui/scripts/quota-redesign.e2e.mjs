import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'

// Existing Storybook only: no server lifecycle or dependency installation.
// Supply the workspace/Nix-pinned Playwright module and its managed Chromium executable.
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const base = process.env.QUOTA_STORYBOOK_URL
const out = process.env.QUOTA_EVIDENCE_DIR
assert(base && out, 'Set QUOTA_STORYBOOK_URL and QUOTA_EVIDENCE_DIR')
await mkdir(out, { recursive: true })
const index = await (await fetch(new URL('index.json', `${base.replace(/\/$/, '')}/`))).json()
const stories = Object.values(index.entries).filter(entry => entry.type === 'story' && entry.title.startsWith('Fractal/App/Quota/'))
assert(stories.length >= 30, 'Complete redesign story matrix is indexed')
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH, headless: true })
const report = { base, stories: stories.map(entry => entry.id), plays: [], checks: [], controls: [], screenshots: [] }
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } })
page.setDefaultTimeout(90000)
const errors = []
page.on('pageerror', error => errors.push(error.message))
const load = async id => {
  errors.length = 0
  await page.goto(new URL(`iframe.html?id=${id}&viewMode=story`, `${base.replace(/\/$/, '')}/`).href, { waitUntil: 'domcontentloaded', timeout: 90000 })
  await page.locator('[data-quota-story]').first().waitFor()
  await page.waitForFunction(storyId => window.__STORYBOOK_ADDONS_CHANNEL__?.last('storyFinished')?.[0]?.storyId === storyId, id, { timeout: 90000 })
  const result = await page.evaluate(() => window.__STORYBOOK_ADDONS_CHANNEL__.last('storyFinished')[0])
  const playError = await page.evaluate(() => window.__STORYBOOK_ADDONS_CHANNEL__.last(window.__STORYBOOK_MODULE_CORE_EVENTS__.PLAY_FUNCTION_THREW_EXCEPTION)?.[0]?.message)
  assert(!playError, `${id} authored play failed: ${playError?.split('\n')[0]}`)
  assert.equal(result.status, 'success', `${id} play status: ${JSON.stringify(result)}; ${errors.join('; ')}`)
  assert.equal(errors.length, 0, `${id} browser errors: ${errors.join('; ')}`)
  return result
}
const order = () => page.locator('[data-quota-row]').evaluateAll(rows => rows.map(row => row.getAttribute('data-quota-row')))
const defaultOrder = ['harbor', 'north', 'birch', 'delta', 'meadow', 'lake', 'elm', 'cove', 'unknown']
const alphabetical = ['birch', 'cove', 'delta', 'elm', 'harbor', 'lake', 'meadow', 'north', 'unknown']
const sortCheck = async () => {
  assert.deepEqual(await order(), defaultOrder)
  await page.locator('[data-quota-sort="account"]').click()
  assert.deepEqual(await order(), alphabetical, 'Account sort changes rendered order')
}
const keyboardCheck = async () => {
  await page.locator('[data-quota-drawer]').focus()
  await page.keyboard.press('s')
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-sort-key'), 'comparison', 'Focused s cycles sort')
  await page.keyboard.press('S')
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-sort-direction'), 'ascending', 'Focused S reverses direction')
  await page.keyboard.press('u')
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-unit'), 'tokens', 'Focused u toggles unit')
  await page.keyboard.press('m')
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-collapsed'), 'true', 'Focused m collapses')
  await page.keyboard.press('m')
  await page.keyboard.press('Escape')
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-escaped'), '1', 'Escape reaches host')
}
const brushCheck = async () => {
  const slider = page.getByRole('slider', { name: /Range start/ })
  await slider.focus(); await page.keyboard.press('Home')
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-range'), '0:29')
  const before = await page.locator('[data-quota-breakdown]').innerText()
  await page.keyboard.press('ArrowRight'); await page.keyboard.press('ArrowRight'); await page.keyboard.press('ArrowRight')
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-range'), '3:29', 'Brush changes controlled range')
  assert.notEqual(await page.locator('[data-quota-breakdown]').innerText(), before, 'Brush changes actual table values')
  await page.locator('[data-quota-day="2030-04-16"]').first().click()
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-selected-day'), '2030-04-16')
  assert.match(await page.locator('[data-quota-breakdown]').innerText(), /16 Apr|2030-04-16/, 'Bar click filters table to exact day')
}
const geometry = () => page.locator('[data-quota-row], [data-quota-status], [data-quota-breakdown], [data-quota-usage] svg').evaluateAll(nodes => nodes.map(node => {
  const rect = node.getBoundingClientRect(); return [rect.x, rect.y, rect.width, rect.height]
}))
const refreshCheck = async () => {
  const before = await geometry()
  assert(before.length > 0, 'Measure real content geometry')
  await page.evaluate(() => {
    window.__quotaShifts = []
    window.__quotaShiftObserver?.disconnect()
    window.__quotaShiftObserver = new PerformanceObserver(list => window.__quotaShifts.push(...list.getEntries().map(entry => entry.value)))
    window.__quotaShiftObserver.observe({ type: 'layout-shift', buffered: false })
  })
  const refreshed = await page.locator('[data-quota-story]').getAttribute('data-refreshed')
  await page.getByRole('button', { name: 'Refresh snapshot', exact: true }).click()
  await page.waitForFunction(previous => document.querySelector('[data-quota-story]').dataset.refreshed !== previous, refreshed)
  assert.deepEqual(await geometry(), before, 'Snapshot refresh preserves row/chart geometry')
  await page.getByRole('button', { name: 'Simulate failed refresh', exact: true }).click()
  await page.getByText(/Collection failed/).first().waitFor()
  assert.deepEqual(await geometry(), before, 'Failed refresh keeps last good geometry')
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  const shifts = await page.evaluate(() => { window.__quotaShiftObserver.disconnect(); return window.__quotaShifts })
  assert.equal(shifts.reduce((sum, value) => sum + value, 0), 0, 'Refresh produces zero layout shift (including recent-input entries)')
}
const control = async (name, id, mutate, check) => {
  await load(id)
  await mutate()
  let failure
  try { await check() } catch (error) { failure = error.message }
  assert(failure, `Negative control ${name} must fail the positive assertion`)
  report.controls.push({ name, expectedFailure: failure })
}
try {
  // Run the authored plays themselves, not a parallel story-only reimplementation.
  for (const story of stories) {
    await load(story.id)
    report.plays.push({ id: story.id, result: 'completed' })
    if (['--dark', '--light', '--columns', '--stacked-mini-meters', '--governing-and-chips', '--overview-and-brush', '--small-multiples', '--heatmap-calendar', '--overview-900', '--overview-light', '--multiples-light-900', '--heatmap-light-900', '--failed-retained', '--not-yet-published'].some(suffix => story.id.endsWith(suffix))) {
      const file = `${story.id}.png`
      await page.locator('[data-quota-story]').screenshot({ path: `${out}/${file}` })
      report.screenshots.push(file)
    }
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)
    assert.equal(overflow, false, `${story.id} has no horizontal document overflow`)
  }
  const drawer = 'fractal-app-quota-drawer-m--dark'
  const usage = 'fractal-app-quota-usage-page-variants--overview-and-brush'
  for (const [name, id, check] of [['sort', drawer, sortCheck], ['keyboard', drawer, keyboardCheck], ['brush', usage, brushCheck], ['drawer-refresh', drawer, refreshCheck], ['usage-refresh', usage, refreshCheck]]) {
    await load(id); await check(); report.checks.push({ name, result: 'passed' })
  }
  await control('disabled-sort', drawer, () => page.evaluate(() => {
    for (const type of ['pointerdown', 'pointerup', 'click']) document.addEventListener(type, event => {
      if (event.target.closest('[data-quota-sort="account"]')) { event.preventDefault(); event.stopImmediatePropagation() }
    }, true)
  }), sortCheck)
  await control('disabled-keyboard', drawer, () => page.evaluate(() => document.addEventListener('keydown', event => { event.preventDefault(); event.stopImmediatePropagation() }, true)), keyboardCheck)
  await control('disabled-brush', usage, () => page.evaluate(() => document.addEventListener('keydown', event => { event.preventDefault(); event.stopImmediatePropagation() }, true)), brushCheck)
  await control('refresh-layout-shift', drawer, () => page.getByRole('button', { name: 'Refresh snapshot', exact: true }).evaluate(node => node.addEventListener('click', () => { document.querySelector('[data-quota-status]').style.marginTop = '25px' }, true)), refreshCheck)
  await load(drawer)
  await page.getByLabel('Filter provider', { exact: true }).selectOption('Orbit')
  assert.deepEqual(await order(), ['harbor', 'birch', 'lake', 'cove'], 'Provider filter changes roster')
  await page.getByRole('button', { name: 'Group by provider', exact: true }).click()
  assert.equal(await page.locator('[data-quota-provider="Orbit"]').count(), 1, 'Provider group subtotal appears')
  await page.getByRole('button', { name: 'Open usage for Harbor', exact: true }).click()
  await page.locator('[data-quota-usage]').waitFor()
  assert.equal(await page.locator('[data-quota-story]').getAttribute('data-selected-account'), 'harbor', 'Row opens Usage with stable account identity')
  await page.getByRole('tab', { name: 'Account', exact: true }).click()
  await page.getByRole('tabpanel').getByText('Reported available resets', { exact: true }).waitFor()
  await page.getByRole('tab', { name: 'History', exact: true }).click()
  report.checks.push({ name: 'filters-group-navigation-detail-tabs', result: 'passed' })
  await writeFile(`${out}/story-ids.json`, JSON.stringify(report.stories, null, 2))
  await writeFile(`${out}/e2e-report.json`, JSON.stringify(report, null, 2))
  console.log(JSON.stringify(report, null, 2))
} catch (error) {
  report.failure = { message: error.message, url: page.url(), browserErrors: errors }
  await page.screenshot({ path: `${out}/failure.png`, fullPage: true }).catch(() => {})
  await writeFile(`${out}/e2e-report.json`, JSON.stringify(report, null, 2))
  throw error
} finally { await browser.close() }
