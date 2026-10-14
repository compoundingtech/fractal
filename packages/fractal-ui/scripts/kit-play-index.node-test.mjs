import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { storyIdsFromIndex } from './kit-play-index.mjs'

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
