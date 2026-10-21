import { describe, expect, it } from 'vitest'
import { compileChangelog, compileChangelogEntries } from './compiler.ts'
import { ChangelogError, decodeChangelogManifest, type ChangelogSourceRegistration } from './contract.ts'

const build = { machineVersion: '0.1.0+synthetic.1' }
const source = (title: string) => ({ title, summary: `${title} is described in one sentence.` })
const sources: readonly ChangelogSourceRegistration[] = [
  { file: '2026-01-02-beta.ts', source: source('Beta'), visibility: 'public' },
  { file: '2026-01-03-gamma.ts', source: { ...source('Gamma'), kind: 'fix', publicPrUrl: 'https://github.com/example/project/pull/7' }, visibility: 'private' },
  { file: '2026-01-02-alpha.ts', source: source('Alpha'), visibility: 'public' },
]
const reason = (run: () => unknown): string | undefined => {
  try {
    run()
  } catch (error) {
    if (error instanceof ChangelogError) return error.reason
    throw error
  }
  return undefined
}
const compiled = () => compileChangelog({ build, sources, audience: 'private' })

describe('changelog compiler', () => {
  it('derives identity from filenames and orders newest first deterministically', () => {
    const manifest = compiled()
    expect(manifest).toEqual({
      schemaVersion: 1,
      product: 'fractal-web',
      build,
      entries: [
        { id: '2026-01-03-gamma', date: '2026-01-03', visibility: 'private', title: 'Gamma', summary: 'Gamma is described in one sentence.', kind: 'fix', publicPrUrl: 'https://github.com/example/project/pull/7' },
        { id: '2026-01-02-beta', date: '2026-01-02', visibility: 'public', title: 'Beta', summary: 'Beta is described in one sentence.' },
        { id: '2026-01-02-alpha', date: '2026-01-02', visibility: 'public', title: 'Alpha', summary: 'Alpha is described in one sentence.' },
      ],
    })
    const permuted = compileChangelog({ build, sources: sources.toReversed(), audience: 'private' })
    expect(JSON.stringify(permuted)).toBe(JSON.stringify(manifest))
  })

  it('refuses private sources for a public audience instead of filtering them', () => {
    expect(reason(() => compileChangelog({ build, sources, audience: 'public' }))).toBe('private-entry-in-public-audience')
    const publicOnly = compileChangelog({ build, sources: sources.filter(({ visibility }) => visibility === 'public'), audience: 'public' })
    expect(publicOnly.entries.map(({ id }) => id)).toEqual(['2026-01-02-beta', '2026-01-02-alpha'])
  })

  it('rejects malformed filenames, impossible dates and duplicate identities', () => {
    const one = (file: string) => compileChangelogEntries({ sources: [{ file, source: source('One'), visibility: 'public' }], audience: 'public' })
    expect(reason(() => one('2026-01-02_Alpha.ts'))).toBe('invalid-filename')
    expect(reason(() => one('2026-02-30-alpha.ts'))).toBe('invalid-filename')
    expect(reason(() => one('2026-01-02-alpha.tsx'))).toBe('invalid-filename')
    const twice = { file: '2026-01-02-alpha.ts', source: source('Alpha'), visibility: 'public' } as const
    expect(reason(() => compileChangelogEntries({ sources: [twice, twice], audience: 'public' }))).toBe('duplicate-registration')
    expect(reason(() => compileChangelogEntries({ sources: [twice, { ...twice, visibility: 'private' }], audience: 'private' }))).toBe('duplicate-entry-id')
  })

  it('decodes authored sources strictly', () => {
    const one = (value: unknown) => compileChangelogEntries({ sources: [{ file: '2026-01-02-alpha.ts', source: value, visibility: 'public' }], audience: 'public' })
    expect(reason(() => one({ ...source('Alpha'), author: 'someone' }))).toBe('invalid-source')
    expect(reason(() => one({ title: 'Alpha', summary: 'One. Two. Three. Four.' }))).toBe('invalid-source')
    expect(reason(() => one({ title: 'Alpha', summary: 'No closing punctuation' }))).toBe('invalid-source')
    expect(reason(() => one({ title: ' Alpha', summary: 'Padded title.' }))).toBe('invalid-source')
    expect(reason(() => one({ ...source('Alpha'), kind: 'chore' }))).toBe('invalid-source')
    expect(reason(() => one({ ...source('Alpha'), publicPrUrl: 'https://github.com/example/project/commit/abc' }))).toBe('invalid-source')
    expect(one({ title: 'Alpha', summary: 'One. Two! Three?' })[0]?.summary).toBe('One. Two! Three?')
  })

  it('requires a bijection between registered files and the directory inventory', () => {
    const publicSources = sources.filter(({ visibility }) => visibility === 'public')
    const inventory = publicSources.map(({ file }) => ({ file, visibility: 'public' as const }))
    expect(compileChangelogEntries({ sources: publicSources, audience: 'public', inventory })).toHaveLength(2)
    expect(reason(() => compileChangelogEntries({ sources: publicSources, audience: 'public', inventory: [...inventory, { file: '2026-01-04-extra.ts', visibility: 'public' }] }))).toBe('inventory-mismatch')
    expect(reason(() => compileChangelogEntries({ sources: publicSources, audience: 'public', inventory: inventory.slice(1) }))).toBe('inventory-mismatch')
    expect(reason(() => compileChangelogEntries({ sources: publicSources, audience: 'public', inventory: inventory.map((row) => ({ ...row, visibility: 'private' as const })) }))).toBe('inventory-mismatch')
  })
})

