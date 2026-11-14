import { readdirSync, readFileSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const repo = resolve(import.meta.dirname, '../../../..')
const roots = ['apps/fractal-web/src', 'packages/fractal-ui/src'].map(root => join(repo, root))
const sources = (dir: string): readonly string[] =>
  readdirSync(dir, { withFileTypes: true }).flatMap(entry =>
    entry.isDirectory() ? sources(join(dir, entry.name)) : /\.(tsx?|css)$/.test(entry.name) ? [join(dir, entry.name)] : [],
  )
const self = 'apps/fractal-web/src/web/liveRing.unit.test.ts'
const files = roots.flatMap(sources).map(file => relative(repo, file)).filter(file => file !== self)
const containing = (needle: string) => files.filter(file => readFileSync(join(repo, file), 'utf8').includes(needle))

describe('the one loading spinner', () => {
  it('is the only rotating drawing in the app and kit', () => {
    expect(containing('rotate(360deg)')).toEqual(['packages/fractal-ui/src/assistant-ui/composition/LiveRing.tsx'])
  })
  it('keeps the rejected double arc out', () => {
    expect(containing('M8 1.5A6.5 6.5')).toEqual([])
  })
})
