import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { parseWebShard, selectKitCases } from './kit-play-shard.mjs'

const ids = ['kit--alpha', 'kit--beta', 'kit--unicode-λ', 'kit--tuple,separator']
const keys = cases => cases.map(({ id, scheme, width }) => JSON.stringify([id, scheme, width])).sort()

test('unset shard preserves every story, scheme and width in the original order', () => {
  assert.equal(parseWebShard(undefined), null)
  assert.deepEqual(selectKitCases(['kit--alpha'], null), [
    { id: 'kit--alpha', scheme: 'dark', width: 440 },
    { id: 'kit--alpha', scheme: 'dark', width: 1440 },
    { id: 'kit--alpha', scheme: 'light', width: 440 },
    { id: 'kit--alpha', scheme: 'light', width: 1440 },
  ])
  assert.deepEqual(selectKitCases(ids, parseWebShard('1/1')), selectKitCases(ids, null))
})

test('shards form complete nonoverlapping partitions, including empty shards', () => {
  const all = keys(selectKitCases(ids, null))
  for (const total of [1, 2, 3, 7, 31]) {
    const partitions = Array.from({ length: total }, (_, index) =>
      selectKitCases(ids, parseWebShard(`${index + 1}/${total}`)))
    const combined = partitions.flat()
    assert.equal(new Set(keys(combined)).size, all.length)
    assert.deepEqual(keys(combined), all)
    if (total > all.length) assert.ok(partitions.some(partition => partition.length === 0))
  }
})

test('case ownership is deterministic and independent of index ordering or other stories', () => {
  for (let index = 1; index <= 7; index++) {
    const shard = parseWebShard(`${index}/7`)
    const selected = selectKitCases(ids, shard)
    assert.deepEqual(selectKitCases(ids, shard), selected)
    assert.deepEqual(keys(selectKitCases([...ids].reverse(), shard)), keys(selected))
    assert.deepEqual(selectKitCases([...ids, 'kit--new'], shard).filter(row => row.id !== 'kit--new'), selected)
    assert.deepEqual(selectKitCases([ids[0]], shard), selected.filter(row => row.id === ids[0]))
  }
})

test('rejects invalid shard values rather than falling back to an unsharded run', () => {
  for (const value of [
    '', ' ', '0/1', '1/0', '2/1', '-1/2', '1/-2', '1.5/2', '1/2.5',
    '01/2', '1/02', '1', '1/2/3', ' 1/2', '1/2 ', '1/2\n', '+1/2',
    'NaN/2', '1/Infinity', '9007199254740992/9007199254740992',
    '1/9007199254740992', null, 1, {},
  ]) assert.throws(() => parseWebShard(value), /Invalid FRACTAL_WEB_SHARD/)
  assert.deepEqual(parseWebShard('2/3'), { index: 2, total: 3 })
  assert.deepEqual(parseWebShard('1/9007199254740991'), { index: 1, total: Number.MAX_SAFE_INTEGER })
})

const runGate = async shard => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-shard-test-'))
  try {
    const build = join(directory, 'build'), output = join(directory, 'receipt')
    await mkdir(build)
    await writeFile(join(build, 'index.json'), JSON.stringify({ entries: {
      sample: { type: 'story', id: ids[0], importPath: './Sample.stories.tsx' },
    } }))
    const child = spawn(process.execPath, [fileURLToPath(new URL('./kit-play-gate.mjs', import.meta.url)), build, output], {
      env: { ...process.env, FRACTAL_WEB_SHARD: shard, PLAYWRIGHT_MODULE: 'must-not-import-browser-for-empty-or-invalid-shard' },
      stdio: 'ignore',
    })
    const code = await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', resolve)
    })
    return { code, receipt: JSON.parse(await readFile(join(output, 'receipt.json'), 'utf8')) }
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
}

test('empty shard receipt passes using selected count instead of all stories times four', async () => {
  const index = Array.from({ length: 5 }, (_, i) => i + 1).find(index =>
    selectKitCases([ids[0]], parseWebShard(`${index}/5`)).length === 0)
  assert.ok(index)
  const { code, receipt } = await runGate(`${index}/5`)
  assert.equal(code, 0)
  assert.equal(receipt.pass, true)
  assert.equal(receipt.stories, 1)
  assert.equal(receipt.totalCases, 4)
  assert.equal(receipt.expectedCases, 0)
  assert.equal(receipt.cases, 0)
  assert.deepEqual(receipt.shard, { index, total: 5, expectedCases: 0, actualCases: 0 })
  assert.deepEqual(receipt.rows, [])
})

test('invalid shard fails the actual gate with a receipt before importing a browser', async () => {
  const { code, receipt } = await runGate('0/2')
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  assert.equal(receipt.cases, 0)
  assert.match(receipt.fatalError, /Invalid FRACTAL_WEB_SHARD/)
  assert.doesNotMatch(receipt.fatalError, /must-not-import-browser/)
})

test('nonempty shard receipt retains its selected expectation when browser startup fails', async () => {
  const index = Array.from({ length: 5 }, (_, i) => i + 1).find(index =>
    selectKitCases([ids[0]], parseWebShard(`${index}/5`)).length > 0)
  const selected = selectKitCases([ids[0]], parseWebShard(`${index}/5`))
  const { code, receipt } = await runGate(`${index}/5`)
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  assert.equal(receipt.totalCases, 4)
  assert.equal(receipt.expectedCases, selected.length)
  assert.equal(receipt.cases, 0)
  assert.deepEqual(receipt.shard, { index, total: 5, expectedCases: selected.length, actualCases: 0 })
  assert.match(receipt.fatalError, /must-not-import-browser/)
})
