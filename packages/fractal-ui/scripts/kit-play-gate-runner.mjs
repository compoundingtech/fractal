import { createServer } from 'node:http'
import { playWorkers } from './kit-play-workers.mjs'
import { readFile, stat, mkdir, writeFile, appendFile } from 'node:fs/promises'
import { resolve, sep, extname, join } from 'node:path'
import { performance } from 'node:perf_hooks'
import { strictStoryGuard } from './details-strict-story-guard.mjs'
import { availableParallelism, cpus, loadavg } from 'node:os'
import { storyIdsFromIndex } from './kit-play-index.mjs'
import { commitSetCases, fullCases, FULL_CASE_OVERRIDES, validateCaseOverrides } from './kit-play-commit-set.mjs'
import { createIsolatedKitWorker } from './kit-play-isolation.mjs'
import { shardStories } from './kit-play-shards.mjs'
import { readinessTimings, summarizeTimings } from './kit-play-timings.mjs'
import { captureKitCpu, kitCpuDelta } from './kit-play-cpu.mjs'
import { assessKitPlayCost, kitPlayBudget } from './kit-play-budget.mjs'
export function assertStrictEvidence(records, id, a11y) {
  if (records.length !== 1 || records[0].finished.length !== 1) {
    throw new Error('expected exactly one completed strict story record')
  }
  const { payload } = records[0].finished[0]
  if (payload.storyId !== id || payload.status !== 'success') throw new Error('storyFinished evidence is not a successful completion of the requested story')
  const reports = (payload.reporters ?? []).filter(report => report.type === 'a11y')
  if (a11y === 'on' && !(reports.length === 1 && reports[0].status === 'passed')) throw new Error(`a11y on: expected one passed a11y report, got ${JSON.stringify(reports)}`)
  if (a11y === 'off' && reports.length) throw new Error(`a11y off: axe still ran: ${JSON.stringify(reports)}`)
}


