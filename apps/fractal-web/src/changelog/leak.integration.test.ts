import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build, type Plugin } from 'vite'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { extensionBuild } from '../../scripts/extensions-build.mjs'
import { changelogSources } from '../../.changelog/mod.ts'

// Real Vite/Rolldown graphs through the production plugin: the changelog modules are replaced by
// compiled literals, and the public gate rejects injected private text or modules.
const app = fileURLToPath(new URL('../..', import.meta.url))
const publicRegistry = join(app, '.changelog/mod.ts')
const machineVersion = '0.1.0+synthetic.leak-control'
const synthetic = { title: 'Synthetic private control entry', summary: 'This synthetic private entry must never reach a public bundle.' }
const syntheticFile = '2000-01-01-synthetic-private-control.ts'
const identity: Plugin = {
  name: 'test:build-identity',
  enforce: 'pre',
  resolveId: (id) => id === 'virtual:build-identity' ? '\0test-build-identity' : undefined,
  load: (id) => id === '\0test-build-identity' ? `export const buildIdentity = ${JSON.stringify({ machineVersion })};\nexport const deploymentId = undefined;\n` : undefined,
}

let root = ''
beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), 'fractal-changelog-leak-'))
})
afterEach(() => rmSync(root, { recursive: true, force: true }))

const writeRegistry = (directory: string, rows: string, header: string) => {
  mkdirSync(directory, { recursive: true })
  writeFileSync(join(directory, syntheticFile), `export default ${JSON.stringify(synthetic)}\n`)
  writeFileSync(join(directory, 'mod.ts'), `${header}import synthetic from './${syntheticFile}'\nexport const changelogSources = [${rows}]\n`)
}
const privateRegistry = () => {
  writeRegistry(join(root, 'private/.changelog'), `...publicSources, { file: '${syntheticFile}', source: synthetic, visibility: 'private' }`,
    `import { changelogSources as publicSources } from ${JSON.stringify(publicRegistry)}\nexport const changelogDeployment = 'live'\n`)
  return join(root, 'private/.changelog/mod.ts')
}
const buildPublic = async (main: string, registries: { readonly public: string; readonly private: string }) => {
  const entry = join(root, 'main.js')
  writeFileSync(entry, [
    `import { changelogManifest } from ${JSON.stringify(join(app, 'src/changelog/build.ts'))}`,
    `import { changelogEntryIds } from ${JSON.stringify(join(app, 'src/changelog/metadata.ts'))}`,
    'globalThis.changelog = { changelogManifest, changelogEntryIds }',
    main,
  ].join('\n'))
  const output = await build({
    configFile: false,
    root,
    logLevel: 'silent',
    plugins: [identity, extensionBuild({ entry: '../src/extensions/app-public.ts', privateBuild: false, changelogRegistries: registries })],
    build: { write: false, minify: false, rolldownOptions: { input: entry } },
  })
  const outputs = Array.isArray(output) ? output : [output]
  return outputs.flatMap((result) => 'output' in result ? result.output : []).map((item) => item.type === 'chunk' ? item.code : String(item.source)).join('\n')
}

describe('public changelog build gate', () => {
  it('replaces the changelog modules with the compiled public manifest for the exact build identity', async () => {
    const text = await buildPublic('', { public: publicRegistry, private: privateRegistry() })
    expect(text).toContain(machineVersion)
    for (const { file } of changelogSources) {
      expect(text).toContain(file.replace(/\.ts$/, ''))
    }
    expect(text).not.toContain(synthetic.title)
    expect(text).not.toContain('2000-01-01-synthetic-private-control')
  })

  it('fails when private entry text reaches the public output', async () => {
    await expect(buildPublic(`globalThis.leak = ${JSON.stringify(synthetic.summary)}`, { public: publicRegistry, private: privateRegistry() }))
      .rejects.toThrow('Public build leaked private changelog entry 2000-01-01-synthetic-private-control')
  })

  it('fails when a private changelog module enters the public graph', async () => {
    const registry = privateRegistry()
    await expect(buildPublic(`import leaked from ${JSON.stringify(join(root, 'private/.changelog', syntheticFile))}\nglobalThis.leak = leaked`, { public: publicRegistry, private: registry }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('fails before bundling when a public registry carries a private entry', async () => {
    writeRegistry(join(root, 'public/.changelog'), `{ file: '${syntheticFile}', source: synthetic, visibility: 'private' }`, `export const changelogDeployment = 'main'\n`)
    await expect(buildPublic('', { public: join(root, 'public/.changelog/mod.ts'), private: join(root, 'absent/.changelog/mod.ts') }))
      .rejects.toThrow('changelog private-entry-in-public-audience')
  })
})
