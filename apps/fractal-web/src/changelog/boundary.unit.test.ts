import { describe, expect, it } from 'vitest'
import { assertPublicChangelogBoundary, evaluateBuildIdentityModule } from '../../scripts/changelog-build.mjs'
import { compileChangelog } from './compiler.ts'

const privateDirectory = '/checkout/apps/fractal-web/private/.changelog'
const manifest = compileChangelog({
  build: { machineVersion: '0.1.0+synthetic.1' },
  sources: [
    { file: '2000-01-02-synthetic-private-control.ts', source: { title: 'Synthetic private control', summary: 'A synthetic private control entry.', publicPrUrl: 'https://github.com/example/project/pull/9' }, visibility: 'private' },
    { file: '2000-01-01-synthetic-public-entry.ts', source: { title: 'Synthetic public entry', summary: 'A synthetic public entry.' }, visibility: 'public' },
  ],
  audience: 'private',
})
const privateEntries = manifest.entries.filter(({ visibility }) => visibility === 'private')
const publicManifest = { ...manifest, entries: manifest.entries.filter(({ visibility }) => visibility === 'public') }
const clean = { manifest: publicManifest, privateEntries, privateDirectory, modules: ['/checkout/apps/fractal-web/src/web/main.tsx'], texts: [JSON.stringify(publicManifest)] }

describe('public changelog boundary', () => {
  it('accepts a public bundle that carries only public entries', () => {
    expect(() => assertPublicChangelogBoundary(clean)).not.toThrow()
    expect(() => assertPublicChangelogBoundary({ ...clean, manifest: undefined })).not.toThrow()
  })

  it('fails on a private entry in the public manifest', () => {
    expect(() => assertPublicChangelogBoundary({ ...clean, manifest })).toThrow('Public build leaked private changelog entries into its manifest')
  })

  it('fails on any private entry text in emitted output', () => {
    for (const marker of [privateEntries[0]!.id, privateEntries[0]!.title, privateEntries[0]!.summary, privateEntries[0]!.publicPrUrl!])
      expect(() => assertPublicChangelogBoundary({ ...clean, texts: [...clean.texts, `globalThis.leak = ${JSON.stringify(marker)}`] }))
        .toThrow('Public build leaked private changelog entry 2000-01-02-synthetic-private-control')
  })

  it('fails on a module from the private changelog directory', () => {
    expect(() => assertPublicChangelogBoundary({ ...clean, modules: [...clean.modules, `${privateDirectory}/mod.ts`] }))
      .toThrow('Public build leaked private changelog modules')
  })
})

describe('build identity evaluation', () => {
  it('reads the exact machine version the identity module exports', async () => {
    expect(await evaluateBuildIdentityModule('export const buildIdentity = { machineVersion: "0.1.0+synthetic.1" }; export const deploymentId = undefined')).toEqual({ machineVersion: '0.1.0+synthetic.1' })
    await expect(evaluateBuildIdentityModule('export const buildIdentity = {}')).rejects.toThrow('machineVersion')
  })
})
