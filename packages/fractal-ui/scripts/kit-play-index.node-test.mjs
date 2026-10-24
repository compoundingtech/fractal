import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { storyIdsFromIndex } from './kit-play-index.mjs'
import { commitSetCases, diagonalCase, fullCases, BOTH_WIDTHS, MERGE_ONLY, FULL_CASE_OVERRIDES, validateCaseOverrides } from './kit-play-commit-set.mjs'
import { aggregateShards, shardStories } from './kit-play-shards.mjs'

const story = { type: 'story', id: 'kit--sample', importPath: './Sample.stories.tsx' }
const docs = { type: 'docs', id: 'kit--docs', importPath: './Sample.mdx' }
test('validates all entries before selecting stories', () => {
  assert.deepEqual(storyIdsFromIndex({ entries: { story, docs } }), ['kit--sample'])
  for (const malformed of [
    null, [], {}, { ...story, type: undefined }, { ...story, type: 'unknown' },
    { ...story, id: undefined }, { ...story, id: ' ' }, { ...story, id: 7 },
    { ...story, importPath: undefined }, { ...story, importPath: '' },
    { ...docs, importPath: undefined }, { ...docs, id: '' },
  ]) {
    assert.throws(() => storyIdsFromIndex({ entries: { story, malformed } }), /Invalid Storybook index entry/)
  }
})
test('selects a tagged set from the validated index and fails on an empty one', () => {
  const critical = { ...story, id: 'kit--critical', tags: ['dev', 'critical'] }
  const other = { ...story, id: 'kit--other', tags: ['dev'] }
  const entries = { critical, other, story, docs }
  assert.deepEqual(storyIdsFromIndex({ entries }), ['kit--critical', 'kit--other', 'kit--sample'])
  assert.deepEqual(storyIdsFromIndex({ entries }, { tag: 'critical' }), ['kit--critical'])
  assert.throws(() => storyIdsFromIndex({ entries: { other, story } }, { tag: 'critical' }), /no stories tagged "critical"/)
  for (const tags of ['critical', [''], [7], null]) {
    assert.throws(() => storyIdsFromIndex({ entries: { critical, malformed: { ...other, tags } } }, { tag: 'critical' }), /tags must be an array/)
  }
})
test('rejects malformed containers, empty story sets and duplicate IDs', () => {
  for (const index of [null, [], {}, { entries: [] }, { entries: null }]) {
    assert.throws(() => storyIdsFromIndex(index), /entries object/)
  }
  for (const entries of [{}, { docs }, { first: story, second: { ...story } }]) {
    assert.throws(() => storyIdsFromIndex({ entries }), /unique, nonempty story IDs/)
  }
})
test('malformed built index fails the actual gate before browser execution', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-index-test-'))
  try {
    const build = join(directory, 'build'), output = join(directory, 'receipt')
    await mkdir(build)
    await writeFile(join(build, 'index.json'), JSON.stringify({ entries: {
      story,
      malformed: { id: 'kit--silently-dropped-before', importPath: './Broken.stories.tsx' },
    } }))
    const child = spawn(process.execPath, [fileURLToPath(new URL('./kit-play-gate.mjs', import.meta.url)), build, output], {
      env: { ...process.env, PLAYWRIGHT_MODULE: 'must-not-import-browser-for-invalid-index' },
      stdio: 'ignore',
    })
    const code = await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', resolve)
    })
    assert.equal(code, 1)
    const receipt = JSON.parse(await readFile(join(output, 'receipt.json'), 'utf8'))
    assert.equal(receipt.pass, false)
    assert.equal(receipt.cases, 0)
    assert.match(receipt.fatalError, /Invalid Storybook index entry "malformed"/)
    assert.doesNotMatch(receipt.fatalError, /must-not-import-browser/)
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
test('the critical set fails the actual gate before browser execution when nothing is tagged', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-index-test-'))
  try {
    const build = join(directory, 'build'), output = join(directory, 'receipt')
    await mkdir(build)
    await writeFile(join(build, 'index.json'), JSON.stringify({ entries: { story: { ...story, tags: ['dev'] } } }))
    const child = spawn(process.execPath, [fileURLToPath(new URL('./kit-play-gate.mjs', import.meta.url)), build, output], {
      env: { ...process.env, KIT_PLAY_SET: 'critical', PLAYWRIGHT_MODULE: 'must-not-import-browser-for-empty-set' },
      stdio: 'ignore',
    })
    const code = await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', resolve)
    })
    assert.equal(code, 1)
    const receipt = JSON.parse(await readFile(join(output, 'receipt.json'), 'utf8'))
    assert.equal(receipt.pass, false)
    assert.equal(receipt.set, 'critical')
    assert.match(receipt.fatalError, /no stories tagged "critical"/)
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
test('the per-commit set runs one diagonal case per story, both widths for the layout stories, and omits merge-only stories', () => {
  const ids = ['a--one', 'a--three', 'b--wide', 'c--heavy']
  const { ids: kept, cases, omittedStories, omittedCases } = commitSetCases(ids, { mergeOnlyIds: ['c--heavy'], bothWidthsIds: ['b--wide'] })
  assert.deepEqual(kept, ['a--one', 'a--three', 'b--wide'])
  assert.deepEqual(omittedStories, ['c--heavy'])
  assert.equal(cases.length, 1 + 1 + 2)
  assert.deepEqual(cases.filter(({ id }) => id === 'b--wide').map(({ scheme, width }) => `${scheme}${width}`).sort(), ['dark1440', 'dark440'])
  assert.equal(omittedCases, ids.length * 4 - cases.length)
  for (const { id, scheme, width } of cases.filter(({ id }) => id !== 'b--wide')) assert.deepEqual([scheme, width], diagonalCase(id))
  // Both diagonals are used, so both schemes and both widths stay covered across a set.
  assert.deepEqual(new Set(['a--one', 'a--three'].map(id => diagonalCase(id).join())), new Set(['dark,440', 'light,1440']))
  assert.equal(fullCases(['x']).length, 4)
})
test('the per-commit set rejects a listed story that is not critical, so a rename cannot drop coverage', () => {
  assert.throws(() => commitSetCases(['a--one'], { mergeOnlyIds: ['a--renamed'], bothWidthsIds: [] }), /a--renamed, which is not a critical story/)
  assert.throws(() => commitSetCases(['a--one'], { mergeOnlyIds: [], bothWidthsIds: ['a--gone'] }), /a--gone, which is not a critical story/)
  assert.equal(new Set([...MERGE_ONLY, ...BOTH_WIDTHS]).size, MERGE_ONLY.length + BOTH_WIDTHS.length, 'a story is merge-only or both-widths, not both')
})

