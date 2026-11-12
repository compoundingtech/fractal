import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'
import { parseSync } from 'oxc-parser'

import {
  sourceMap,
  type Frame,
  type SourceMapDocument,
  type SourceRead,
  type Span,
} from './source-map.ts'

const modUrl = new URL('./mod.ts', import.meta.url).href
const treeUrl = new URL('./tree.ts', import.meta.url).href
const fixtureScript = fileURLToPath(new URL('./source-map-trace.fixture.ts', import.meta.url))

const files: Readonly<Record<string, string>> = {
  'team/policy.ts': [
    'export const reviews = {',
    "  code: { goal: 'Review the change against the conventions.' },",
    '} as const',
    "export const rules = { push: 'Never force-push.' }",
    "export const shared = { inner: { value: 'kept' } }",
    '',
  ].join('\n'),
  'team/steps.ts': [
    `import { step } from '${modUrl}'`,
    "import { reviews } from './policy.ts'",
    '',
    "export const reviewStep = (kind: keyof typeof reviews, id: string) =>",
    "  step({ id, assignedTo: { id: 'team/owner' }, goal: reviews[kind].goal })",
    '',
  ].join('\n'),
  'team/owner/agent.ts': [
    `import { defineAgent } from '${treeUrl}'`,
    '',
    'const declared = defineAgent({',
    '  meta: import.meta,',
    "  intent: { workspace: '/srv/team', restart: 'always' },",
    '})',
    'export default declared',
    '',
  ].join('\n'),
  'team/review/mission.ts': [
    `import { defineMission } from '${treeUrl}'`,
    `import { step } from '${modUrl}'`,
    "import owner from '../owner/agent.ts'",
    "import { rules, shared } from '../policy.ts'",
    "import { reviewStep } from '../steps.ts'",
    '',
    "const read = step({ id: 'read', assignedTo: { id: 'team/owner' }, goal: 'Read the change.' })",
    'const notes = { push: rules.push }',
    "if (shared.inner !== shared.inner) throw new Error('a traced literal lost its identity')",
    '',
    'export default defineMission({',
    '  meta: import.meta,',
    '  intent: {',
    "    state: 'ready',",
    '    reportTo: owner,',
    "    goal: 'Review one change.',",
    '    constraints: [notes.push],',
    "    steps: [read, reviewStep('code', 'code-review')],",
    '  },',
    '})',
    '',
  ].join('\n'),
}

/** Writes `files` below a fresh real root. */
const fixture = (tree: Readonly<Record<string, string>>): string => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'fractal-source-map-')))
  for (const [path, text] of Object.entries(tree)) {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    writeFileSync(join(root, path), text)
  }
  return root
}

type Rendered = {
  readonly subjects: ReadonlyArray<{ readonly subject: string; readonly file: string; readonly kdl: string }>
  readonly reads: readonly SourceRead[]
}

/** Loads and renders the tree in Bun, which the read trace needs. */
const render = (root: string, mode: 'plain' | 'trace'): Rendered => {
  // `nix develop .#web` puts bun on PATH.
  const result = spawnSync('bun', [fixtureScript, root, mode], { encoding: 'utf8' })
  expect(result.stderr).toBe('')
  expect(result.status).toBe(0)
  return JSON.parse(result.stdout) as Rendered
}

const locateIn = (root: string) => (file: string) =>
  file.startsWith(`${root}/`) === true
    ? { repository: 'acme/team', path: relative(root, file) }
    : undefined

/** `path:line:column` of a span's start, for readable assertions. */
const at = (document: SourceMapDocument, span: Span) =>
  `${document.files[span.file]!.path}:${span.start.line}:${span.start.column}`

const node = (document: SourceMapDocument, subject: string, path: string) =>
  document.subjects[subject]!.nodes.find((entry) => entry.kdl === path)

const roles = (frames: readonly Frame[] | undefined) => (frames ?? []).map((frame) => frame.role)

