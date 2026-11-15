import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { markdownWasmInputs } from './markdown-wasm-inputs.mjs'

const composition = new URL('../src/assistant-ui/composition/', import.meta.url)
const root = fileURLToPath(new URL('../../../', import.meta.url))
const regenerate = 'regenerate with node packages/fractal-ui/scripts/generate-markdown-wasm.mjs'

async function assertCurrentInputs(tree, manifest) {
  assert.deepEqual(await markdownWasmInputs(tree), manifest.inputs, `Markdown wasm build inputs changed; ${regenerate}`)
}

test('the committed pulldown-cmark module records the current build inputs', async () => {
  const manifest = JSON.parse(await readFile(new URL('markdown-pulldown.generated.json', composition), 'utf8'))
  await assertCurrentInputs(root, manifest)
})

test('one changed source byte rejects a stale wasm manifest without changing the tree', async () => {
  const manifest = JSON.parse(await readFile(new URL('markdown-pulldown.generated.json', composition), 'utf8'))
  const parent = resolve(root, 'tmp/markdown-wasm-inputs')
  await mkdir(parent, { recursive: true })
  const fixture = await mkdtemp(resolve(parent, 'negative-'))
  try {
    for (const path of (await markdownWasmInputs(root)).files) {
      const destination = resolve(fixture, path)
      await mkdir(dirname(destination), { recursive: true })
      await copyFile(resolve(root, path), destination)
    }
    await assertCurrentInputs(fixture, manifest)
    const source = resolve(fixture, 'packages/fractal-ui/markdown-wasm/src/lib.rs')
    const bytes = await readFile(source)
    bytes[0] ^= 1
    await writeFile(source, bytes)
    await assert.rejects(assertCurrentInputs(fixture, manifest), error => error.code === 'ERR_ASSERTION' && error.message.includes(regenerate))
  } finally {
    await rm(fixture, { recursive: true, force: true })
  }
})

test('the committed pulldown-cmark module embeds no builder home paths', async () => {
  const bytes = await readFile(new URL('markdown-pulldown.generated.wasm', composition))
  for (const home of ['/home/', '/Users/', '/root/'])
    assert.ok(!bytes.includes(Buffer.from(home)), `the module embeds ${home}: rebuild with the generator's path remapping`)
})

test('the committed pulldown-cmark module is the one its manifest records', async () => {
  const bytes = await readFile(new URL('markdown-pulldown.generated.wasm', composition))
  const manifest = JSON.parse(await readFile(new URL('markdown-pulldown.generated.json', composition), 'utf8'))
  assert.equal(createHash('sha256').update(bytes).digest('hex'), manifest.sha256, 'regenerate with packages/fractal-ui/scripts/generate-markdown-wasm.mjs')
  assert.ok(manifest.crates.every(crate => typeof crate.license === 'string' && crate.license !== ''))
})
