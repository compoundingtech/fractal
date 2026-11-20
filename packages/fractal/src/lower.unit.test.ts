import { describe, expect, it } from '@effect/vitest'
import { Effect, Schema } from 'effect'

import * as Fractal from './fractal.ts'
import { GraphCompileRefusal, lower } from './lower.ts'
import { MissionSchema, missionWire, type MissionInput } from './mod.ts'
import { fakeHost, journalOf, start } from './program-host.fixture.ts'
import type { ProgramExit } from './program-engine.ts'

const meta = { url: 'file:///tree/example/change/mission.ts' }
const owner = { id: 'example/owner' }
const worker = { id: 'example/worker' }
const bindings = { id: 'example/change', host: 'local', workers: { 'example/worker': { mission: 'example/seat-turn', goalInput: 'goal' } } }
const base = { state: 'ready' as const, reportTo: owner, goal: 'Deliver the bounded change.' }
const completed = (key: string): ProgramExit => ({ _tag: 'Success', value: { run: `mission-run/${key}`, status: 'completed' } })

// Runtime fixtures use public synthetic values. The three authoring excerpts below retain
// real step ids/edges while replacing goals, actors, hosts and command bodies with test data.
// No test loads or imports an external mission repository.
const diamond = () => {
  const inspect = Fractal.step({ id: 'inspect', assignedTo: worker, goal: 'Inspect the change.' })
  const code = Fractal.step({ id: 'code', assignedTo: worker, dependsOn: inspect, goal: 'Implement.' })
  const docs = Fractal.step({ id: 'docs', assignedTo: worker, dependsOn: inspect, goal: 'Document.' })
  const finish = Fractal.step({ id: 'finish', assignedTo: worker, dependsOn: [code, docs], goal: 'Report.' })
  return Fractal.mission({ meta, ...base, steps: [inspect, code, docs, finish] })
}
const renewal = () => {
  const approval = Fractal.step({ id: 'approve', agentless: true, gates: [Fractal.gate.human({ name: 'owner-consent', reviewer: Fractal.person('person/owner'), question: 'Renew access?' })] })
  const renew = Fractal.step({ id: 'renew', assignedTo: worker, dependsOn: approval, goal: 'Renew approved access.' })
  return Fractal.defineMission({ meta, intent: { ...base, steps: [approval, renew] } })
}
const review = (path = 'verdict', value = 'GO') => {
  const head = Fractal.input.text('head')
  const inspect = Fractal.step({ id: 'review', assignedTo: worker, goal: Fractal.t`Review ${head}.`, produces: {
    verdict: Fractal.product.resource({ kind: 'custom.example.verdict', subject: 'resource/mission-run/${ST_MISSION_RUN}/review/${input.head}/verdict', fields: { head: Fractal.t`${head}` } }),
    evidence: Fractal.product.resource({ kind: 'custom.example.evidence', fields: { present: true } }),
  } })
  const check = Fractal.step({ id: 'check', agentless: true, dependsOn: inspect, gates: [Fractal.gate.fieldIs({ name: 'verdict-go', subject: Fractal.t`${inspect.products.verdict}`, path, value })] })
  return Fractal.mission({ meta, ...base, inputs: [head], steps: [inspect, check] })
}
const verify = () => {
  const head = Fractal.input.text('head')
  const check = Fractal.step({ id: 'verify', assignedTo: worker, goal: 'Verify the exact head.', gates: [Fractal.gate.exec({ name: 'exact-head', host: 'local', workspace: '/work', command: Fractal.t`test "$HEAD" = "${head}"`, env: { HEAD: Fractal.t`${head}` } })] })
  return Fractal.mission({ meta, ...base, inputs: [head], steps: [check] })
}

