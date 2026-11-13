import { createHash } from 'node:crypto'
import { readdir, readFile } from 'node:fs/promises'
import { resolve } from 'node:path'

// Paths are repository-relative, sorted and length-framed with their contents.
// Hash package-local build inputs only: unrelated root flake bumps must not stale this
// package's generated manifest. The generator records rustc separately as provenance.
export async function markdownWasmInputs(root) {
  const files = [
    'packages/fractal-ui/markdown-wasm/Cargo.toml',
    'packages/fractal-ui/markdown-wasm/Cargo.lock',
    'packages/fractal-ui/scripts/generate-markdown-wasm.mjs',
    'packages/fractal-ui/scripts/markdown-wasm-inputs.mjs',
  ]
  async function visit(path) {
    for (const entry of await readdir(resolve(root, path), { withFileTypes: true })) {
      const child = `${path}/${entry.name}`
      if (entry.isDirectory()) await visit(child)
      else files.push(child)
    }
  }
  await visit('packages/fractal-ui/markdown-wasm/src')
  files.sort()
  const hash = createHash('sha256')
  for (const path of files) {
    const bytes = await readFile(resolve(root, path))
    hash.update(`${JSON.stringify(path)}:${bytes.length}\n`).update(bytes)
  }
  return { sha256: hash.digest('hex'), files }
}
