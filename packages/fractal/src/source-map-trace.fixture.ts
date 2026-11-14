/**
 * `bun src/source-map-trace.fixture.ts ROOT [trace]` loads the subject tree at ROOT, renders every
 * agent and mission, and prints `{ subjects, reads }` as JSON. With `trace` the tree loads under
 * `traceSourceReads`; source-map.unit.test.ts compares both renderings and maps the reads.
 */
import { realpathSync } from 'node:fs'
import { join } from 'node:path'

import { NodeFileSystem } from '@effect/platform-node'
import { Effect } from 'effect'
import { parseSync } from 'oxc-parser'

import { agent, emit, mission } from './mod.ts'
import { traceSourceReads } from './source-map.ts'
import { loadSubjectTree } from './tree.ts'

const [root, mode] = process.argv.slice(2)
if (root === undefined) throw new Error('usage: source-map-trace.fixture.ts ROOT [trace]')
const trace = mode === 'trace' ? traceSourceReads({ parse: parseSync, roots: [root] }) : undefined
const subjects = await Effect.runPromise(
  loadSubjectTree({ root, context: {} }).pipe(Effect.provide(NodeFileSystem.layer)),
)
const rendered = subjects.flatMap((subject) => {
  const file = join(realpathSync(root), subject.path)
  if (subject.kind === 'agent' && subject.declaration._tag === 'Module') {
    return [{ subject: `agent/${subject.id}`, file, kdl: emit([agent(subject.declaration.intent)]) }]
  }
  if (subject.kind === 'mission' && subject.declaration._tag === 'Module') {
    return [{ subject: `mission/${subject.id}`, file, kdl: emit([mission(subject.declaration.intent)]) }]
  }
  return []
})
console.log(JSON.stringify({ subjects: rendered, reads: trace?.reads() ?? [] }))