const maintenanceExcerpt = () => Fractal.mission({ meta, ...base, steps: [
  { id: 'inventory-upstream', assignedTo: worker, goal: 'Inventory releases.' },
  { id: 'update-and-land', assignedTo: worker, dependsOn: [{ step: 'inventory-upstream', state: 'completed' }], goal: 'Update reviewed inputs.' },
  { id: 'verify-and-report', assignedTo: worker, dependsOn: [{ step: 'update-and-land', state: 'completed' }], goal: 'Verify and report.' },
] })
const renewalExcerpt = () => Fractal.defineMission({ meta, intent: { ...base, steps: [
  { id: 'precheck', assignedTo: worker, goal: 'Inspect required renewals.' },
  { id: 'restart-provisioning', assignedTo: worker, dependsOn: ['precheck'], goal: 'Provision the approved renewal.' },
  { id: 'observe-approvals-and-verify', assignedTo: worker, dependsOn: ['restart-provisioning'], goal: 'Verify external approval evidence.' },
] } })
const acceptanceExcerpt = () => Fractal.mission({ meta, ...base, assignedTo: worker, steps: [
  { id: 'implement', goal: 'Implement the selected change.' },
  { id: 'deterministic-interaction-proof', dependsOn: ['implement'], goal: 'Prove deterministic interactions.' },
  { id: 'independent-exact-head-review', dependsOn: ['deterministic-interaction-proof'], goal: 'Obtain independent exact-head review.' },
  { id: 'land-and-packaged-observation', dependsOn: ['independent-exact-head-review'], goal: 'Observe the reviewed artifact.' },
  { id: 'acceptance', dependsOn: ['land-and-packaged-observation'], goal: 'Check acceptance evidence.', gates: [Fractal.gate.exec({ name: 'evidence-check', command: 'test -f evidence.json', host: 'local', workspace: '/work' })] },
] })

const fixtures = [
  { name: 'real maintenance excerpt', make: maintenanceExcerpt, ids: ['inventory-upstream', 'update-and-land', 'verify-and-report'], edges: [[], ['inventory-upstream'], ['update-and-land']] },
  { name: 'real renewal excerpt', make: renewalExcerpt, ids: ['precheck', 'restart-provisioning', 'observe-approvals-and-verify'], edges: [[], ['precheck'], ['restart-provisioning']] },
  { name: 'real acceptance excerpt', make: acceptanceExcerpt, ids: ['implement', 'deterministic-interaction-proof', 'independent-exact-head-review', 'land-and-packaged-observation', 'acceptance'], edges: [[], ['implement'], ['deterministic-interaction-proof'], ['independent-exact-head-review'], ['land-and-packaged-observation']] },
  { name: 'maintenance diamond', make: diamond, ids: ['inspect', 'code', 'docs', 'finish'], edges: [[], ['inspect'], ['inspect'], ['code', 'docs']] },
  { name: 'approval renewal', make: renewal, ids: ['approve', 'renew'], edges: [[], ['approve']] },
  { name: 'review products', make: review, ids: ['review', 'check'], edges: [[], ['review']] },
  { name: 'worker verification', make: verify, ids: ['verify'], edges: [[]] },
]

