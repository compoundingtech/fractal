import { describe, expect, it } from 'vitest'
import { Fractal, emit, githubScope, githubSource } from './index.ts'
import type { GithubScopeInput, GithubSourceInput } from './github.ts'

const source: GithubSourceInput = {
  name: 'sample', host: 'github.com', generation: 1,
  repositories: ['acme/widgets'], enabled: false, checkpointEveryMs: 60_000, replicatePrivate: false,
}
const scope: GithubScopeInput = {
  name: 'sample', revision: 'revision-one',
  repositories: [{ repositoryId: 'R_sample', fromMs: 100, untilMs: 200 }, { repositoryId: 'R_sample', fromMs: 200 }],
}

describe('native GitHub root declarations', () => {
  it('exports both builders on the authoring namespace', () => {
    expect(Fractal.githubSource).toBe(githubSource)
    expect(Fractal.githubScope).toBe(githubScope)
  })

  it('renders disabled acquisition settings before any authority grant exists', () => {
    expect(emit([githubSource(source)])).toBe(`version 2
github-source "sample" {
  host "github.com"
  generation 1
  repository "acme/widgets"
  enabled #false
  checkpoint-every-ms 60000
  replicate-private #false
}
`)
  })

  it('requires an admitted grant when enabling but permits a selected grant while disabled', () => {
    expect(() => Reflect.apply(githubSource, undefined, [{ ...source, enabled: true }]))
      .toThrow('enabled github-source requires authorityGrant')
    expect(emit([githubSource({ ...source, enabled: true, authorityGrant: 'grant/sample' })]))
      .toContain('authority-grant "grant/sample"')
    expect(emit([githubSource({ ...source, authorityGrant: 'grant/sample' })]))
      .toContain('authority-grant "grant/sample"')
  })

  it('renders effective half-open membership intervals and an explicit revision', () => {
    expect(emit([githubScope(scope)])).toBe(`version 2
github-scope "sample" {
  revision "revision-one"
  repository "R_sample" from-ms=100 until-ms=200
  repository "R_sample" from-ms=200
}
`)
  })

  it('rejects invalid source identities, duplicate locators and unsafe numbers', () => {
    for (const change of [
      { name: '' }, { name: 'é'.repeat(65) }, { host: 'https://github.com' }, { host: 'GITHUB.COM' },
      { generation: 0 }, { generation: 1.5 }, { generation: Number.MAX_SAFE_INTEGER + 1 },
      { authorityGrant: '' }, { checkpointEveryMs: 59_999 }, { repositories: [] },
      { repositories: ['acme/widgets', 'acme/widgets'] }, { repositories: ['acme/widgets/extra'] },
    ]) expect(() => githubSource({ ...source, ...change })).toThrow(TypeError)
  })

  it('requires authored nonempty membership, revision and valid dates', () => {
    expect(() => githubScope({ ...scope, revision: '' })).toThrow(TypeError)
    expect(() => githubScope({ ...scope, repositories: [] })).toThrow(TypeError)
    for (const member of [
      { repositoryId: '', fromMs: 100 }, { repositoryId: 'R_sample', fromMs: -1 },
      { repositoryId: 'R_sample', fromMs: 100, untilMs: 100 },
      { repositoryId: 'R_sample', fromMs: Number.MAX_SAFE_INTEGER + 1 },
    ]) expect(() => githubScope({ ...scope, repositories: [member] })).toThrow(TypeError)
  })

  it('rejects same-repository overlaps in either input order, but permits different repositories', () => {
    const repositories = [{ repositoryId: 'R_sample', fromMs: 100, untilMs: 300 }, { repositoryId: 'R_sample', fromMs: 200 }]
    expect(() => githubScope({ ...scope, repositories })).toThrow('overlap')
    expect(() => githubScope({ ...scope, repositories: [...repositories].reverse() })).toThrow('overlap')
    expect(() => githubScope({ ...scope, repositories: [{ repositoryId: 'R_other', fromMs: 100 }, ...scope.repositories] })).not.toThrow()
  })
})
