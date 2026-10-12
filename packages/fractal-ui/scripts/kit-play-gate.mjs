import { createServer } from 'node:http'
import { readFile, stat, mkdir, writeFile } from 'node:fs/promises'
import { resolve, sep, extname, join } from 'node:path'
import { performance } from 'node:perf_hooks'
import { strictStoryGuard } from './details-strict-story-guard.mjs'
import { availableParallelism } from 'node:os'

// Own an ephemeral localhost server: missing assets remain real, standard 404s.
const [buildDirectory, outputDirectory] = process.argv.slice(2)
if (!buildDirectory || !outputDirectory) throw new Error('Usage: node kit-play-gate.mjs <storybook-static> <receipt-directory>')
const root = resolve(buildDirectory)
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2' }
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
    let file = resolve(root, `.${pathname}`)
    if (file !== root && !file.startsWith(root + sep)) {
      response.writeHead(403).end('Forbidden')
      return
    }
    if ((await stat(file)).isDirectory()) file = join(file, 'index.html')
    const bytes = await readFile(file)
    response.writeHead(200, { 'Content-Type': mime[extname(file)] ?? 'application/octet-stream' }).end(bytes)
  } catch (error) {
    const missing = error.code === 'ENOENT' || error.code === 'ENOTDIR'
    response.writeHead(missing ? 404 : 500).end(missing ? 'Not Found' : 'Internal Server Error')
  }
})
await new Promise((accept, reject) => {
  server.once('error', reject)
  server.listen(0, '127.0.0.1', accept)
})
const base = `http://127.0.0.1:${server.address().port}`
const started = performance.now()
const rows = []
const workers = Math.min(6, availableParallelism())
let ids = [], browser, fatalError
await mkdir(outputDirectory, { recursive: true })
const writeReceipt = async () => {
  const receipt = {
    pass: !fatalError && ids.length > 0 && rows.length === ids.length * 4 && rows.every(row => row.pass),
    stories: ids.length, expectedCases: ids.length * 4, cases: rows.length, workers,
    elapsedSeconds: (performance.now() - started) / 1000, fatalError, rows: rows.toSorted((a, b) => a.id.localeCompare(b.id) || a.scheme.localeCompare(b.scheme) || a.width - b.width),
  }
  await writeFile(join(outputDirectory, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n')
  return receipt
}
try {
  const response = await fetch(`${base}/index.json`)
  if (!response.ok) throw new Error(`Storybook index: HTTP ${response.status}`)
  const index = await response.json()
  ids = Object.values(index.entries).filter(entry => entry.type === 'story').map(entry => entry.id).sort()
  if (!ids.length || new Set(ids).size !== ids.length) throw new Error('Storybook index must contain unique, nonempty story IDs')
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
  browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
  const cases = ids.flatMap(id => ['dark', 'light'].flatMap(scheme => [440, 1440].map(width => ({ id, scheme, width }))))
  let nextCase = 0
  const runCase = async ({ id, scheme, width }) => {
    const caseStarted = performance.now()
    const page = await browser.newPage({ viewport: { width, height: 900 }, colorScheme: scheme })
    const guard = await strictStoryGuard(page, base)
    const row = { id, scheme, width, height: 900, pass: false }
    try {
      await page.goto(`${base}/iframe.html?id=${encodeURIComponent(id)}&viewMode=story&args=${encodeURIComponent(`scheme:${scheme}`)}`, { waitUntil: 'networkidle' })
      await guard.check()
      if (!guard.records.every(record => record.finished.every(event => event.payload.storyId === id))) {
        throw new Error('storyFinished evidence belongs to a different story')
      }
      row.pass = true
    } catch (error) {
      row.error = String(error)
      row.events = await page.evaluate(() => window.__dt10StrictEvents ?? []).catch(() => [])
    } finally {
      row.strict = guard.records
      row.elapsedSeconds = (performance.now() - caseStarted) / 1000
      rows.push(row)
      await page.close()
      console.log(JSON.stringify(row))
    }
  }
  const results = await Promise.allSettled(Array.from({ length: workers }, async () => {
    while (nextCase < cases.length) await runCase(cases[nextCase++])
  }))
  const rejected = results.filter(result => result.status === 'rejected')
  if (rejected.length) throw new AggregateError(rejected.map(result => result.reason), 'Kit play workers failed')
} catch (error) {
  fatalError = String(error)
} finally {
  await browser?.close()
  await new Promise(resolve => server.close(resolve))
}
const receipt = await writeReceipt()
// The complete receipt stays in CI logs even when the gate fails.
console.log(JSON.stringify(receipt))
if (!receipt.pass) process.exitCode = 1