describe('sourceMap', () => {
  it('renders byte-identical KDL with and without the read trace', () => {
    const root = fixture(files)
    const plain = render(root, 'plain')
    const traced = render(root, 'trace')
    expect(traced.subjects).toEqual(plain.subjects)
    expect(traced.reads.length).toBeGreaterThan(0)
  })

  it('locates declarations, steps and values: declaration → helper → value', () => {
    const root = fixture(files)
    const { subjects, reads } = render(root, 'trace')
    const { document, coverage } = sourceMap({ parse: parseSync, subjects, reads, locate: locateIn(root), texts: true })
    const mission = 'mission/team/review'
    const agent = 'agent/team/owner'

    expect(document.schema).toBe('fractal.source-map/v1')
    expect(Object.keys(document.subjects).toSorted()).toEqual([agent, mission])
    expect(coverage.orphanNodes).toBe(0)
    for (const file of document.files) {
      expect(file.repository).toBe('acme/team')
      expect(file.blob).toMatch(/^[0-9a-f]{40}$/u)
      expect(file.sha256).toMatch(/^[0-9a-f]{64}$/u)
      expect(file.text).toBe(files[file.path])
    }
    const review = document.subjects[mission]!
    expect(document.files[review.declaration]!.path).toBe('team/review/mission.ts')
    expect(review.inputs.map((id) => document.files[id]!.path).toSorted()).toEqual([
      'team/owner/agent.ts',
      'team/policy.ts',
      'team/review/mission.ts',
      'team/steps.ts',
    ])

    // An agent's declaration is its constructor call, not `export default declared`.
    const seat = node(document, agent, 'agent[team/owner]')!
    expect(at(document, seat.span)).toBe('team/owner/agent.ts:3:18')
    expect(seat.span.end.line).toBe(6)
    expect(at(document, node(document, agent, 'agent[team/owner]/workspace[0]')!.span)).toBe('team/owner/agent.ts:5:13')

    expect(at(document, node(document, mission, 'mission[team/review]')!.span)).toBe('team/review/mission.ts:11:16')
    expect(at(document, node(document, mission, 'mission[team/review]/goal[0]')!.span)).toBe('team/review/mission.ts:16:5')
    const read = node(document, mission, 'mission[team/review]/step[read]')!
    expect(at(document, read.span)).toBe('team/review/mission.ts:7:19')
    expect(roles(read.frames)).toEqual(['constructor'])
    expect(at(document, node(document, mission, 'mission[team/review]/step[read]/goal[0]')!.span)).toBe('team/review/mission.ts:7:67')

    // A constant read through a literal in the declaration module: declaration → value.
    const constraint = node(document, mission, 'mission[team/review]/constraint[0]')!
    expect(at(document, constraint.span)).toBe('team/policy.ts:4:24')
    expect(roles(constraint.frames)).toEqual(['declaration', 'value'])
    const declared = constraint.frames![0]!
    expect(declared.start.line).toBe(8)
    // The trace inserted text before this read on its line; the column is the authored one.
    const authored = files['team/review/mission.ts']!.split('\n')[7]!
    expect(authored.slice(declared.start.column - 1)).toMatch(/^(rules\.)?push/u)
    expect(constraint.frames![1]).toMatchObject({ precision: 'exact', end: { line: 4, column: 49 } })

    // A step a helper builds, and the value the helper reads: declaration → helper → value.
    const helperStep = node(document, mission, 'mission[team/review]/step[code-review]')!
    expect(at(document, helperStep.span)).toBe('team/steps.ts:5:3')
    expect(roles(helperStep.frames)).toEqual(['declaration', 'helper'])
    expect(at(document, helperStep.frames![0]!)).toMatch(/^team\/review\/mission\.ts:18:/u)
    const goal = node(document, mission, 'mission[team/review]/step[code-review]/goal[0]')!
    expect(at(document, goal.span)).toBe('team/policy.ts:2:11')
    expect(roles(goal.frames)).toEqual(['declaration', 'helper', 'value'])
    expect(goal.frames!.map((frame) => `${document.files[frame.file]!.path}:${frame.start.line}`)).toEqual([
      'team/review/mission.ts:18',
      'team/steps.ts:5',
      'team/policy.ts:2',
    ])
    expect(goal.kdlLines[0]).toBe(goal.kdlLines[1])
    const kdl = subjects.find((entry) => entry.subject === mission)!.kdl.split('\n')
    expect(kdl[goal.kdlLines[0] - 1]!.trim()).toBe('goal "Review the change against the conventions."')
  })

  it('binds each traced read to its own call site and keeps keyed nodes on their declaration', () => {
    const root = fixture({
      ...files,
      'team/twice/mission.ts': [
        `import { defineMission } from '${treeUrl}'`,
        `import { step } from '${modUrl}'`,
        "import owner from '../owner/agent.ts'",
        "import { reviewStep } from '../steps.ts'",
        '',
        "const ids = { first: 'first', second: 'second' }",
        '',
        'export default defineMission({',
        '  meta: import.meta,',
        '  intent: {',
        "    state: 'ready',",
        '    reportTo: owner,',
        "    goal: 'Review twice.',",
        '    steps: [',
        "      reviewStep('code', 'one'),",
        "      reviewStep('code', 'two'),",
        "      step({ id: ids.second, assignedTo: { id: 'team/owner' }, goal: 'Second.' }),",
        "      reviewStep('code', ids.first),",
        '    ],',
        '  },',
        '})',
        '',
      ].join('\n'),
    })
    const { subjects, reads } = render(root, 'trace')
    const { document } = sourceMap({ parse: parseSync, subjects, reads, locate: locateIn(root) })
    const mission = 'mission/team/twice'
    const lines = (path: string) =>
      node(document, mission, path)!.frames!.map((frame) => `${document.files[frame.file]!.path}:${frame.start.line}`)

    // A helper called twice: each step's goal names its own call, not the first one.
    expect(lines('mission[team/twice]/step[one]/goal[0]')).toEqual(['team/twice/mission.ts:15', 'team/steps.ts:5', 'team/policy.ts:2'])
    expect(lines('mission[team/twice]/step[two]/goal[0]')).toEqual(['team/twice/mission.ts:16', 'team/steps.ts:5', 'team/policy.ts:2'])
    expect(lines('mission[team/twice]/step[one]')[0]).toBe('team/twice/mission.ts:15')
    expect(lines('mission[team/twice]/step[two]')[0]).toBe('team/twice/mission.ts:16')

    // A step whose id is read from a constant stays spanned by its `step({…})` declaration.
    const second = node(document, mission, 'mission[team/twice]/step[second]')!
    expect(at(document, second.span)).toBe('team/twice/mission.ts:17:12')
    expect(roles(second.frames)).toContain('value')

    // The key passed to a helper: the helper's `step({…})` spans it, bounded by that call.
    const first = node(document, mission, 'mission[team/twice]/step[first]')!
    expect(at(document, first.span)).toBe('team/steps.ts:5:3')
    expect(lines('mission[team/twice]/step[first]')).toEqual(['team/twice/mission.ts:18', 'team/steps.ts:5'])
    expect(lines('mission[team/twice]/step[first]/goal[0]')).toEqual(['team/twice/mission.ts:18', 'team/steps.ts:5', 'team/policy.ts:2'])
  })

  it('keys agents, missions, steps and gates by name and other nodes by sibling index', () => {
    const root = fixture({
      'team/ship/mission.ts': [
        'export default {',
        "  id: 'team/ship',",
        "  constraints: ['One.', 'Two.'],",
        '  steps: [',
        "    { id: 'build' },",
        "    { id: 'ship', gates: [{ name: 'Ship it' }] },",
        '  ],',
        '}',
        '',
      ].join('\n'),
    })
    const kdl = [
      'version 2',
      'mission "team/ship" state="ready" {',
      '  constraint "One."',
      '  constraint "Two."',
      '  step "build" {',
      '  }',
      '  step "ship" {',
      '    depends-on {',
      '      step "build" "completed"',
      '    }',
      '    gate "Ship it" mode="approve" type="human" {',
      '      reviewer "person/ada"',
      '    }',
      '  }',
      '}',
      '',
    ].join('\n')
    const { document, coverage } = sourceMap({
      parse: parseSync,
      subjects: [{ subject: 'mission/team/ship', file: join(root, 'team/ship/mission.ts'), kdl }],
      locate: locateIn(root),
    })
    const nodes = document.subjects['mission/team/ship']!.nodes
    expect(nodes.map((entry) => `${entry.kdl} ${entry.kdlLines.join('-')} ${at(document, entry.span)}`)).toEqual([
      'mission[team/ship] 2-15 team/ship/mission.ts:1:1',
      'mission[team/ship]/constraint[0] 3-3 team/ship/mission.ts:3:17',
      'mission[team/ship]/constraint[1] 4-4 team/ship/mission.ts:3:25',
      'mission[team/ship]/step[build] 5-6 team/ship/mission.ts:5:5',
      'mission[team/ship]/step[ship] 7-14 team/ship/mission.ts:6:5',
      'mission[team/ship]/step[ship]/gate[Ship it] 11-13 team/ship/mission.ts:6:27',
    ])
    expect(document.files[0]).not.toHaveProperty('text')
    expect(coverage).toMatchObject({ kdlNodes: 9, exactNodes: 6, enclosedNodes: 3, orphanNodes: 0, declaredSteps: 2, mappedSteps: 2 })
  })
})
