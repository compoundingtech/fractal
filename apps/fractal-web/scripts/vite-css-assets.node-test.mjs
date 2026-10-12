import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { build } from 'vite'
import { fractalCssAssets } from './vite-css-assets.mjs'

// The terminal's shape: the entry lazily loads a chunk that imports its face stylesheet
// and, when `preload` is set, also loads the font file by URL as terminalFonts.ts does.
const buildFixture = async ({ stylesheet, preload }) => {
  const root = await mkdtemp(join(tmpdir(), 'fractal-css-assets-'))
  try {
    await writeFile(join(root, 'index.html'), '<script type="module" src="./main.js"></script>\n')
    await writeFile(join(root, 'main.js'), "import('./terminal.js')\n")
    await writeFile(
      join(root, 'terminal.js'),
      preload ? "import './fonts.css'\nexport { default } from './Mono.woff2?url'\n" : "import './fonts.css'\nexport default 1\n",
    )
    await writeFile(join(root, 'fonts.css'), stylesheet)
    // Above Vite's 4 KiB inline limit, like the real faces, so it is emitted as a file.
    await writeFile(join(root, 'Mono.woff2'), randomBytes(8192))
    const result = await build({
      root,
      configFile: false,
      logLevel: 'silent',
      plugins: [fractalCssAssets()],
      build: { write: false },
    })
    return (Array.isArray(result) ? result : [result]).flatMap((output) => output.output)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

const face = "@font-face { font-family: 'Mono'; src: url('./Mono.woff2') format('woff2'); }\n"

test('control: a lazy chunk whose stylesheet declares its preloaded face builds', async () => {
  const output = await buildFixture({ stylesheet: face, preload: true })
  const stylesheets = output.filter((asset) => asset.fileName.endsWith('.css'))
  const font = output.find((asset) => asset.fileName.endsWith('.woff2'))
  assert.equal(stylesheets.length, 1)
  assert.ok(font)
  assert.match(String(stylesheets[0]?.source), new RegExp(`@font-face\\{[^}]*${font.fileName.split('/').pop()}`))
})

test('negative: a lazy chunk whose stylesheet lost every rule fails the build', async () => {
  await assert.rejects(
    buildFixture({ stylesheet: '/* Faces moved elsewhere. */\n', preload: false }),
    /\[fractal:css-assets\]\nassets\/terminal-[\w-]+\.css has no CSS rules \(\d+ bytes\), yet assets\/terminal-[\w-]+\.js loads it/,
  )
})

test('negative: a preloaded font without a declared face fails even when other rules remain', async () => {
  await assert.rejects(
    buildFixture({ stylesheet: '::highlight(find) { color: red; }\n', preload: true }),
    /assets\/terminal-[\w-]+\.js loads assets\/Mono-[\w-]+\.woff2, but no @font-face in its stylesheets \(assets\/terminal-[\w-]+\.css\) declares it/,
  )
})
