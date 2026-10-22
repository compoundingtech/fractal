import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { readinessTimings, summarizeTimings, costTable, notice } from './kit-play-timings.mjs'
import { shardArtifactName, shardStories } from './kit-play-shards.mjs'
import { fullCases } from './kit-play-commit-set.mjs'
import { fileURLToPath } from 'node:url'
import config from '../.storybook/main.ts'

test('Storybook 10.6.1 a11y annotation path and accepted entry shapes stay pinned', async () => {
  for (const name of ['storybook', '@storybook/addon-a11y']) {
    const pkg = JSON.parse(await readFile(fileURLToPath(import.meta.resolve(`${name}/package.json`)), 'utf8'))
    assert.equal(pkg.version, '10.6.1', 'Revalidate annotation replacement when updating Storybook')
  }
  const original = fileURLToPath(import.meta.resolve('@storybook/addon-a11y/preview'))
  assert.ok(original.endsWith('/@storybook/addon-a11y/dist/preview.js'), 'Revalidate the addon preview entry path')
  const wrapper = fileURLToPath(new URL('../.storybook/a11y-timing.mjs', import.meta.url))
  assert.deepEqual(await config.previewAnnotations([original]), [wrapper])
  const unrelated = { bare: 'unchanged', absolute: '' }
  const result = await config.previewAnnotations([unrelated, { bare: '@storybook/addon-a11y/preview', absolute: original }])
  assert.equal(result[0], unrelated)
  assert.equal(result[1], wrapper)
  await assert.rejects(config.previewAnnotations([]), /Missing pinned a11y preview annotation/)
})

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
  storyFile: `./${index % 2 ? 'a' : 'b'}.stories.tsx`,
  elapsedSeconds: index + 1, pass: true, timings: { playMilliseconds: (index + 1) * 10 },
}))
test('summary reports nearest-rank p95, mean, deterministic top 20 and family totals', () => {
  const result = summarizeTimings(rows)
  assert.deepEqual(result.phaseMilliseconds.playMilliseconds, { samples: 21, mean: 110, p95: 200 })
  assert.deepEqual(result.phaseMeanMilliseconds, { playMilliseconds: 110 })
  assert.equal(result.topCases.length, 20)
  assert.equal(result.topCases[0].elapsedSeconds, 21)
  assert.equal(result.topCases.at(-1).elapsedSeconds, 2)
  assert.deepEqual(result.storyFamilies, [
    { family: 'family-b', cases: 11, elapsedSeconds: 121 },
    { family: 'family-a', cases: 10, elapsedSeconds: 110 },
  ])
  assert.deepEqual(result.storyFiles, [
    { storyFile: './b.stories.tsx', cases: 11, elapsedSeconds: 121 },
    { storyFile: './a.stories.tsx', cases: 10, elapsedSeconds: 110 },
  ])
  assert.deepEqual(summarizeTimings([]), { phaseMilliseconds: {}, phaseMeanMilliseconds: {}, topCases: [], storyFamilies: [], storyFiles: [] })
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
    await writeFile(join(artifacts, `kit-play-123-${sha}-shard-0-attempt-1`, 'receipt.json'), '{"rows":')
    const run = spawnSync(process.execPath, [new URL('./kit-play-cost.mjs', import.meta.url).pathname, artifacts], {
      encoding: 'utf8', env: { ...process.env, GITHUB_RUN_ID: '123', GITHUB_SHA: sha, GITHUB_RUN_ATTEMPT: '2', GITHUB_STEP_SUMMARY: summary },
    })
    assert.equal(run.status, 0, run.stderr)
    const table = await readFile(summary, 'utf8')
    assert.match(table, /\| 0 \| 42.00 \| 3.00 \| 2.00 \| 1 \|/)
    assert.match(table, /\| 1 \| 37.00 \| unavailable \| unavailable \| not run \|/)
    assert.doesNotMatch(table, /999/)
    assert.equal(run.stdout.trimEnd(), notice(table))
    await writeFile(join(artifacts, `kit-play-123-${sha}-shard-0-attempt-2`, 'receipt.json'), '{"rows":')
    const malformed = spawnSync(process.execPath, [new URL('./kit-play-cost.mjs', import.meta.url).pathname, artifacts], {
      encoding: 'utf8', env: { ...process.env, GITHUB_RUN_ID: '123', GITHUB_SHA: sha, GITHUB_RUN_ATTEMPT: '2', GITHUB_STEP_SUMMARY: '' },
    })
    assert.notEqual(malformed.status, 0, 'A malformed newest receipt must not fall back to an older attempt')
    assert.match(malformed.stderr, /SyntaxError/)
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

test('shard CLI selects attempts before requiring receipts, without weakening provenance or coverage', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-shard-attempt-test-'))
  try {
    const provenance = { runId: '123', headSha: 'a'.repeat(40), runAttempt: 2 }
    const ids = Array.from({ length: 32 }, (_, index) => `story-${index}`).sort()
    const receipt = shardIndex => ({
      ...provenance, runAttempt: shardIndex === 0 ? 2 : 1,
      shardIndex, shardCount: 2, pass: true, fullIds: ids,
      rows: fullCases(shardStories(ids, shardIndex, 2)).map(row => ({ ...row, pass: true })),
    })
    const save = async (identity, content) => {
      const path = join(directory, shardArtifactName(identity))
      await mkdir(path)
      await writeFile(join(path, 'lane-seconds'), '42')
      if (content) await writeFile(join(path, 'receipt.json'), JSON.stringify(content))
      return path
    }
    await save({ ...receipt(0), runAttempt: 1 })
    const latest = await save(receipt(0), receipt(0))
    await save(receipt(1), receipt(1))
    const run = (attempt = 2) => spawnSync(process.execPath, [new URL('./kit-play-shards.mjs', import.meta.url).pathname, directory, '2'], {
      encoding: 'utf8', env: { ...process.env, GITHUB_RUN_ID: provenance.runId, GITHUB_SHA: provenance.headSha, GITHUB_RUN_ATTEMPT: String(attempt) },
    })
    const passed = run()
    assert.equal(passed.status, 0, passed.stderr)
    assert.deepEqual(JSON.parse(passed.stdout), { stories: ids.length, cases: fullCases(ids).length, shards: 2, pass: true })

    // The public-cost publisher also accepts the retained lane-only older attempt.
    const cost = spawnSync(process.execPath, [new URL('./kit-play-cost.mjs', import.meta.url).pathname, directory], {
      encoding: 'utf8', env: { ...process.env, GITHUB_RUN_ID: provenance.runId, GITHUB_SHA: provenance.headSha, GITHUB_RUN_ATTEMPT: '2', GITHUB_STEP_SUMMARY: '' },
    })
    assert.equal(cost.status, 0, cost.stderr)
    assert.ok(cost.stdout.includes('| 0 | 42.00'))

    const corrupt = { ...receipt(0), headSha: 'b'.repeat(40) }
    await writeFile(join(latest, 'receipt.json'), JSON.stringify(corrupt))
    assert.match(run().stderr, /Artifact name does not match receipt provenance/)
    await writeFile(join(latest, 'receipt.json'), JSON.stringify({ ...receipt(0), rows: [] }))
    assert.match(run().stderr, /Incomplete shard cases/)
    await writeFile(join(latest, 'receipt.json'), JSON.stringify(receipt(0)))

    const missing = await save({ ...receipt(0), runAttempt: 3 })
    assert.match(run(3).stderr, /ENOENT/)
    await rm(missing, { recursive: true })
    const foreign = await save({ ...receipt(0), runId: '999', runAttempt: 1 })
    assert.match(run().stderr, /Shard workflow provenance mismatch/)
    await rm(foreign, { recursive: true })
    assert.match(run(1).stderr, /Shard workflow provenance mismatch/)
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
