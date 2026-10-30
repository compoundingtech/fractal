// Negative control for the public changelog privacy gate: real public Vite builds against a
// synthetic private registry. The clean build must pass; every injected leak must fail the build.
// Usage: node scripts/changelog-leak-control.mjs   (each case runs in its own process)
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const app = fileURLToPath(new URL('..', import.meta.url))
const self = fileURLToPath(import.meta.url)
const synthetic = { title: 'Synthetic private control entry', summary: 'This synthetic private entry must never reach a public bundle.' }
const syntheticFile = '2000-01-01-synthetic-private-control.ts'

const cases = {
  clean: { expect: undefined },
  text: { expect: 'Public build leaked private changelog entry 2000-01-01-synthetic-private-control' },
  module: { expect: 'Public build leaked private changelog modules' },
  manifest: { expect: 'changelog private-entry-in-public-audience' },
}

const writeRegistry = (directory, rows, extra = '') => {
  mkdirSync(directory, { recursive: true })
  writeFileSync(join(directory, syntheticFile), `export default ${JSON.stringify(synthetic)}\n`)
  writeFileSync(join(directory, 'mod.ts'), `${extra}import synthetic from './${syntheticFile}'\nexport const changelogSources = [${rows}]\n`)
}

const runCase = async (name, root) => {
  const { build } = await import('vite')
  const { default: publicConfig } = await import(pathToFileURL(resolve(app, 'vite.config.ts')).href)
  const { extensionBuild } = await import('./extensions-build.mjs')
  const privateRegistry = join(root, 'private/.changelog/mod.ts')
  writeRegistry(join(root, 'private/.changelog'), `...publicSources, { file: '${syntheticFile}', source: synthetic, visibility: 'private' }`,
    `import { changelogSources as publicSources } from ${JSON.stringify(resolve(app, '.changelog/mod.ts'))}\nexport const changelogDeployment = 'live'\n`)
  const registries = name === 'manifest'
    ? { public: join(root, 'public/.changelog/mod.ts'), private: join(root, 'absent/.changelog/mod.ts') }
    : { public: resolve(app, '.changelog/mod.ts'), private: privateRegistry }
  if (name === 'manifest')
    writeRegistry(join(root, 'public/.changelog'), `{ file: '${syntheticFile}', source: synthetic, visibility: 'private' }`, `export const changelogDeployment = 'main'\n`)
  const injected = name === 'text' ? `\nglobalThis.__changelogLeakControl = ${JSON.stringify(synthetic.title)};\n`
    : name === 'module' ? `\nimport __changelogLeakControl from ${JSON.stringify(join(root, 'private/.changelog', syntheticFile))};\nglobalThis.__changelogLeakControl = __changelogLeakControl;\n`
      : undefined
  const leak = {
    name: 'fractal:changelog-leak-control',
    enforce: 'post',
    transform(code, id) {
      if (injected !== undefined && id.split('?')[0].endsWith('/src/web/main.tsx')) return { code: code + injected, map: null }
    },
  }
  await build({
    ...publicConfig,
    configFile: false,
    root: resolve(app, 'src/web'),
    logLevel: 'error',
    plugins: [
      ...(publicConfig.plugins ?? []).filter((plugin) => plugin?.name !== 'fractal:build-time-extensions'),
      extensionBuild({ entry: '../src/extensions/app-public.ts', privateBuild: false, changelogRegistries: registries }),
      leak,
    ],
    build: { ...publicConfig.build, outDir: join(root, 'out'), emptyOutDir: true },
  })
}

const [, , mode, name, root] = process.argv
if (mode === '--case') {
  try {
    await runCase(name, root)
  } catch (error) {
    process.stderr.write(`CONTROL-FAILURE ${error instanceof Error ? error.message : String(error)}\n`)
    process.exit(1)
  }
  process.exit(0)
}

let failed = false
for (const [name, { expect }] of Object.entries(cases)) {
  const root = mkdtempSync(join(tmpdir(), `fractal-changelog-${name}-`))
  try {
    const run = spawnSync(process.execPath, [self, '--case', name, root], { cwd: app, encoding: 'utf8' })
    const ok = expect === undefined ? run.status === 0 : run.status !== 0 && run.stderr.includes(expect)
    failed ||= !ok
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${expect === undefined ? 'public build passes' : `build fails with "${expect}"`}${ok ? '' : `\n${run.stderr}${run.stdout}`}`)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}
process.exit(failed ? 1 : 0)