describe('graph lowering golden parity', () => {
  it.each(fixtures)('$name preserves ids, edges and gates from the graph wire', ({ make, ids, edges }) => {
    const graph = make()
    const source = Schema.decodeUnknownSync(MissionSchema)(missionWire({ ...graph.intent, id: bindings.id } as MissionInput<string>))
    const compiled = lower(graph, bindings)
    expect(compiled.graph.map((step) => step.id)).toEqual(ids)
    expect(compiled.graph.map((step) => step.dependsOn)).toEqual(edges)
    expect(compiled.graph).toEqual(source.steps.map((step) => ({ id: step.id, dependsOn: (step.dependsOn ?? []).map((edge) => edge.step), gates: 'gates' in step ? step.gates ?? [] : [] })))
  })

  it.effect('preserves ordered multiline policy in the declaration and every worker instruction', () => Effect.gen(function* () {
    const target = Fractal.input.text('target')
    const constraints = [
      'Read-only inspection; no activation.\nRetain exact evidence and owner approval.',
      Fractal.t`Use the approved ${target} scope only.`,
    ]
    const graph = Fractal.mission({ meta, ...base, inputs: [target], constraints, steps: [
      Fractal.step({ id: 'inspect', assignedTo: worker, goal: 'Inspect.' }),
      Fractal.step({ id: 'report', assignedTo: worker, dependsOn: 'inspect', goal: 'Report.' }),
    ] })
    const compiled = lower(graph, bindings)
    const text = 'Read-only inspection; no activation.\nRetain exact evidence and owner approval.\nUse the approved ${input.target} scope only.'
    expect(compiled.intent.goal).toBe(`${base.goal}\n\nConstraints:\n${text}`)
    const seen: string[] = []
    const host = fakeHost({ payload: { target: 'fixture' }, child: async ({ key, input }) => {
      seen.push(key)
      expect(input).toEqual({ target: 'fixture', goal: `${key === 'inspect' ? 'Inspect.' : 'Report.'}\n\nConstraints:\n${text.replace('${input.target}', 'fixture')}` })
      return completed(key)
    } })
    expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
    expect(seen).toEqual(['inspect', 'report'])
  }))

  it.effect('preserves graph dollar escapes in policy and worker goals', () => Effect.gen(function* () {
    const target = Fractal.input.text('target')
    const graph = Fractal.mission({ meta, ...base, inputs: [target], constraints: [
      'Explain $${input.example} and $${ST_MISSION_RUN} literally.',
      'Keep $$${input.target} escaped; expand ${input.target}.',
    ], steps: [
      Fractal.step({ id: 'work', assignedTo: worker, goal: 'Document $${input.example}.' }),
    ] })
    const host = fakeHost({ payload: { target: 'fixture' }, child: async ({ key, input }) => {
      expect(input).toEqual({ target: 'fixture', goal: [
        'Document ${input.example}.',
        '',
        'Constraints:',
        'Explain ${input.example} and ${ST_MISSION_RUN} literally.',
        'Keep $${input.target} escaped; expand fixture.',
      ].join('\n') })
      return completed(key)
    } })
    expect(yield* start(lower(graph, bindings), host)).toEqual({ _tag: 'Success', value: null })
  }))

  it('treats empty constraints as absent and preserves native resources and completed ordering', () => {
    const resources = [{ id: 'example/repository', kind: 'vcs.repository' as const }]
    const graph = Fractal.mission({ meta, ...base, constraints: [], resources, concurrentRuns: 1, steps: [
      { id: 'first', agentless: true },
      { id: 'last', agentless: true, dependsOn: 'first' },
    ] })
    const compiled = lower(graph, bindings)
    expect(compiled.intent.goal).toBe(base.goal)
    expect(compiled.intent.resources).toEqual(resources)
    expect(compiled.intent.concurrentRuns).toBe(1)
    expect(compiled.graph.map((step) => step.dependsOn)).toEqual([[], ['first']])
  })

  it.effect('runs independent siblings concurrently and journals original worker ids', () => Effect.gen(function* () {
    const entered: string[] = []
    const release = Promise.withResolvers<void>()
    const journal: Array<readonly [string, ProgramExit]> = []
    const host = fakeHost({ payload: {}, child: async ({ key }) => {
      entered.push(key)
      if (key === 'code') await release.promise
      if (key === 'docs') release.resolve()
      const exit = completed(key)
      journal.push([key, exit])
      return exit
    } })
    const compiled = lower(diamond(), bindings)
    expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
    expect(entered).toEqual(['inspect', 'code', 'docs', 'finish'])
    expect(journal.map(([key]) => key).sort()).toEqual(['code', 'docs', 'finish', 'inspect'])
    const replay = fakeHost({ payload: {}, journal })
    expect(yield* start(compiled, replay)).toEqual({ _tag: 'Success', value: null })
    expect(replay.recorded).toEqual([])
  }))

  it.effect('requires the declared human approval before starting its dependent worker', () => Effect.gen(function* () {
    const events: string[] = []
    const resolutions: Record<string, ProgramExit> = {}
    const host = fakeHost({ payload: {}, ask: async (request) => {
      events.push(request.key)
      expect(request.for).toBe('person/owner')
      expect(request.request).toMatchObject({ version: 1, type: 'decision', answers: [ { id: 'approve', outcome: 'accept' }, { id: 'reject', outcome: 'decline' } ] })
      return resolutions[request.key] = { _tag: 'Success', value: { resolved_by: 'person/owner', answer: { type: 'decision', outcome: 'accept', id: 'approve' } } }
    }, child: async ({ key }) => { events.push(key); return resolutions[key] = completed(key) } })
    expect(yield* start(lower(renewal(), bindings), host)).toEqual({ _tag: 'Success', value: null })
    expect(events).toEqual(['approve', 'renew'])
    expect(Object.keys(resolutions)).toEqual(['approve', 'renew'])
    expect(yield* start(lower(renewal(), bindings), fakeHost({ payload: {}, journal: Object.entries(resolutions) }))).toEqual({ _tag: 'Success', value: null })
    for (const answer of [
      { resolved_by: 'person/owner', answer: { type: 'decision', outcome: 'decline', id: 'reject' } },
      { resolved_by: 'person/other', answer: { type: 'decision', outcome: 'accept', id: 'approve' } },
      { resolved_by: 'deadline' },
    ]) {
      const refused = fakeHost({ payload: {}, ask: async () => ({ _tag: 'Success', value: answer }) })
      expect(yield* start(lower(renewal(), bindings), refused)).toMatchObject({ _tag: 'Failure', error: { _tag: 'GraphGateFailed', step: 'approve' } })
    }
  }))

  it.effect('decodes multiple full resource subjects, checks fields and replays without children', () => Effect.gen(function* () {
    const compiled = lower(review(), bindings)
    const childJournal: Array<readonly [string, ProgramExit]> = []
    const host = fakeHost({ payload: { head: 'abc' }, child: async ({ key, mission, input }) => {
      expect({ key, mission, input }).toEqual({ key: 'review', mission: 'example/seat-turn', input: { head: 'abc', goal: 'Review abc.' } })
      const exit: ProgramExit = { _tag: 'Success', value: { run: 'mission-run/child', status: 'completed', products: {
        'resource/mission-run/child/review/abc/verdict': { kind: 'custom.example.verdict', head: 'abc', verdict: 'GO' },
        'resource/mission-run/child/review/evidence': { kind: 'custom.example.evidence', present: true },
      } } }
      childJournal.push([key, exit])
      return exit
    } })
    expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
    const journal = [...childJournal, ...journalOf(host.recorded)]
    expect(journal.map(([key]) => key)).toEqual(['review', 'review/product', 'check'])
    expect(yield* start(compiled, fakeHost({ payload: { head: 'abc' }, journal }))).toEqual({ _tag: 'Success', value: null })
    const noGo = journal.slice(0, 1).map(([key, exit]) => [key, exit._tag === 'Success' ? { ...exit, value: { ...(exit.value as object), products: {
      'resource/mission-run/child/review/abc/verdict': { kind: 'custom.example.verdict', head: 'abc', verdict: 'NO-GO' },
      'resource/mission-run/child/review/evidence': { kind: 'custom.example.evidence', present: true },
    } } } : exit] as const)
    const rejected = fakeHost({ payload: { head: 'abc' }, journal: noGo })
    expect(yield* start(compiled, rejected)).toMatchObject({ _tag: 'Failure', error: { _tag: 'GraphGateFailed', step: 'check', gate: 'verdict-go' } })
    for (const [path, value] of [['missing', 'GO'], ['constructor.name', 'Object']] as const) {
      const absent = fakeHost({ payload: { head: 'abc' }, journal: journal.slice(0, 1) })
      expect(yield* start(lower(review(path, value), bindings), absent)).toMatchObject({ _tag: 'Failure', error: { _tag: 'GraphGateFailed', step: 'check', gate: 'verdict-go' } })
    }
  }))

  it.effect('runs exec gates inside a journaled activity and refuses nonzero exits', () => Effect.gen(function* () {
    const commands: unknown[] = []
    const compiled = lower(verify(), bindings)
    const host = fakeHost({ payload: { head: 'abc' }, child: async ({ key }) => completed(key), spawn: async (argv, options) => {
      commands.push({ argv, options })
      return { code: 0, signal: null, stdout: '', stderr: '' }
    } })
    expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
    expect(commands).toEqual([{ argv: ['sh', '-c', 'test "$HEAD" = "abc"'], options: { cwd: '/work', env: { HEAD: 'abc' } } }])
    const journal = [['verify', completed('verify')] as const, ...journalOf(host.recorded)]
    expect(journal.map(([key]) => key)).toEqual(['verify', 'verify/gate/0'])
    expect(yield* start(compiled, fakeHost({ payload: { head: 'abc' }, journal, spawn: async () => { throw new Error('must not replay command') } }))).toEqual({ _tag: 'Success', value: null })
    const failed = fakeHost({ payload: { head: 'abc' }, child: async ({ key }) => completed(key), spawn: async () => ({ code: 1, signal: null, stdout: '', stderr: '' }) })
    expect(yield* start(compiled, failed)).toMatchObject({ _tag: 'Failure', error: { _tag: 'GraphGateFailed', step: 'verify', gate: 'exact-head' } })
    expect(failed.recorded.at(-1)).toMatchObject({ key: 'verify/gate/0', status: 'failed' })
  }))
})