const benchmarkId = 'fractal-ui-sidebar-tree--filter-keystroke-to-paint'
const functionalId = 'fractal-ui-sidebar-tree--filter-behavior'
const freezeId = 'fractal-narrow-layout--sidebar-drag-unmount-cleanup'
const aliasIds = [
  'fractal-ui-transcript--no-green-all-states-light',
  'fractal-ui-composer--footer-width-budget-light',
]
const baseIds = [
  'fractal-ui-transcript--no-green-all-states',
  'fractal-ui-composer--footer-width-budget',
]
test('the strict case set measures the filter once, keeps functional corners, and omits only the scheme aliases', () => {
  assert.deepEqual([...FULL_CASE_OVERRIDES], [
    ['fractal-ui-transcript--no-green-all-states-light', []],
    ['fractal-ui-composer--footer-width-budget-light', []],
    ['fractal-ui-sidebar-tree--filter-keystroke-to-paint', [['dark', 1440]]],
    [freezeId, [['dark', 1100], ['light', 1100]]],
  ])
  const omittedBenchmarkCases = [
    { id: 'fractal-ui-sidebar-tree--filter-keystroke-to-paint', scheme: 'dark', width: 440 },
    { id: 'fractal-ui-sidebar-tree--filter-keystroke-to-paint', scheme: 'light', width: 440 },
    { id: 'fractal-ui-sidebar-tree--filter-keystroke-to-paint', scheme: 'light', width: 1440 },
  ]
  const ids = [benchmarkId, functionalId, ...aliasIds]
  for (const omitted of omittedBenchmarkCases) assert.ok(!fullCases(ids).some(row => row.id === omitted.id && row.scheme === omitted.scheme && row.width === omitted.width))
  for (const id of baseIds) {
    assert.deepEqual(fullCases([id]), [
      { id, scheme: 'dark', width: 440 }, { id, scheme: 'dark', width: 1440 },
      { id, scheme: 'light', width: 440 }, { id, scheme: 'light', width: 1440 },
    ])
  }
  assert.deepEqual(fullCases([benchmarkId]), [{ id: benchmarkId, scheme: 'dark', width: 1440 }])
  assert.equal(fullCases([functionalId]).length, 4)
  assert.deepEqual(fullCases(aliasIds), [])
  assert.equal(fullCases(ids).length, 5)
  const commit = commitSetCases([functionalId, ...aliasIds], { mergeOnlyIds: [], bothWidthsIds: [] })
  assert.deepEqual(commit.ids, [functionalId])
  assert.deepEqual(commit.omittedStories, aliasIds)
})
test('the intermediate-width resize regression runs at its declared real viewport in both gate sets', () => {
  const expected = [{ id: freezeId, scheme: 'dark', width: 1100 }, { id: freezeId, scheme: 'light', width: 1100 }]
  assert.deepEqual(fullCases([freezeId]), expected)
  assert.deepEqual(commitSetCases([freezeId], { mergeOnlyIds: [], bothWidthsIds: [] }).cases, expected)
})
test('strict case policy rejects unknown stories and aliases without a kept same-source base', () => {
  const ids = [benchmarkId, functionalId, freezeId, ...aliasIds, ...baseIds]
  const entries = Object.fromEntries(ids.map(id => [id, { ...story, id }]))
  assert.doesNotThrow(() => validateCaseOverrides(entries))
  assert.throws(() => validateCaseOverrides(entries, new Map([['kit--unknown', []]])), /not a known story/)
  assert.throws(() => validateCaseOverrides(entries, new Map([[benchmarkId, []]])), /not a scheme alias/)
  const withoutBase = { ...entries }
  delete withoutBase[baseIds[0]]
  assert.throws(() => validateCaseOverrides(withoutBase), /not a scheme alias/)
  const differentSource = { ...entries, [baseIds[0]]: { ...entries[baseIds[0]], importPath: './Other.stories.tsx' } }
  assert.throws(() => validateCaseOverrides(differentSource), /not a scheme alias/)
  assert.throws(() => validateCaseOverrides(entries, new Map([[aliasIds[0], []], [baseIds[0], []]])), /not a scheme alias/)
})
test('the actual gate rejects stale case policy before browser execution', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-policy-test-'))
  try {
    const build = join(directory, 'build'), output = join(directory, 'receipt')
    await mkdir(build)
    await writeFile(join(build, 'index.json'), JSON.stringify({ entries: { story } }))
    const child = spawn(process.execPath, [fileURLToPath(new URL('./kit-play-gate.mjs', import.meta.url)), build, output], {
      env: { ...process.env, KIT_PLAY_SET: 'all', PLAYWRIGHT_MODULE: 'must-not-import-browser-for-stale-policy' },
      stdio: 'ignore',
    })
    const code = await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', resolve)
    })
    assert.equal(code, 1)
    const receipt = JSON.parse(await readFile(join(output, 'receipt.json'), 'utf8'))
    assert.equal(receipt.pass, false)
    assert.equal(receipt.cases, 0)
    assert.match(receipt.fatalError, /strict case policy names .*not a known story/)
    assert.doesNotMatch(receipt.fatalError, /must-not-import-browser/)
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
test('the production gate fails closed when the policy environment variable is set, even empty', async () => {
  for (const value of ['[]', '']) {
    const child = spawn(process.execPath, [fileURLToPath(new URL('./kit-play-gate.mjs', import.meta.url)), 'unused-build', 'unused-output'], {
      env: { ...process.env, KIT_PLAY_CASE_OVERRIDES: value, PLAYWRIGHT_MODULE: 'must-not-import-browser-for-policy-env' },
      stdio: ['ignore', 'ignore', 'pipe'],
    })
    let stderr = ''
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk })
    const code = await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', resolve)
    })
    assert.equal(code, 1)
    assert.match(stderr, /KIT_PLAY_CASE_OVERRIDES is not supported: production kit plays must use the reviewed source policy/)
    assert.doesNotMatch(stderr, /must-not-import-browser-for-policy-env/)
  }
})
test('shard coverage requires the exact strict case set, including the benchmark corner', () => {
  const fullIds = [benchmarkId, functionalId, ...aliasIds].sort()
  const provenance = { runId: '1234', runAttempt: 1, headSha: 'a'.repeat(40) }
  const receipts = [0, 1].map(shardIndex => ({
    ...provenance, pass: true, fullIds, shardIndex, shardCount: 2,
    rows: fullCases(shardStories(fullIds, shardIndex, 2)).map(row => ({ ...row, pass: true })),
  }))
  assert.equal(aggregateShards(receipts, 2, provenance).cases, 5)
  const wrongCorner = structuredClone(receipts)
  wrongCorner.flatMap(receipt => receipt.rows).find(row => row.id === benchmarkId).width = 440
  assert.throws(() => aggregateShards(wrongCorner, 2, provenance), /invalid play case/)
  const missing = structuredClone(receipts)
  const benchmarkShard = missing.find(receipt => receipt.rows.some(row => row.id === benchmarkId))
  benchmarkShard.rows = benchmarkShard.rows.filter(row => row.id !== benchmarkId)
  assert.throws(() => aggregateShards(missing, 2, provenance), /Incomplete shard cases/)
})
test('the gate receipt declares the intended benchmark corner and alias omissions before browser execution', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-cases-test-'))
  try {
    const build = join(directory, 'build'), output = join(directory, 'receipt')
    await mkdir(build)
    const ids = [benchmarkId, functionalId, freezeId, ...aliasIds, ...baseIds]
    await writeFile(join(build, 'index.json'), JSON.stringify({ entries: Object.fromEntries(ids.map(id => [id, { ...story, id }])) }))
    const child = spawn(process.execPath, [fileURLToPath(new URL('./kit-play-gate.mjs', import.meta.url)), build, output], {
      env: { ...process.env, KIT_PLAY_SET: 'all', PLAYWRIGHT_MODULE: 'must-not-import-browser-for-case-contract' },
      stdio: 'ignore',
    })
    const code = await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', resolve)
    })
    assert.equal(code, 1)
    const receipt = JSON.parse(await readFile(join(output, 'receipt.json'), 'utf8'))
    assert.equal(receipt.expectedCases, 15)
    assert.equal(receipt.omitted.cases, 13)
    assert.deepEqual(receipt.omitted.stories, aliasIds.toSorted())
    assert.deepEqual(receipt.caseOverrides[benchmarkId], [['dark', 1440]])
    assert.match(receipt.fatalError, /must-not-import-browser-for-case-contract/)
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
