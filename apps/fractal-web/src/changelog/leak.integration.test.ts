import { existsSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
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
    `import { getChangelogManifest } from ${JSON.stringify(join(app, 'src/changelog/build.ts'))}`,
    `import { changelogEntryIds } from ${JSON.stringify(join(app, 'src/changelog/metadata.ts'))}`,
    `globalThis.changelog = { changelogManifest: getChangelogManifest(${JSON.stringify(machineVersion)}), changelogEntryIds }`,
    main,
  ].join('\n'))
  const html = join(root, 'index.html')
  writeFileSync(html, '<script>globalThis.earlyClient = "__FRACTAL_WEB_CLIENT__"</script><script type="module" src="/main.js"></script>')
  const output = await build({
    configFile: false,
    root,
    logLevel: 'silent',
    plugins: [identity, extensionBuild({ entry: '../src/extensions/app-public.ts', privateBuild: false, changelogRegistries: registries })],
    build: { write: false, minify: false, rolldownOptions: { input: html } },
  })
  const outputs = Array.isArray(output) ? output : [output]
  return outputs.flatMap((result) => 'output' in result ? result.output : []).map((item) => item.type === 'chunk' ? item.code : String(item.source)).join('\n')
}

describe('public changelog build gate', () => {
  it('replaces the changelog modules with the compiled public manifest for the exact build identity', async () => {
    const text = await buildPublic('', { public: publicRegistry, private: privateRegistry() })
    expect(text).toContain(machineVersion)
    expect(text).toContain(`fractal-web ${machineVersion}`)
    expect(text).not.toContain('__FRACTAL_WEB_CLIENT__')
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

  it('fails when a private registry module enters the public graph', async () => {
    const registry = privateRegistry()
    await expect(buildPublic(`import { changelogDeployment as leaked } from ${JSON.stringify(registry)}\nglobalThis.leak = leaked`, { public: publicRegistry, private: registry }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('fails when a private entry file enters the public graph', async () => {
    const registry = privateRegistry()
    await expect(buildPublic(`import leaked from ${JSON.stringify(join(root, 'private/.changelog', syntheticFile))}\nglobalThis.leak = leaked`, { public: publicRegistry, private: registry }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('protects sibling private files when the registry file is a symlink to another directory', async () => {
    const registry = privateRegistry()
    const targetDirectory = join(root, 'registry-target')
    writeRegistry(targetDirectory, `...publicSources, { file: '${syntheticFile}', source: synthetic, visibility: 'private' }`,
      `import { changelogSources as publicSources } from ${JSON.stringify(publicRegistry)}\nexport const changelogDeployment = 'live'\n`)
    rmSync(registry)
    symlinkSync(join(targetDirectory, 'mod.ts'), registry)
    const sibling = join(root, 'private/.changelog', syntheticFile)
    writeFileSync(sibling, `export default ${JSON.stringify(synthetic)}\nexport const neutral = 'fixture'\n`)
    await expect(buildPublic(`import { neutral } from ${JSON.stringify(sibling)}\nglobalThis.leak = neutral`, { public: publicRegistry, private: registry }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('rejects a neutral registry export through a registry-file symlink outside the private directory', async () => {
    const registry = privateRegistry()
    const targetDirectory = join(root, 'registry-target')
    writeRegistry(targetDirectory, `...publicSources, { file: '${syntheticFile}', source: synthetic, visibility: 'private' }`,
      `import { changelogSources as publicSources } from ${JSON.stringify(publicRegistry)}\nexport const changelogDeployment = 'live'\nexport const neutral = 'synthetic-neutral-export'\n`)
    rmSync(registry)
    symlinkSync(join(targetDirectory, 'mod.ts'), registry)
    await expect(buildPublic(`import { neutral } from ${JSON.stringify(join(targetDirectory, 'mod.ts'))}\nglobalThis.leak = neutral`, { public: publicRegistry, private: registry }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('permits a neutral module in a neighbouring directory of a registry-file symlink target', async () => {
    const registry = privateRegistry()
    const targetDirectory = join(root, 'registry-target')
    writeRegistry(targetDirectory, `...publicSources, { file: '${syntheticFile}', source: synthetic, visibility: 'private' }`,
      `import { changelogSources as publicSources } from ${JSON.stringify(publicRegistry)}\nexport const changelogDeployment = 'live'\n`)
    rmSync(registry)
    symlinkSync(join(targetDirectory, 'mod.ts'), registry)
    const neighbour = `${targetDirectory}-public`
    mkdirSync(neighbour)
    writeFileSync(join(neighbour, 'mod.ts'), "export const neutral = 'synthetic-public-neighbour'\n")
    await expect(buildPublic(`import { neutral } from ${JSON.stringify(join(neighbour, 'mod.ts'))}\nglobalThis.allowed = neutral`, { public: publicRegistry, private: registry }))
      .resolves.toContain('synthetic-public-neighbour')
  })

  it('uses native on-disk casing for a differently cased private registry path (requires a case-insensitive filesystem)', async ({ skip }) => {
    privateRegistry()
    const registry = join(root, 'PRIVATE/.changelog/mod.ts')
    if (!existsSync(registry)) {
      skip('Case-insensitive filesystem required; this filesystem treats PRIVATE and private as different paths')
      return
    }
    const sibling = join(root, 'private/.changelog', syntheticFile)
    writeFileSync(sibling, `export default ${JSON.stringify(synthetic)}\nexport const neutral = 'fixture'\n`)
    await expect(buildPublic(`import { neutral } from ${JSON.stringify(sibling)}\nglobalThis.leak = neutral`, { public: publicRegistry, private: registry }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('rejects a neutral private export through a symlinked registry even when entry text is tree-shaken', async () => {
    privateRegistry()
    const alias = join(root, 'private-link')
    symlinkSync(join(root, 'private'), alias, 'dir')
    const registry = join(alias, '.changelog/mod.ts')
    // Only the neutral deployment string reaches output. The private module
    // graph itself must fail; the separate entry-text test cannot cover this.
    await expect(buildPublic(`import { changelogDeployment as leaked } from ${JSON.stringify(registry)}\nglobalThis.leak = leaked`, { public: publicRegistry, private: registry }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('rejects a private module when the configured registry uses a symlink spelling', async () => {
    privateRegistry()
    const alias = join(root, 'private-alias')
    symlinkSync(join(root, 'private'), alias, 'dir')
    await expect(buildPublic(`import leaked from ${JSON.stringify(join(alias, '.changelog', syntheticFile))}\nglobalThis.leak = leaked`, { public: publicRegistry, private: join(alias, '.changelog/mod.ts') }))
      .rejects.toThrow('Public build leaked private changelog modules')
  })

  it('fails before bundling when a public registry carries a private entry', async () => {
    writeRegistry(join(root, 'public/.changelog'), `{ file: '${syntheticFile}', source: synthetic, visibility: 'private' }`, `export const changelogDeployment = 'main'\n`)
    await expect(buildPublic('', { public: join(root, 'public/.changelog/mod.ts'), private: join(root, 'absent/.changelog/mod.ts') }))
      .rejects.toThrow('changelog private-entry-in-public-audience')
  })
})