describe('changelog manifest decoding', () => {
  const manifest = JSON.parse(JSON.stringify(compiled()))
  const withEntries = (entries: unknown[]) => ({ ...manifest, entries })
  const [newest, middle, oldest] = manifest.entries

  it('accepts the compiled manifest and the build it describes', () => {
    expect(decodeChangelogManifest(manifest, build)).toEqual(compiled())
    expect(reason(() => decodeChangelogManifest(manifest, { machineVersion: '0.1.0+synthetic.2' }))).toBe('build-mismatch')
  })

  it('rejects unknown fields, foreign products and schema versions', () => {
    expect(reason(() => decodeChangelogManifest({ ...manifest, audience: 'public' }))).toBe('invalid-manifest')
    expect(reason(() => decodeChangelogManifest({ ...manifest, build: { ...build, rev: 'abc' } }))).toBe('invalid-manifest')
    expect(reason(() => decodeChangelogManifest(withEntries([{ ...newest, contributors: [] }, middle, oldest])))).toBe('invalid-manifest')
    expect(reason(() => decodeChangelogManifest({ ...manifest, product: 'other' }))).toBe('invalid-manifest')
    expect(reason(() => decodeChangelogManifest({ ...manifest, schemaVersion: 2 }))).toBe('invalid-manifest')
    expect(reason(() => decodeChangelogManifest({ ...manifest, build: { machineVersion: '' } }))).toBe('invalid-manifest')
  })

  it('rejects duplicate ids, invalid dates and unsorted entries', () => {
    expect(reason(() => decodeChangelogManifest(withEntries([newest, newest])))).toBe('duplicate-entry-id')
    expect(reason(() => decodeChangelogManifest(withEntries([oldest, newest])))).toBe('unsorted-entries')
    expect(reason(() => decodeChangelogManifest(withEntries([{ ...newest, id: '2026-02-30-gamma', date: '2026-02-30' }])))).toBe('invalid-manifest')
    expect(reason(() => decodeChangelogManifest(withEntries([{ ...newest, date: '2026-01-04' }])))).toBe('invalid-manifest')
    expect(reason(() => decodeChangelogManifest(withEntries([{ ...newest, visibility: 'internal' }])))).toBe('invalid-manifest')
  })
})
