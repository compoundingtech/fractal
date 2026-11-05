import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build, type Plugin } from 'vite'
import { expect, it } from 'vitest'
import { englishAriaLocales } from '../../scripts/aria-locales.mjs'

const app = fileURLToPath(new URL('../..', import.meta.url))

it('rejects a production locale graph when the optimizer is accidentally disabled', async () => {
  const fixture = await mkdtemp(join(app, '.aria-locales-test-'))
  try {
    const entry = join(fixture, 'entry.js')
    await writeFile(entry, "export { useDrag } from 'react-aria'\n")
    const compile = (plugins: Plugin[]) => build({
      root: app, configFile: false, logLevel: 'silent', plugins,
      build: { write: false, minify: false, lib: { entry, formats: ['es'] } },
    })
    await compile(englishAriaLocales())
    // Keep the graph guard but remove the optimizer: English runtime providers alone
    // must not pass this proof while every translation still ships.
    await expect(compile(englishAriaLocales().slice(1))).rejects.toThrow(/Unsupported React Aria locales retained/)
  } finally {
    await rm(fixture, { recursive: true, force: true })
  }
}, 30_000)
