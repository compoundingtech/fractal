/**
 * Built-Storybook dt10 gates (the server is owned by the caller):
 *   node scripts/details-run-gates.mjs <base-url> <output-dir> [canonical|painted|raw0] [story-id ...]
 * With no IDs, each mode runs its complete dt10 matrix at 1440x900 and 1280x720,
 * in dark and light. Receipts contain strict storyFinished evidence and controls.
 * PLAYWRIGHT_MODULE / CHROMIUM_PATH select the managed browser, matching the
 * repository's browser proof scripts; otherwise resolve the installed playwright.
 * DETAILS_EXTRA_ARGS appends Storybook args to canonical/raw0 navigations.
 * RawStability's resource initialization is reported separately, exactly as dt10;
 * raw0 asserts the unfiltered Details/strip/recovery window, not that resource window.
 */
import fs from 'node:fs'
import { join } from 'node:path'
import { strictStoryGuard } from './details-strict-story-guard.mjs'
import { tabStripPaintedIntersection } from './details-painted-intersection.mjs'

const [baseUrl, output, mode = 'canonical', ...requestedIds] = process.argv.slice(2)
if (!baseUrl || !output || !['canonical', 'painted', 'raw0'].includes(mode)) {
  throw new Error('Usage: node scripts/details-run-gates.mjs <base-url> <output-dir> [canonical|painted|raw0] [story-id ...]')
}
const base = baseUrl.replace(/\/+$/, '')
const storyId = suffix => `fractal-ui-details-tab-strip--${suffix}`
const canonicalIds = [
  'wide', 'threshold',
  'mounted-159-selected-first', 'mounted-160-selected-first',
  'mounted-159-selected-last', 'mounted-160-selected-last',
  'mounted-159-selected-first-light', 'mounted-160-selected-first-light',
  'mounted-159-selected-last-light', 'mounted-160-selected-last-light',
  'many-tabs', 'light', 'dark',
  'raw-stability', 'raw-stability-159', 'raw-stability-160', 'all-states',
].map(storyId)
const paintedIds = [
  'mounted-159-selected-first', 'mounted-159-selected-last',
  'mounted-160-selected-first', 'mounted-160-selected-last', 'many-tabs',
].map(storyId)
const rawIds = ['raw-stability', 'raw-stability-159', 'raw-stability-160'].map(storyId)
const ids = requestedIds.length ? requestedIds : mode === 'painted' ? paintedIds : mode === 'raw0' ? rawIds : canonicalIds
const viewports = [{ width: 1440, height: 900 }, { width: 1280, height: 720 }]
const rows = []
const expectedCases = ids.length * viewports.length * 2
const writeReceipt = () => {
  const cases = mode === 'painted' ? rows.reduce((count, row) => count + (row.cases ?? 0), 0) : rows.length
  const receipt = { mode, base, pass: cases === expectedCases && rows.every(row => row.pass), expectedCases, cases, rows }
  fs.writeFileSync(join(output, 'receipt.json'), JSON.stringify(receipt, null, 2))
  return receipt
}
const rawEvidence = facts => {
  if (!facts.secureContext || facts.raw.length === 0) throw new Error('RawStability requires a native secure context and raw LayoutShift evidence')
  return facts.raw.map(dataset => {
    const entries = JSON.parse(dataset.rawLayoutShiftEntries)
    const controlEntries = JSON.parse(dataset.rawLayoutShiftControlEntries)
    const rawSum = entries.reduce((total, entry) => total + entry.value, 0)
    const controlSum = controlEntries.reduce((total, entry) => total + entry.value, 0)
    if (Number(dataset.rawLayoutShiftSum) !== 0 || rawSum !== 0) throw new Error('Nonzero raw layout shift')
    if (!(controlSum > 0) || !controlEntries.some(entry => entry.value > 0 && entry.sources?.length > 0)) {
      throw new Error('Raw LayoutShift displacement control did not collect a positive shift with source rectangles')
    }
    return {
      rawSum, entries, controlSum, controlEntries,
      resourceSum: Number(dataset.resourceLayoutShiftSum),
      resourceEntries: JSON.parse(dataset.resourceLayoutShiftEntries),
    }
  })
}

fs.mkdirSync(output, { recursive: true })
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, async () => {
  await browser.close()
  process.exit(128 + (signal === 'SIGTERM' ? 15 : 2))
})
try {
  if (mode === 'painted') {
    for (const id of ids) {
      const page = await browser.newPage()
      const guard = await strictStoryGuard(page, base)
      try {
        const report = await tabStripPaintedIntersection(page, base, id)
        await guard.check()
        rows.push({ id, ...report, strict: guard.records })
      } catch (error) {
        rows.push({ id, pass: false, error: String(error), strict: guard.records })
      } finally { await page.close() }
      writeReceipt()
      console.log(JSON.stringify({ id, pass: rows.at(-1).pass, failures: rows.at(-1).failures ?? rows.at(-1).error }))
    }
  } else {
    cases: for (const id of ids) for (const scheme of ['dark', 'light']) for (const viewport of viewports) {
      const page = await browser.newPage({ viewport, colorScheme: scheme })
      const guard = await strictStoryGuard(page, base)
      const label = `${id}-${scheme}-${viewport.width}x${viewport.height}`
      try {
        const storyArgs = [`scheme:${scheme}`, process.env.DETAILS_EXTRA_ARGS].filter(Boolean).join(';')
        await page.goto(`${base}/iframe.html?id=${id}&viewMode=story&args=${encodeURIComponent(storyArgs)}`, { waitUntil: 'networkidle' })
        await guard.check()
        const facts = await page.evaluate(() => ({
          secureContext: isSecureContext,
          raw: [...document.querySelectorAll('[data-raw-layout-shift-sum]')].map(element => ({ ...element.dataset })),
          strips: [...document.querySelectorAll('[data-tab-strip]')].map(list => ({
            dataset: { ...list.dataset }, scrollLeft: list.scrollLeft, scrollWidth: list.scrollWidth,
            width: list.getBoundingClientRect().width,
            tabs: [...list.querySelectorAll('[role="tab"]')].map(tab => ({
              name: tab.getAttribute('aria-label'), kind: tab.dataset.tabKind,
              selected: tab.getAttribute('aria-selected'), clipped: tab.hasAttribute('data-tab-clipped'),
              rect: tab.getBoundingClientRect().toJSON(),
            })),
          })),
        }))
        const raw = mode === 'raw0' || rawIds.includes(id) ? rawEvidence(facts) : undefined
        rows.push({ label, pass: true, facts, raw, strict: guard.records })
      } catch (error) {
        const diagnostic = await page.evaluate(() => ({
          raw: [...document.querySelectorAll('[data-raw-layout-shift-sum]')].map(element => ({ ...element.dataset })),
          tabs: [...document.querySelectorAll('[role="tab"]')].map(tab => ({
            name: tab.getAttribute('aria-label'), selected: tab.getAttribute('aria-selected'),
            clipped: tab.hasAttribute('data-tab-clipped'), rect: tab.getBoundingClientRect().toJSON(),
          })),
        })).catch(() => null)
        rows.push({ label, pass: false, error: String(error), diagnostic, strict: guard.records })
      }
      writeReceipt()
      try { await page.screenshot({ path: join(output, `${label}.png`) }) } finally { await page.close() }
      if (!rows.at(-1).pass) break cases
    }
  }
} finally { await browser.close() }
const receipt = writeReceipt()
console.log(JSON.stringify({ pass: receipt.pass, cases: receipt.cases, expectedCases, failures: rows.filter(row => !row.pass) }))
if (!receipt.pass) process.exitCode = 1
