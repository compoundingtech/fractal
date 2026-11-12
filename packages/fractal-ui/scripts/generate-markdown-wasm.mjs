// Regenerates the committed pulldown-cmark WebAssembly module the kit's Markdown can load.
// Usage (inside `nix develop`): node packages/fractal-ui/scripts/generate-markdown-wasm.mjs
// The build is pinned by markdown-wasm/Cargo.lock (--locked); the manifest records the output's hash, the crates in
// it and their licenses, and `markdown-wasm.node-test.mjs` checks the committed module against that hash.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const kit = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const crate = resolve(kit, 'markdown-wasm')
const output = resolve(kit, 'src/assistant-ui/composition/markdown-pulldown.generated.wasm')
const manifest = resolve(kit, 'src/assistant-ui/composition/markdown-pulldown.generated.json')
const LICENSES = { 'pulldown-cmark': 'MIT', bitflags: 'MIT OR Apache-2.0', memchr: 'Unlicense OR MIT', unicase: 'MIT OR Apache-2.0', 'fractal-markdown-wasm': 'repository license' }

execFileSync('cargo', ['build', '--release', '--locked', '--target', 'wasm32-unknown-unknown'], { cwd: crate, stdio: 'inherit' })
const bytes = await readFile(resolve(crate, 'target/wasm32-unknown-unknown/release/fractal_markdown_wasm.wasm'))
await writeFile(output, bytes)
const lock = await readFile(resolve(crate, 'Cargo.lock'), 'utf8')
const crates = [...lock.matchAll(/name = "([^"]+)"\nversion = "([^"]+)"/g)].map(([, name, version]) => {
  const license = LICENSES[name]
  if (license === undefined) throw new Error(`Record the license of ${name} in generate-markdown-wasm.mjs before shipping it`)
  return { name, version, license }
})
const rustc = execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim()
await writeFile(manifest, `${JSON.stringify({ sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length, rustc, crates }, null, 2)}\n`)
console.log(`wrote ${bytes.length} bytes`)