describe('typed compile refusals', () => {
  it.each([
    ['targetAt', { targetAt: '2030-01-01T00:00:00Z' }],
    ['milestones', { milestones: [{ id: 'release', at: '2030-01-01T00:00:00Z' }] }],
  ] as const)('refuses %s instead of silently losing mission date metadata', (construct, fields) => {
    const graph = Fractal.mission({ meta, ...base, ...fields, steps: [{ id: 'work', agentless: true }] })
    expect(() => lower(graph, bindings)).toThrow(expect.objectContaining({ _tag: 'GraphCompileRefusal', construct }))
  })

  it.each([
    ['waitFor', { waitFor: Fractal.pinMission({ subject: diamond(), revision: 'a'.repeat(64) }) }], ['watch', { watch: {} }], ['observer', { observer: {} }],
    ['Timing', { timing: { _tag: 'Calendar', at: '09:00', timezone: 'UTC' } }],
    ['Review', { review: [{ _tag: 'Subject', subject: 'resource/example' }] }],
    ['waitUntil', { waitUntil: '2030-01-01T00:00:00Z' }],
  ] as const)('names %s and its graph step', (construct, fields) => {
    const graph = Fractal.defineMission({ meta, intent: { ...base, steps: [{ id: 'blocked', agentless: true, ...fields }] } })
    expect(() => lower(graph, bindings)).toThrow(GraphCompileRefusal)
    try { lower(graph, bindings) } catch (error) {
      expect(error).toMatchObject({ _tag: 'GraphCompileRefusal', step: 'blocked', construct })
    }
  })

  it('refuses missing worker bindings and remote exec hosts', () => {
    expect(() => lower(diamond(), { ...bindings, workers: {}, strictWorkers: true })).toThrow(expect.objectContaining({ construct: 'worker binding', step: 'inspect' }))
    expect(() => lower(verify(), { ...bindings, host: 'other' })).toThrow(expect.objectContaining({ construct: 'gate.exec.host', step: 'verify' }))
  })

  it('refuses publication approval lanes rather than treating them as resource lanes', () => {
    const graph = Fractal.mission({ meta, ...base, lanes: [{ name: 'review', entries: 'resource/example/change/', approver: 'person/owner' }], steps: [] })
    expect(() => lower(graph, bindings)).toThrow(expect.objectContaining({ construct: 'lanes', step: bindings.id }))
  })

  it.each([
    ['steps', () => ({ steps: [{ ...Fractal.step({ id: 'work', assignedTo: worker, goal: 'Work.' }) }] })],
    ['inputs', () => ({ inputs: [{ name: 'target', kind: 'text' as const, [Symbol()]: true }], steps: [] })],
    ['concurrentRuns', () => ({ concurrentRuns: 0, steps: [] })],
  ] as const)('reports invalid %s authoring data with its original schema path', (path, fields) => {
    const graph = Fractal.defineMission({ meta, intent: { ...base, ...fields() } })
    expect(() => lower(graph, bindings)).toThrow(expect.objectContaining({
      _tag: 'GraphCompileRefusal', step: bindings.id, construct: 'authoring boundary',
      reason: expect.stringContaining(path),
    }))
  })

  it('does not reclassify resolver defects or typed binding refusals as schema failures', () => {
    const ref = Fractal.agent({ meta: { url: 'file:///tree/example/worker/agent.ts' } })
    const graph = Fractal.defineMission({ meta, intent: { ...base, steps: [{ id: 'work', assignedTo: ref, goal: 'Work.' }] } })
    const defect = new Error('resolver defect')
    expect(() => lower(graph, { ...bindings, resolveAgent: () => { throw defect } })).toThrow(defect)
    expect(() => lower(graph, bindings)).toThrow(expect.objectContaining({ construct: 'agent binding', step: 'work' }))
  })

  it.effect('preserves a declared goal input through explicit instruction-input binding', () => Effect.gen(function* () {
    const goal = Fractal.input.text('goal')
    const graph = Fractal.mission({ meta, ...base, inputs: [goal], steps: [Fractal.step({ id: 'work', assignedTo: worker, goal: Fractal.t`Implement ${goal}.` })] })
    expect(() => lower(graph, bindings)).toThrow(expect.objectContaining({ construct: 'worker input binding', step: 'work' }))
    const compiled = lower(graph, { ...bindings, workers: { 'example/worker': { mission: 'example/seat-turn', goalInput: 'instructions' } } })
    const host = fakeHost({ payload: { goal: 'requested change' }, child: async ({ key, input }) => {
      expect(input).toEqual({ goal: 'requested change', instructions: 'Implement requested change.' })
      return completed(key)
    } })
    expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
  }))

  it('uses explicit source-reference resolution and refuses a missing resolver', () => {
    const ref = Fractal.agent({ meta: { url: 'file:///tree/example/worker/agent.ts' } })
    const graph = Fractal.defineMission({ meta, intent: { ...base, steps: [{ id: 'work', assignedTo: ref, goal: 'Work.' }] } })
    expect(() => lower(graph, bindings)).toThrow(expect.objectContaining({ construct: 'agent binding', step: 'work' }))
    expect(lower(graph, { ...bindings, resolveAgent: () => worker }).graph.map((step) => step.id)).toEqual(['work'])
  })

  it('refuses cyclic graphs at compile time rather than spinning the program', () => {
    const graph = Fractal.mission({ meta, ...base, steps: [{ id: 'a', agentless: true, dependsOn: 'b' }, { id: 'b', agentless: true, dependsOn: 'a' }] })
    expect(() => lower(graph, bindings)).toThrow(expect.objectContaining({ construct: 'dependsOn cycle', step: 'a' }))
  })
})

