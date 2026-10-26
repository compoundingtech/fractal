import { existsSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { defaultChangelogRegistries, prepareChangelogBuild } from '../../scripts/changelog-build.mjs'

const build = { machineVersion: '0.1.0+synthetic.1' }

describe('authored changelog registries', () => {
  it('the public registry lists every public file once and compiles to public entries only', async () => {
    const prepared = await prepareChangelogBuild({ privateBuild: false })
    const manifest = prepared.compile(build)
    expect(prepared.deployment).toBe('main')
    expect(manifest.entries.every(({ visibility }) => visibility === 'public')).toBe(true)
    expect(manifest.entries.map(({ id }) => id)).toEqual(prepared.entryIds)
    for (const entry of prepared.privateEntries) expect(prepared.entryIds).not.toContain(entry.id)
  })

  it('a public build may select the stable channel but never a private one', async () => {
    expect((await prepareChangelogBuild({ privateBuild: false, deployment: 'stable' })).deployment).toBe('stable')
    await expect(prepareChangelogBuild({ privateBuild: false, deployment: 'live' })).rejects.toThrow('does not match a public build')
  })

  it.runIf(existsSync(defaultChangelogRegistries.private))('the private registry is a superset of the public one', async () => {
    const publicIds = (await prepareChangelogBuild({ privateBuild: false })).entryIds
    const prepared = await prepareChangelogBuild({ privateBuild: true })
    expect(prepared.deployment).toBe('live')
    expect(prepared.entryIds).toEqual(expect.arrayContaining([...publicIds]))
    expect(prepared.compile(build).entries.map(({ id }) => id)).toEqual(prepared.entryIds)
  })
})