// The CLI always supplies the reviewed source policy; fixture tests may inject theirs directly.
export async function runKitPlayGate({ buildDirectory, outputDirectory, overrides = FULL_CASE_OVERRIDES }) {
  const shardIndex = Number(process.env.KIT_PLAY_SHARD_INDEX ?? 0)
  const shardCount = Number(process.env.KIT_PLAY_SHARD_COUNT ?? 1)
  // KIT_PLAY_SET=critical runs only stories tagged `critical`; KIT_PLAY_SET=commit is the per-commit
  // set, a narrowing of those (see kit-play-commit-set.mjs for what it leaves out);
  // Commit plays use raw CPU seconds; wall time is only a hang guard.
  // Empty environment values mean unset.
  const option = (name, allowed, fallback) => {
    const value = process.env[name] || fallback
    if (!allowed.includes(value)) throw new Error(`${name} must be one of ${allowed.join(', ')}`)
    return value
  }
  const set = option('KIT_PLAY_SET', ['all', 'critical', 'commit'], 'all')
  const a11y = option('KIT_PLAY_A11Y', ['on', 'off'], 'on')
  const budget = kitPlayBudget(process.env)
  const initialCpu = await captureKitCpu()
  // Only the full set is sharded; the critical and per-commit sets run whole on one runner.
  if (set !== 'all' && (shardCount !== 1 || shardIndex !== 0)) throw new Error(`KIT_PLAY_SET=${set} runs unsharded; only KIT_PLAY_SET=all is sharded`)
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
      // This server owns one immutable build for the job's lifetime. Context reuse
      // may cache resources, never the mutable storage belonging to a story.
      response.writeHead(200, {
        'Content-Type': mime[extname(file)] ?? 'application/octet-stream',
        'Cache-Control': 'public, max-age=31536000, immutable',
      }).end(bytes)
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
  let playStarted = performance.now(), elapsedSeconds
  let cpuStarted = initialCpu, cpuDelta, loadStarted = loadavg(), loadEnded
  const rows = []
  const workers = playWorkers(process.env, availableParallelism())
  let ids = [], fullIds = [], expectedCases = 0, omitted = { stories: [], cases: 0 }, browser, fatalError, storyFiles = {}
  await mkdir(outputDirectory, { recursive: true })
  const writeReceipt = async () => {
    const cost = assessKitPlayCost(budget, {
      elapsedSeconds, busyCoreMinutes: cpuDelta.busyCoreMinutes, costRequired: set === 'commit',
    })
    const receipt = {
      pass: !fatalError && !cost.overBudget && (shardCount > 1 ? fullIds : ids).length > 0 && rows.length === expectedCases && rows.every(row => row.pass),
      shardIndex, shardCount, fullIds,
      runId: process.env.GITHUB_RUN_ID, runAttempt: Number(process.env.GITHUB_RUN_ATTEMPT),
      headSha: process.env.GITHUB_SHA,
      set, a11y, ...cost,
      stories: ids.length, expectedCases, cases: rows.length, workers, omitted,
      caseOverrides: Object.fromEntries([...overrides].filter(([id]) => fullIds.includes(id))),
      passed: rows.filter(row => row.pass).length, failed: rows.filter(row => !row.pass).length,
      playMinutes: elapsedSeconds / 60,
      storybookBuildSeconds: Number(process.env.KIT_STORYBOOK_BUILD_SECONDS ?? 0),
      isolation: 'reused-context-fresh-page-cleared-storage',
      cpu: { count: cpus().length, ...cpuDelta, loadavgStart: loadStarted, loadavgEnd: loadEnded },
      elapsedSeconds, fatalError, rows: rows.toSorted((a, b) => a.id.localeCompare(b.id) || a.scheme.localeCompare(b.scheme) || a.width - b.width),
      effectiveWorkerSecondsPerCase: rows.length ? elapsedSeconds * workers / rows.length : null,
      meanCaseSeconds: rows.length ? rows.reduce((sum, row) => sum + row.elapsedSeconds, 0) / rows.length : null,
      ...summarizeTimings(rows),
    }
    await writeFile(join(outputDirectory, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n')
    const { rows: _rows, ...summary } = receipt
    console.log(`KIT_PLAY_SUMMARY ${JSON.stringify(summary)}`)
    if (process.env.GITHUB_STEP_SUMMARY && process.env.KIT_PLAY_SHARD_COUNT) {
      await appendFile(process.env.GITHUB_STEP_SUMMARY, `\n| Shard | Cases (completed/expected) | Passed | Failed | Play minutes | Result |\n| --- | --- | --- | --- | --- | --- |\n| kit ${shardIndex}/${shardCount} | ${receipt.cases}/${receipt.expectedCases} | ${receipt.passed} | ${receipt.failed} | ${receipt.playMinutes.toFixed(2)} | ${receipt.pass ? 'PASS' : 'FAIL'} |\n`)
      await appendFile(process.env.GITHUB_STEP_SUMMARY, `\nkit ${shardIndex}/${shardCount} Storybook build: ${(receipt.storybookBuildSeconds / 60).toFixed(2)} runner-minutes (included in this shard; duplicate build cost is the sum of nonzero-shard builds).\n`)
    }
    if (cost.overCpuBudget) console.error(`Kit plays (${set}) used ${cost.cpuSeconds.toFixed(2)} CPU s, over the ${cost.maxCpuSeconds} CPU s limit (anchor ${cost.cpuAnchorSeconds} s +10%).`)
    if (cost.overWallGuard) console.error(`Kit plays (${set}) took ${elapsedSeconds.toFixed(1)} s, over the ${cost.wallGuardSeconds} s wall hang guard.`)
    return receipt
  }
  try {
    const response = await fetch(`${base}/index.json`)
    if (!response.ok) throw new Error(`Storybook index: HTTP ${response.status}`)
    // Validate the complete index; fullIds is the selected set's inventory, which the union gate
    // checks a sharded full run against. Only KIT_PLAY_SET=all partitions it across shards.
    const index = await response.json()
    fullIds = storyIdsFromIndex(index, set === 'all' ? {} : { tag: 'critical' }).toSorted()
    validateCaseOverrides(index.entries, overrides)
    storyFiles = Object.fromEntries(Object.values(index.entries).filter(entry => entry.type === 'story').map(entry => [entry.id, entry.importPath]))
    ids = set === 'all' ? shardStories(fullIds, shardIndex, shardCount) : fullIds
    let cases
    if (set === 'commit') {
      const selection = commitSetCases(ids, { overrides })
      ;({ ids, cases } = selection)
      omitted = { stories: selection.omittedStories, cases: selection.omittedCases }
    } else {
      cases = fullCases(ids, overrides)
      omitted = { stories: ids.filter(id => overrides.get(id)?.length === 0), cases: ids.length * 4 - cases.length }
    }
    expectedCases = cases.length
    // Record CPU and proc/stat deltas, including steal ticks, around the play phase.
    cpuStarted = await captureKitCpu()
    loadStarted = loadavg()
    playStarted = performance.now()
    const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
    browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
    let nextCase = 0
    const runCase = async (worker, workerId, previousCase, { id, scheme, width }) => {
      const caseStarted = performance.now()
      const row = {
        id, storyFile: storyFiles[id], scheme, width, height: 900, workerId, previousCase, pass: false, timings: {},
      }
      let page, guard, domLoaded
      try {
        const { resetMilliseconds } = await worker.reset()
        row.timings.resetMilliseconds = resetMilliseconds
        const createStarted = performance.now()
        page = await worker.newPage({ viewport: { width, height: 900 }, colorScheme: scheme })
        const pageCreated = performance.now()
        guard = await strictStoryGuard(page, base)
        const guardReady = performance.now()
        Object.assign(row.timings, {
          pageCreateMilliseconds: pageCreated - createStarted,
          guardMilliseconds: guardReady - pageCreated,
        })
        page.once('domcontentloaded', () => { domLoaded = performance.now() })
        // The guard awaits strict storyFinished; unrelated network idleness is not play readiness.
        // a11y.manual is addon-a11y's own switch for its afterEach axe run.
        await page.goto(`${base}/iframe.html?id=${encodeURIComponent(id)}&viewMode=story&args=${encodeURIComponent(`scheme:${scheme}`)}${a11y === 'off' ? `&globals=${encodeURIComponent('a11y.manual:!true')}` : ''}`, { waitUntil: 'domcontentloaded' })
        const storyReady = performance.now()
        row.timings.navigationToDomMilliseconds = domLoaded - guardReady
        row.timings.readinessAfterDomMilliseconds = storyReady - domLoaded
        await guard.check()
        const timing = guard.records.find(record => record.timing?.finished?.storyId === id)?.timing
        if (timing) {
          const phase = name => timing.phases.find(mark => mark.storyId === id && mark.phase === name)?.at
          const a11yMark = name => timing.a11y.find(mark => mark.storyId === id && mark.phase === name)?.at
          Object.assign(row.timings, readinessTimings({
            dom: timing.dom, playing: phase('playing'), played: phase('played'),
            completing: phase('completing'), finished: timing.finished.at, observed: timing.observed,
            a11yStart: a11yMark('start'), a11yEnd: a11yMark('end'),
          }, a11y === 'on'))
        }
        assertStrictEvidence(guard.records, id, a11y)
        row.timings.assertionsMilliseconds = performance.now() - storyReady
        row.pass = true
      } catch (error) {
        row.error = String(error)
        if (error.isolation) row.isolation = error.isolation
        row.events = await page?.evaluate(() => window.__dt10StrictEvents ?? []).catch(() => []) ?? []
        if (!page) throw error
      } finally {
        row.strict = guard?.records ?? []
        const probes = await page?.evaluate(() => window.__kitFilterBenchmark ?? []).catch(() => []) ?? []
        if (probes.length) row.filterBenchmark = probes
        // Keep the original elapsed definition (before page.close) for comparison.
        row.elapsedSeconds = (performance.now() - caseStarted) / 1000
        const closeStarted = performance.now()
        try {
          await page?.close()
        } catch (error) {
          row.pass = false
          row.error = `${row.error ?? ''} page cleanup: ${error}`
          throw error
        } finally {
          row.timings.closeMilliseconds = performance.now() - closeStarted
          rows.push(row)
          console.log(JSON.stringify(row))
        }
      }
    }
    const results = await Promise.allSettled(Array.from({ length: workers }, async (_, workerId) => {
      const worker = await createIsolatedKitWorker(browser, base, { workerId })
      let previousCase = null
      try {
        while (nextCase < cases.length) {
          const currentCase = cases[nextCase++]
          await runCase(worker, workerId, previousCase, currentCase)
          previousCase = currentCase
          worker.setPreviousCase(previousCase)
        }
      } finally {
        await worker.close()
      }
    }))
    const rejected = results.filter(result => result.status === 'rejected')
    if (rejected.length) throw new AggregateError(rejected.map(result => result.reason), 'Kit play workers failed')
  } catch (error) {
    fatalError = String(error)
  } finally {
    await browser?.close()
    await new Promise(resolve => server.close(resolve))
    elapsedSeconds = (performance.now() - playStarted) / 1000
    cpuDelta = kitCpuDelta(cpuStarted, await captureKitCpu())
    loadEnded = loadavg()
  }
  const receipt = await writeReceipt()
  // The complete receipt stays in CI logs even when the gate fails.
  console.log(JSON.stringify(receipt))
  return receipt
}