describe('standard native worker binding', () => {
  it.effect('defaults to the published standard mission with full product and input context', () => Effect.gen(function* () {
    const compiled = lower(review(), { id: bindings.id, host: 'local' })
    const host = fakeHost({ payload: { head: 'abc' }, child: async ({ key, mission, input }) => {
      expect({ key, mission }).toEqual({ key: 'review', mission: 'fractal/seat-turn' })
      expect(input.agent).toBe('agent/example/worker')
      expect(input.instructions).toBe('Review abc.')
      expect(JSON.parse(input.context!)).toEqual({ inputs: { head: 'abc' }, products: [
        { path: 'review/abc/verdict', kind: 'custom.example.verdict', fields: { kind: 'custom.example.verdict', head: 'abc' } },
        { path: 'review/evidence', kind: 'custom.example.evidence', fields: { kind: 'custom.example.evidence', present: true } },
      ] })
      return { _tag: 'Success', value: { run: 'mission-run/native-program', status: 'completed', result: {
        run: 'mission-run/real-episode', step: 'step-run/real-episode/work', summary: 'Reviewed.', evidence: ['work.completed'],
        products: {
          'review/abc/verdict': { kind: 'custom.example.verdict', head: 'abc', verdict: 'GO' },
          'review/evidence': { kind: 'custom.example.evidence', present: true },
        },
      } } }
    } })
    expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
  }))

  it.effect('keeps all colliding graph inputs inert and preserves instruction constraints', () => Effect.gen(function* () {
    const names = ['agent', 'instructions', 'context', 'goal'] as const
    const graph = Fractal.mission({ meta, ...base,
      inputs: names.map((name) => Fractal.input.text(name)),
      constraints: ['Preserve ${input.context}.'],
      steps: [Fractal.step({ id: 'work', assignedTo: worker, goal: 'Implement ${input.goal}.' })],
    })
    const original = { agent: 'not an assignee', instructions: 'not instructions', context: '$(not-shell)', goal: 'requested change' }
    const host = fakeHost({ payload: original, child: async ({ input }) => {
      expect(input).toEqual({
        agent: 'agent/example/worker', instructions: 'Implement requested change.\n\nConstraints:\nPreserve $(not-shell).',
        context: JSON.stringify({ inputs: original, products: [] }),
      })
      return { _tag: 'Success', value: { run: 'mission-run/program', status: 'completed', result: {
        run: 'mission-run/episode', step: 'step-run/episode/work', summary: 'Done.', evidence: [], products: {},
      } } }
    } })
    expect(yield* start(lower(graph, { id: bindings.id, host: 'local' }), host)).toEqual({ _tag: 'Success', value: null })
  }))

  it.effect('supports explicit context, agent and report mappings while retaining legacy overrides', () => Effect.gen(function* () {
    const graph = diamond()
    const host = fakeHost({ payload: {}, child: async ({ key, mission, input }) => {
      expect(mission).toBe('example/custom-turn')
      expect(input.assignee).toBe('agent/example/worker')
      expect(input.prompt).toBeDefined()
      expect(JSON.parse(input.data!)).toEqual({ inputs: {}, products: [] })
      return { _tag: 'Success', value: { run: `mission-run/${key}`, status: 'completed', result: {
        run: `mission-run/${key}-episode`, step: `step-run/${key}/work`, summary: 'Done.', evidence: [], products: {},
      } } }
    } })
    const compiled = lower(graph, { ...bindings, workers: { 'example/worker': {
      mission: 'example/custom-turn', goalInput: 'prompt', contextInput: 'data', agentInput: 'assignee', report: true,
    } } })
    expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
  }))

  it.effect('preserves prototype-named instruction and agent mappings as own input fields', () => Effect.gen(function* () {
    for (const mapping of [
      { goalInput: '__proto__', agentInput: 'assignee' },
      { goalInput: 'prompt', agentInput: '__proto__' },
    ]) {
      const host = fakeHost({ payload: {}, child: async ({ key, input }) => {
        expect(Object.hasOwn(input, mapping.goalInput)).toBe(true)
        expect(typeof input[mapping.goalInput]).toBe('string')
        expect(input[mapping.goalInput]!.length).toBeGreaterThan(0)
        expect(Object.hasOwn(input, mapping.agentInput)).toBe(true)
        expect(input[mapping.agentInput]).toBe('agent/example/worker')
        return completed(key)
      } })
      const compiled = lower(diamond(), { ...bindings, workers: { 'example/worker': {
        mission: 'example/custom-turn', contextInput: 'data', ...mapping,
      } } })
      expect(yield* start(compiled, host)).toEqual({ _tag: 'Success', value: null })
    }
  }))

  it.effect('does not accept an unreported child success even when no product was requested', () => Effect.gen(function* () {
    const host = fakeHost({ payload: {}, child: async ({ key }) => completed(key) })
    expect(yield* start(lower(diamond(), { id: bindings.id, host: 'local' }), host)).toMatchObject({
      _tag: 'Failure', error: { _tag: 'Defect' },
    })
  }))

  it('refuses ambiguous explicit protocol mappings', () => {
    expect(() => lower(diamond(), { ...bindings, workers: { 'example/worker': {
      mission: 'example/custom-turn', goalInput: 'same', contextInput: 'same',
    } } })).toThrow(expect.objectContaining({ construct: 'worker input binding' }))
  })
})
