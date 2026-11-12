import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'

const composition = new URL('../src/assistant-ui/composition/', import.meta.url)

test('the committed pulldown-cmark module is the one its manifest records', async () => {
  const bytes = await readFile(new URL('markdown-pulldown.generated.wasm', composition))
  const manifest = JSON.parse(await readFile(new URL('markdown-pulldown.generated.json', composition), 'utf8'))
  assert.equal(createHash('sha256').update(bytes).digest('hex'), manifest.sha256, 'regenerate with packages/fractal-ui/scripts/generate-markdown-wasm.mjs')
  assert.ok(manifest.crates.every(crate => typeof crate.license === 'string' && crate.license !== ''))
})
