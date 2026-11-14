// Regenerates the committed pulldown-cmark WebAssembly module the kit's Markdown can load.
// Usage (inside `nix develop`): node packages/fractal-ui/scripts/generate-markdown-wasm.mjs
// The manifest records build-input and output hashes, the crates and their licenses.
// `markdown-wasm.node-test.mjs` rejects stale generated artifacts after source or pin changes.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { homedir } from 'node:os'
import { markdownWasmInputs } from './markdown-wasm-inputs.mjs'

const kit = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const crate = resolve(kit, 'markdown-wasm')
const output = resolve(kit, 'src/assistant-ui/composition/markdown-pulldown.generated.wasm')
const manifest = resolve(kit, 'src/assistant-ui/composition/markdown-pulldown.generated.json')
const LICENSES = { 'pulldown-cmark': 'MIT', bitflags: 'MIT OR Apache-2.0', memchr: 'Unlicense OR MIT', unicase: 'MIT OR Apache-2.0', 'fractal-markdown-wasm': 'repository license' }

// The module must not remember the machine that built it: rustc bakes the source paths of panic
// locations into the binary, so every absolute prefix it can embed — the cargo home (and with it
// the registry sources), the workspace checkout and this crate — is remapped to a stable constant
// before compilation. The committed bytes then depend only on the sources, on any machine.
// markdown-wasm.node-test.mjs fails if a builder home path ever returns.
const cargoHome = process.env.CARGO_HOME || resolve(homedir(), '.cargo')
const remapped = [
  [cargoHome, '/cargo'],
  [resolve(kit, '../..'), '/repository'],
  [crate, '/crate'],
].map(([from, to]) => `--remap-path-prefix=${from}=${to}`)
const inherited = (process.env.CARGO_ENCODED_RUSTFLAGS ?? process.env.RUSTFLAGS ?? '').split(/\s+/).filter(Boolean)
const encodedRustflags = [...remapped, ...inherited].join('\t')
execFileSync('cargo', ['build', '--release', '--locked', '--target', 'wasm32-unknown-unknown'], { cwd: crate, stdio: 'inherit', env: { ...process.env, CARGO_ENCODED_RUSTFLAGS: encodedRustflags } })
const bytes = await readFile(resolve(crate, 'target/wasm32-unknown-unknown/release/fractal_markdown_wasm.wasm'))
await writeFile(output, bytes)
const lock = await readFile(resolve(crate, 'Cargo.lock'), 'utf8')
const crates = [...lock.matchAll(/name = "([^"]+)"\nversion = "([^"]+)"/g)].map(([, name, version]) => {
  const license = LICENSES[name]
  if (license === undefined) throw new Error(`Record the license of ${name} in generate-markdown-wasm.mjs before shipping it`)
  return { name, version, license }
})
const rustc = execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim()
await writeFile(manifest, `${JSON.stringify({ inputs, sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length, rustc, crates }, null, 2)}\n`)
console.log(`wrote ${bytes.length} bytes`)
