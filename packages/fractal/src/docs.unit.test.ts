import { mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { NodeFileSystem } from '@effect/platform-node'
import { describe, expect, it } from '@effect/vitest'
import { Effect } from 'effect'

import { agent, emit, mission, type MissionInput } from './mod.ts'
import { loadSubjectTree } from './tree.ts'

/**
 * The documentation's `variant=ts` samples that declare a whole file start with a `// <id>/<kind>.ts`
 * path comment. Together they form one tree; each must render exactly its group's `variant=kdl` block.
 */
const site = fileURLToPath(new URL('../../../docs/site', import.meta.url))
const entrypoint = new URL('./index.ts', import.meta.url).href

type Sample = { readonly page: string; readonly group: string; readonly ts: string; readonly kdl: string }

const samples: readonly Sample[] = readdirSync(site, { recursive: true, encoding: 'utf8' })
  .filter((path) => path.endsWith('.md'))
  .toSorted()
  .flatMap((page) => {
    const variants = new Map<string, Map<string, string>>()
    const markdown = readFileSync(join(site, page), 'utf8')
    for (const [, group, variant, body] of markdown.matchAll(
      /^```(?:ts|kdl) group=([a-z][a-z0-9-]*) variant=(effect|ts|kdl)\n([\s\S]*?)^```$/gmu,
    )) {
      const entry = variants.get(group!) ?? new Map<string, string>()
      variants.set(group!, entry.set(variant!, body!))
    }
    return [...variants].flatMap(([group, entry]) => {
      const ts = entry.get('ts')
      const kdl = entry.get('kdl')
      return ts === undefined || kdl === undefined || /^\/\/ \S+\/(agent|mission|schedule)\.ts\n/u.test(ts) === false
        ? []
        : [{ page, group, ts, kdl }]
    })
  })

const path = (sample: Sample) => /^\/\/ (\S+)\n/u.exec(sample.ts)![1]!

describe('documentation samples', () => {
  it('declare each file once', () => {
    const sources = new Map<string, string>()
    for (const sample of samples) {
      const existing = sources.get(path(sample))
      expect(existing ?? sample.ts, `${sample.page} ${sample.group}`).toBe(sample.ts)
      sources.set(path(sample), sample.ts)
    }
    expect(sources.size).toBeGreaterThan(0)
  })

  it.effect('render the KDL shown beside them', () =>
    Effect.gen(function* () {
      const root = mkdtempSync(join(tmpdir(), 'fractal-docs-'))
      for (const sample of samples) {
        mkdirSync(dirname(join(root, path(sample))), { recursive: true })
        writeFileSync(join(root, path(sample)), sample.ts.replaceAll("from 'fractal'", `from '${entrypoint}'`))
      }
      const subjects = yield* loadSubjectTree({ root, context: undefined }).pipe(
        Effect.provide(NodeFileSystem.layer),
      )
      for (const sample of samples) {
        const subject = subjects.find((candidate) => candidate.path === path(sample))
        if (subject?.declaration._tag !== 'Module') return yield* Effect.die(`${path(sample)} did not load`)
        const rendered =
          subject.kind === 'agent'
            ? agent(subject.declaration.intent)
            : mission(subject.declaration.intent as MissionInput<string>)
        expect(emit([rendered]), `${sample.page} ${sample.group}`).toBe(sample.kdl)
      }
    }),
  )
})
