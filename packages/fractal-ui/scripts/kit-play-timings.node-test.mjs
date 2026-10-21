import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { readinessTimings, summarizeTimings, costTable, notice } from './kit-play-timings.mjs'

const marks = { dom: 10, playing: 30, played: 70, completing: 75, a11yStart: 80, a11yEnd: 150, finished: 160, observed: 170 }
test('readiness phases partition one document clock, including lifecycle overhead', () => {
  assert.deepEqual(readinessTimings(marks, true), {
    readinessAfterDomMilliseconds: 160, bootstrapMilliseconds: 20, playMilliseconds: 40,
    a11yMilliseconds: 70, lifecycleMilliseconds: 20, completionObservationMilliseconds: 10,
  })
  assert.equal(readinessTimings({ ...marks, playing: undefined, played: undefined }, true).playMilliseconds, 0)
  assert.equal(readinessTimings({ ...marks, playing: 2, played: 5 }, true).playMilliseconds, 0)
  assert.equal(readinessTimings(marks, false).a11yMilliseconds, 0)
  for (const invalid of [undefined, {}, { ...marks, finished: 50 }, { ...marks, observed: 100 }, { ...marks, a11yStart: undefined }]) {
    assert.deepEqual(readinessTimings(invalid, true), {})
  }
})

const rows = Array.from({ length: 21 }, (_, index) => ({
  id: `${index % 2 ? 'family-a' : 'family-b'}--story-${index}`, scheme: 'dark', width: 440,
  elapsedSeconds: index + 1, pass: true, timings: { playMilliseconds: (index + 1) * 10 },
}))
test('summary reports nearest-rank p95, mean, deterministic top 20 and family totals', () => {
  const result = summarizeTimings(rows)
  assert.deepEqual(result.phaseMilliseconds.playMilliseconds, { samples: 21, mean: 110, p95: 200 })
  assert.equal(result.topCases.length, 20)
  assert.equal(result.topCases[0].elapsedSeconds, 21)
  assert.equal(result.topCases.at(-1).elapsedSeconds, 2)
  assert.deepEqual(result.storyFamilies, [
    { family: 'family-b', cases: 11, elapsedSeconds: 121 },
    { family: 'family-a', cases: 10, elapsedSeconds: 110 },
  ])
  assert.deepEqual(summarizeTimings([]), { phaseMilliseconds: {}, topCases: [], storyFamilies: [] })
  assert.deepEqual(summarizeTimings([{ ...rows[0], timings: { a: NaN, b: Infinity, c: -1 } }]).phaseMilliseconds, {})
})

test('public table recomputes weighted means from all cases, limits top cases and escapes notices', () => {
  const table = costTable([{ shardIndex: 0, laneSeconds: 10, receipt: { elapsedSeconds: 5, storybookBuildSeconds: 2, cases: 20, rows: rows.slice(0, 20) } },
    { shardIndex: 1, laneSeconds: 11, receipt: { elapsedSeconds: 6, storybookBuildSeconds: 3, cases: 1, rows: rows.slice(20) } }])
  assert.match(table, /\| 0 \| 10.00 \| 5.00 \| 2.00 \| 20 \|/)
  assert.match(table, /\| play \| 110.00 \| 200.00 \| 21 \|/)
  assert.equal(table.split('\n').filter(line => line.startsWith('| family-')).length, 10)
  assert.match(costTable([{ shardIndex: 0 }]), /unavailable/)
  assert.equal(notice('100%\r\nx'), '::notice title=Fractal web cost::100%25%0D%0Ax')
})

test('publisher selects the newest attempt, handles no-kit lanes and writes the same public and job summary', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-cost-test-'))
  try {
    const sha = 'a'.repeat(40), summary = join(directory, 'summary'), artifacts = join(directory, 'artifacts')
    await mkdir(artifacts)
    for (const [shard, attempt, lane, receipt] of [[0, 1, 999, { rows }], [0, 2, 42, { rows: rows.slice(0, 1), cases: 1, elapsedSeconds: 3, storybookBuildSeconds: 2 }], [1, 1, 37, undefined]]) {
      const path = join(artifacts, `kit-play-123-${sha}-shard-${shard}-attempt-${attempt}`)
      await mkdir(path)
      await writeFile(join(path, 'lane-seconds'), String(lane))
      if (receipt) await writeFile(join(path, 'receipt.json'), JSON.stringify(receipt))
    }
    const run = spawnSync(process.execPath, [new URL('./kit-play-cost.mjs', import.meta.url).pathname, artifacts], {
      encoding: 'utf8', env: { ...process.env, GITHUB_RUN_ID: '123', GITHUB_SHA: sha, GITHUB_RUN_ATTEMPT: '2', GITHUB_STEP_SUMMARY: summary },
    })
    assert.equal(run.status, 0, run.stderr)
    const table = await readFile(summary, 'utf8')
    assert.match(table, /\| 0 \| 42.00 \| 3.00 \| 2.00 \| 1 \|/)
    assert.match(table, /\| 1 \| 37.00 \| unavailable \| unavailable \| not run \|/)
    assert.doesNotMatch(table, /999/)
    assert.equal(run.stdout.trimEnd(), notice(table))
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})

test('public benchmark margin uses the worst probe and does not invent unobserved values', () => {
  const table = costTable([{ shardIndex: 0, receipt: { rows: [{ ...rows[0], filterBenchmark: [
    { pooledRatio: 1.5, threshold: 3 }, { pooledRatio: 2, threshold: 3 },
  ] }] } }])
  assert.match(table, /minimum margin: 1.500× .*2 probes/)
  assert.match(costTable([]), /minimum margin: unavailable/)
})
