import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { NodeFileSystem } from '@effect/platform-node'
import { describe, expect, it } from '@effect/vitest'
import { Effect } from 'effect'

import { encodeKdl, type Agent, type Mission } from '@fractal/declarations'
import { parse, type Node as KdlNode } from '@overeng/kdl'

import { agent, emit, mission, type MissionInput } from './mod.ts'
import { prLanding } from './pr-landing.fixture.ts'
import { loadSubjectTree, type Subject } from './tree.ts'
import { upstreamRepin } from './upstream-repin.fixture.ts'

// The committed goldens are parsed by `fractal validate`'s parser in crates/fractal/tests/validate.rs.
const fixture = (name: string) => new URL(`../fixtures/${name}`, import.meta.url)
const golden = (name: string) => readFileSync(fixture(name), 'utf8')

/** Renders a loaded tree's module subjects as one document: agents, then missions, in path order. */
const render = (subjects: readonly Subject[]): string =>
  emit(
    (['agent', 'mission'] as const).flatMap((kind) =>
      subjects.flatMap((subject) => {
        if (subject.declaration._tag !== 'Module' || (subject.kind === 'agent') !== (kind === 'agent'))
          return []
        return subject.kind === 'agent'
          ? [agent(subject.declaration.intent)]
          : [mission(subject.declaration.intent as MissionInput<string>)]
      }),
    ),
  )

/** Node structure without formatting: property order and top-level order do not matter. */
type Shape = {
  readonly name: string
  readonly args: readonly unknown[]
  readonly props: readonly (readonly [string, unknown])[]
  readonly children: readonly Shape[]
}
const shape = (node: KdlNode): Shape => ({
  name: node.getName(),
  args: node.getArguments(),
  props: [...node.getProperties()].toSorted(([a], [b]) => a.localeCompare(b, 'en')),
  children: (node.children?.nodes ?? []).map(shape),
})
const declarations = (text: string): readonly Shape[] =>
  parse(text)
    .nodes.filter((node) => node.getName() !== 'version')
    .map(shape)
    .toSorted((a, b) => JSON.stringify([a.name, a.args]).localeCompare(JSON.stringify([b.name, b.args])))

const loadDailyReview = loadSubjectTree({
  root: fileURLToPath(fixture('daily-review')),
  context: undefined,
}).pipe(Effect.provide(NodeFileSystem.layer))

describe('KDL parity', () => {
  it.effect('renders the Fractal namespace tree to the committed daily-review golden', () =>
    Effect.gen(function* () {
      expect(render(yield* loadDailyReview)).toBe(golden('daily-review.kdl'))
    }),
  )

  it.effect('declares the same agents and mission as @fractal/declarations', () =>
    Effect.gen(function* () {
      const editor = {
        id: 'docs/editor',
        workspace: '.',
        launch: { command: 'true' },
      } as const satisfies Agent
      const writer = {
        id: 'docs/writer',
        workspace: '.',
        launch: { command: 'true' },
        under: { agent: editor, reason: "The editor reviews the writer's work." },
      } as const satisfies Agent
      const review = {
        id: 'docs/daily-review',
        state: 'ready',
        reportTo: editor,
        goals: ['Review the documentation each day.'],
        completion: 'all-steps-exhausted',
        steps: [
          {
            id: 'review',
            assignedTo: writer,
            goals: ['Review the documentation and report corrections.'],
          },
        ],
      } as const satisfies Mission
      expect(declarations(render(yield* loadDailyReview))).toEqual(
        declarations(encodeKdl({ agents: [editor, writer], missions: [review] })),
      )
    }),
  )

  it('renders the shared fixtures to their committed goldens', () => {
    expect(emit([prLanding()])).toBe(golden('pr-landing.kdl'))
    expect(upstreamRepin()).toBe(golden('upstream-repin.kdl'))
  })
})
