import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import { NodeFileSystem } from '@effect/platform-node'
import { describe, expect, it } from '@effect/vitest'
import { Effect } from 'effect'

import { Fractal } from './index.ts'
import { emit, type KdlFileOutput } from './kdl.ts'
import { objectiveNode, type Objective, type ObjectiveInput } from './objective.ts'
import { loadSubjectTree } from './tree.ts'

/**
 * The Genie source lies outside this package's ESM `tsconfig` scope, so it is imported at runtime;
 * its exports are the `fractalKdl` output and the checked objectives it lowers.
 */
const genie = (await import(new URL('../../../slo/objectives.kdl.genie.ts', import.meta.url).href)) as {
  readonly default: KdlFileOutput
  readonly objectives: readonly (Objective & { readonly name: string })[]
}
const builtins = genie.default
const objectives = genie.objectives

const render = (input: ObjectiveInput & { readonly name: string }) => emit([objectiveNode(input)])

const rollout = {
  name: 'ops/change-rollout',
  about: 'A change, from its author marking it ready to every member that serves it',
  indicator: Fractal.indicator.interval({
    subjectPrefix: 'custom/acme/change/',
    start: { kind: 'custom.acme.change-ready', at: 'ready_at' },
    expect: { kind: 'custom.acme.change-merged', set: 'hosts', at: 'merged_at' },
    stop: { kind: 'custom.acme.change-applied', member: 'host', at: 'applied_at' },
    cancel: 'custom.acme.change-abandoned',
    pause: { kind: 'custom.acme.change-held', resume: 'custom.acme.change-released' },
    labels: ['class'],
    exclude: [{ field: 'exclusion', values: ['deferred-launch', 'deferred-runtime'] }],
  }),
  window: { span: '7d', minSamples: 20 },
  targets: [
    { stat: 'p50', within: '60s' },
    { stat: 'p95', within: '5m' },
  ],
  onBreach: Fractal.response.fault({
    owner: 'agent/ops/release-manager',
    backup: 'agent/ops/observability',
    condition: 'change-rollout-slo-missed',
  }),
} satisfies ObjectiveInput

describe('Fractal.objective lowering', () => {
  it('lowers an interval over claims as docs/st3/objectives.md declares it', () => {
    expect(render(rollout)).toBe(`version 2
objective "ops/change-rollout" {
  about "A change, from its author marking it ready to every member that serves it"
  indicator "interval" {
    subject-prefix "custom/acme/change/"
    start "custom.acme.change-ready" at="ready_at"
    expect "custom.acme.change-merged" at="merged_at" set="hosts"
    stop "custom.acme.change-applied" at="applied_at" member="host"
    cancel "custom.acme.change-abandoned"
    pause "custom.acme.change-held" resume="custom.acme.change-released"
    label "class"
    exclude "exclusion" "deferred-launch" "deferred-runtime"
  }
  window "7d" min-samples=20
  target "p50" within="60s"
  target "p95" within="5m"
  on-breach "fault" backup="agent/ops/observability" condition="change-rollout-slo-missed" owner="agent/ops/release-manager"
}
`)
  })

  it('lowers a minimal interval: no expect, accepted times, no window minimum or response', () => {
    expect(
      render({
        name: 'ops/build',
        about: 'A build',
        indicator: Fractal.indicator.interval({
          subjectPrefix: 'custom/acme/build/',
          start: { kind: 'custom.acme.build-started' },
          stop: { kind: 'custom.acme.build-finished' },
        }),
        window: { span: '1d' },
        targets: [{ stat: 'p99', within: '30m' }],
      }),
    ).toBe(`version 2
objective "ops/build" {
  about "A build"
  indicator "interval" {
    subject-prefix "custom/acme/build/"
    start "custom.acme.build-started"
    stop "custom.acme.build-finished"
  }
  window "1d"
  target "p99" within="30m"
}
`)
  })

  const native: ReadonlyArray<{
    readonly indicator: ObjectiveInput['indicator']
    readonly targets: ObjectiveInput['targets']
    readonly kdl: string
  }> = [
    {
      indicator: Fractal.indicator.latency({ paths: ['GET /v1/client/agents', 'stream agents'] }),
      targets: [
        { stat: 'p99', within: '300ms' },
        { stat: 'p99', within: '500ms', scope: 'remote' },
      ],
      kdl: '  indicator "latency" {\n    path "GET /v1/client/agents"\n    path "stream agents"\n  }\n  target "p99" within="300ms"\n  target "p99" scope="remote" within="500ms"\n',
    },
    {
      indicator: Fractal.indicator.share({ path: 'client/ios/live-share' }),
      targets: [{ stat: 'live-share', minPercent: 99.5 }],
      kdl: '  indicator "share" {\n    path "client/ios/live-share"\n  }\n  target "live-share" min-percent=99.5\n',
    },
    {
      indicator: Fractal.indicator.statement(),
      targets: [
        { stat: 'p99', within: '10ms' },
        { stat: 'max', within: '100ms' },
      ],
      kdl: '  indicator "statement"\n  target "p99" within="10ms"\n  target "max" within="100ms"\n',
    },
    {
      indicator: Fractal.indicator.transaction(),
      targets: [{ stat: 'max', within: '300ms' }],
      kdl: '  indicator "transaction"\n  target "max" within="300ms"\n',
    },
    {
      indicator: Fractal.indicator.writerHold(),
      targets: [{ stat: 'max', within: '300ms' }],
      kdl: '  indicator "writer-hold"\n  target "max" within="300ms"\n',
    },
    {
      indicator: Fractal.indicator.cpu({ idleMinMinutes: 10, loadMaxCores: 2.5 }),
      targets: [
        { stat: 'cores', max: 1 },
        { stat: 'idle-cores', max: 0.5 },
      ],
      kdl: '  indicator "cpu" {\n    idle-min-minutes 10\n    load-max-cores 2.5\n  }\n  target "cores" max=1\n  target "idle-cores" max=0.5\n',
    },
    {
      indicator: Fractal.indicator.database(),
      targets: [
        { stat: 'live-gb', max: 8 },
        { stat: 'growth-mb-per-day', max: 300 },
      ],
      kdl: '  indicator "database"\n  target "live-gb" max=8\n  target "growth-mb-per-day" max=300\n',
    },
    {
      indicator: Fractal.indicator.divergence(),
      targets: [{ stat: 'open', max: 0 }],
      kdl: '  indicator "divergence"\n  target "open" max=0\n',
    },
    {
      indicator: Fractal.indicator.ciDuration({ series: 'ci/merge-queue' }),
      targets: [{ stat: 'p50', within: '10m' }],
      kdl: '  indicator "ci-duration" {\n    series "ci/merge-queue"\n  }\n  target "p50" within="10m"\n',
    },
    {
      indicator: Fractal.indicator.ciCount({ series: 'ci/flakes' }),
      targets: [{ stat: 'per-hour', max: 0 }],
      kdl: '  indicator "ci-count" {\n    series "ci/flakes"\n  }\n  target "per-hour" max=0\n',
    },
  ]
  it.each(native)('lowers indicator $indicator.kind', ({ indicator, targets, kdl }) => {
    expect(render({ name: 'shipped', about: 'Measured', indicator, targets })).toBe(
      `version 2\nobjective "shipped" {\n  about "Measured"\n${kdl}}\n`,
    )
  })

  it('returns frozen, checked values and keeps an unnamed objective for the subject tree', () => {
    const { name: _, ...unnamed } = rollout
    const value = Fractal.objective(unnamed)
    expect(Object.isFrozen(value)).toBe(true)
    expect(value.name).toBeUndefined()
    expect(() => objectiveNode(value as ObjectiveInput & { name: string })).toThrow(/needs a `name`/)
  })
})

describe('Fractal.objective validation', () => {
  const base = rollout as ObjectiveInput
  const interval = rollout.indicator
  const latency = Fractal.indicator.latency({ paths: ['GET /v1/x'] })
  const cases: ReadonlyArray<{ readonly name: string; readonly input: unknown; readonly message: RegExp }> = [
    { name: 'a malformed name', input: { ...base, name: 'Ops/x' }, message: /lowercase words joined by `-`/ },
    { name: 'a name ending in a hyphen', input: { ...base, name: 'ops/x-' }, message: /lowercase words/ },
    { name: 'an unknown field', input: { ...base, owner: 'x' }, message: /unknown field `owner`/ },
    { name: 'no about', input: { ...base, about: undefined }, message: /has no `about`/ },
    { name: 'an empty about', input: { ...base, about: '  ' }, message: /`about` is empty/ },
    { name: 'an unknown indicator', input: { ...base, indicator: { kind: 'ratio' } }, message: /unknown indicator `ratio`/ },
    { name: 'an empty latency', input: { name: 'x', about: 'a', indicator: { kind: 'latency', paths: [] }, targets: [{ stat: 'p99', within: '1s' }] }, message: /needs a `path`/ },
    { name: 'an extra indicator field', input: { name: 'x', about: 'a', indicator: { kind: 'statement', path: 'p' }, targets: [] }, message: /indicator `statement` has an unknown field `path`/ },
    { name: 'a fractional idle minimum', input: { name: 'cpu', about: 'a', indicator: { kind: 'cpu', idleMinMinutes: 1.5, loadMaxCores: 1 }, targets: [] }, message: /idleMinMinutes is not a positive integer/ },
    { name: 'an idle minimum over an hour', input: { name: 'cpu', about: 'a', indicator: Fractal.indicator.cpu({ idleMinMinutes: 61, loadMaxCores: 1 }), targets: [{ stat: 'cores', max: 1 }, { stat: 'idle-cores', max: 0.5 }] }, message: /idle minimum is not between 1 and 60/ },
    { name: 'idle cores above cores', input: { name: 'cpu', about: 'a', indicator: Fractal.indicator.cpu({ idleMinMinutes: 10, loadMaxCores: 1 }), targets: [{ stat: 'cores', max: 1 }, { stat: 'idle-cores', max: 2 }] }, message: /idle CPU target is above/ },
    { name: 'a non-CI series', input: { name: 'x', about: 'a', indicator: { kind: 'ci-count', series: 'merge-queue' }, targets: [] }, message: /is not `ci\/` and lowercase words/ },
    { name: 'a relative subject prefix', input: { ...base, indicator: { ...interval, subjectPrefix: '/custom/' } }, message: /must name subjects and end with `\/`/ },
    { name: 'a prefix without a slash', input: { ...base, indicator: { ...interval, subjectPrefix: 'custom' } }, message: /must name subjects/ },
    { name: 'no stop', input: { ...base, indicator: { ...interval, stop: undefined } }, message: /an interval needs `stop`/ },
    { name: 'expect without stop member', input: { ...base, indicator: { ...interval, stop: { kind: 'custom.acme.change-applied' } } }, message: /with `expect`, `stop` needs `member`/ },
    { name: 'a label "member" without stop member', input: { ...base, indicator: { ...interval, expect: undefined, stop: { kind: 'k' }, labels: ['member'] } }, message: /`label "member"` needs `stop.member`/ },
    { name: 'a time field that is not a field', input: { ...base, indicator: { ...interval, start: { kind: 'k', at: 'ready at' } } }, message: /`at` is not a field name/ },
    { name: 'a repeated label', input: { ...base, indicator: { ...interval, labels: ['class', 'class'] } }, message: /label "class" is not a field, or is repeated/ },
    { name: 'an exclude without values', input: { ...base, indicator: { ...interval, exclude: [{ field: 'exclusion', values: [] }] } }, message: /needs a field and at least one value/ },
    { name: 'a pause without resume', input: { ...base, indicator: { ...interval, pause: { kind: 'held', resume: '' } } }, message: /`pause` needs `resume`/ },
    { name: 'a claim kind in two roles', input: { ...base, indicator: { ...interval, cancel: 'custom.acme.change-ready' } }, message: /claim kind "custom.acme.change-ready" has two roles/ },
    { name: 'an interval without a window', input: { ...base, window: undefined }, message: /an interval needs a `window`/ },
    { name: 'a window on native telemetry', input: { name: 'x', about: 'a', indicator: latency, window: { span: '1d' }, targets: [{ stat: 'p99', within: '1s' }] }, message: /only an interval takes a `window`/ },
    { name: 'a zero window', input: { ...base, window: { span: '0d' } }, message: /must be positive/ },
    { name: 'a malformed duration', input: { ...base, targets: [{ stat: 'p95', within: '5 minutes' }] }, message: /invalid duration `5 minutes`/ },
    { name: 'a zero minimum sample count', input: { ...base, window: { span: '1d', minSamples: 0 } }, message: /minSamples is not a positive integer/ },
    { name: 'two bounds', input: { ...base, targets: [{ stat: 'p95', within: '5m', max: 1 }] }, message: /needs one bound/ },
    { name: 'no target', input: { ...base, targets: [] }, message: /has no `target`/ },
    { name: 'a repeated target', input: { ...base, targets: [{ stat: 'p95', within: '5m' }, { stat: 'p95', within: '6m' }] }, message: /repeats target `p95`/ },
    { name: 'a non-percentile interval stat', input: { ...base, targets: [{ stat: 'p100', within: '5m' }] }, message: /no target `p100` with that bound/ },
    { name: 'a remote interval target', input: { ...base, targets: [{ stat: 'p95', within: '5m', scope: 'remote' }] }, message: /only a latency target has a remote scope/ },
    { name: 'an unknown scope', input: { ...base, targets: [{ stat: 'p95', within: '5m', scope: 'local' }] }, message: /unknown scope "local"/ },
    { name: 'a missing required stat', input: { name: 'sql', about: 'a', indicator: Fractal.indicator.statement(), targets: [{ stat: 'p99', within: '10ms' }] }, message: /needs target `max`/ },
    { name: 'a statement max below p99', input: { name: 'sql', about: 'a', indicator: Fractal.indicator.statement(), targets: [{ stat: 'p99', within: '10ms' }, { stat: 'max', within: '5ms' }] }, message: /max is below its p99/ },
    { name: 'a remote latency below local', input: { name: 'x', about: 'a', indicator: latency, targets: [{ stat: 'p99', within: '1s' }, { stat: 'p99', within: '500ms', scope: 'remote' }] }, message: /remote latency is below its own/ },
    { name: 'a fractional divergence max', input: { name: 'x', about: 'a', indicator: Fractal.indicator.divergence(), targets: [{ stat: 'open', max: 0.5 }] }, message: /no target `open` with that bound/ },
    { name: 'a min-percent over 100', input: { name: 'x', about: 'a', indicator: Fractal.indicator.share({ path: 'p' }), targets: [{ stat: 'live-share', minPercent: 101 }] }, message: /is not between 0 and 100/ },
    { name: 'a response on native telemetry', input: { name: 'x', about: 'a', indicator: latency, targets: [{ stat: 'p99', within: '1s' }], onBreach: rollout.onBreach }, message: /only an interval takes `onBreach`/ },
    { name: 'an owner that is not an agent', input: { ...base, onBreach: { ...rollout.onBreach, owner: 'person/ada' } }, message: /`owner` is not an agent/ },
    { name: 'a backup equal to the owner', input: { ...base, onBreach: { ...rollout.onBreach, backup: rollout.onBreach.owner } }, message: /`backup` is an agent other than the owner/ },
    { name: 'a malformed condition', input: { ...base, onBreach: { ...rollout.onBreach, condition: 'Missed' } }, message: /needs `condition`/ },
    { name: 'a response other than fault', input: { ...base, onBreach: { ...rollout.onBreach, kind: 'page' } }, message: /`onBreach` is `fault`/ },
  ]
  it.each(cases)('refuses $name', ({ input, message }) => {
    expect(() => Fractal.objective(input as ObjectiveInput)).toThrow(message)
  })

  it('lets the expect kind also start the sample', () => {
    const mergeToLive = { ...base, indicator: { ...interval, start: { kind: 'custom.acme.change-merged', at: 'merged_at' } } }
    expect(Fractal.objective(mergeToLive as ObjectiveInput).indicator).toMatchObject({ start: { kind: 'custom.acme.change-merged' } })
  })

  it('names the objective in its errors', () => {
    expect(() => Fractal.objective({ ...base, targets: [] })).toThrow(
      'objective `ops/change-rollout`: has no `target`',
    )
  })
})

describe('the objectives Fractal ships', () => {
  it('render slo/objectives.kdl from slo/objectives.kdl.genie.ts', () => {
    const committed = readFileSync(new URL('../../../slo/objectives.kdl', import.meta.url), 'utf8')
    expect(committed).toBe(
      `// Generated file - DO NOT EDIT\n// Source: objectives.kdl.genie.ts\n${builtins.stringify()}`,
    )
    expect(builtins.validate()).toEqual([])
  })

  it('are the 21 native objectives, one segment each, so none is declarable', () => {
    expect(objectives.map(({ name }) => name)).toEqual([
      'person-read',
      'person-read-fresh',
      'projection-fresh',
      'write-ack',
      'terminal-attach',
      'sql-statement',
      'transaction',
      'writer-hold',
      'cpu',
      'ios-open-to-live',
      'ios-connect',
      'ios-message-ack',
      'ios-conversation-open',
      'ios-terminal-open',
      'ios-recover',
      'ios-live-share',
      'projection-divergence',
      'database',
      'merge-latency',
      'ci-critical-path',
      'flakes',
    ])
    expect(objectives.some(({ indicator, name }) => indicator.kind === 'interval' || name.includes('/'))).toBe(
      false,
    )
  })
})

const treeModule = new URL('./tree.ts', import.meta.url).href
const indexModule = new URL('./index.ts', import.meta.url).href

const fixture = (files: Readonly<Record<string, string>>): string => {
  const root = mkdtempSync(join(tmpdir(), 'fractal-objective-'))
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    writeFileSync(join(root, path), text)
  }
  return root
}

const load = (root: string) =>
  loadSubjectTree({ root, context: { prefix: 'custom/acme/change/' } }).pipe(Effect.provide(NodeFileSystem.layer))

const declared = `{
  about: 'A change, from ready to applied everywhere',
  indicator: Fractal.indicator.interval({
    subjectPrefix: 'custom/acme/change/',
    start: { kind: 'custom.acme.change-ready' },
    stop: { kind: 'custom.acme.change-applied' },
  }),
  window: { span: '7d' },
  targets: [{ stat: 'p95', within: '5m' }],
}`

const header = `import { Fractal } from '${indexModule}'\nimport { defineAgent, defineObjective } from '${treeModule}'\n`

describe('objective subject trees', () => {
  it.effect('declare objective/<path> from objective.ts and objective.kdl', () =>
    Effect.gen(function* () {
      const kdl = render({ ...rollout, name: 'ops/kdl' })
      const root = fixture({
        'ops/release-manager/agent.ts': `${header}export default defineAgent({ meta: import.meta, intent: { workspace: '/srv/ops' } })`,
        'ops/change-rollout/objective.ts': `${header}import owner from '../release-manager/agent.ts'
export default Fractal.objective({ ...${declared}, onBreach: Fractal.response.fault({ owner, backup: 'agent/ops/observability', condition: 'change-rollout-slo-missed' }) })`,
        'ops/factory/objective.ts': `${header}export default defineObjective({ meta: import.meta, intent: (ctx: { prefix: string }) => ({ ...${declared}, name: 'ops/factory', indicator: Fractal.indicator.interval({ subjectPrefix: ctx.prefix, start: { kind: 'a' }, stop: { kind: 'b' } }) }) })`,
        'ops/kdl/objective.kdl': kdl,
      })
      const subjects = (yield* load(root)).filter(({ kind }) => kind === 'objective')
      expect(subjects.map(({ id, path, declaration }) => ({ id, path, tag: declaration._tag }))).toEqual([
        { id: 'ops/change-rollout', path: 'ops/change-rollout/objective.ts', tag: 'Module' },
        { id: 'ops/factory', path: 'ops/factory/objective.ts', tag: 'Module' },
        { id: 'ops/kdl', path: 'ops/kdl/objective.kdl', tag: 'Kdl' },
      ])
      const [rolloutSubject, factory, kdlSubject] = subjects
      if (rolloutSubject?.kind !== 'objective' || rolloutSubject.declaration._tag !== 'Module') {
        return yield* Effect.die('objective.ts did not load')
      }
      expect(rolloutSubject.declaration.intent.onBreach).toEqual({
        kind: 'fault',
        owner: 'agent/ops/release-manager',
        backup: 'agent/ops/observability',
        condition: 'change-rollout-slo-missed',
      })
      expect(emit([objectiveNode({ ...rolloutSubject.declaration.intent, name: rolloutSubject.id })])).toContain(
        'on-breach "fault" backup="agent/ops/observability" condition="change-rollout-slo-missed" owner="agent/ops/release-manager"',
      )
      expect(factory?.declaration).toMatchObject({
        _tag: 'Module',
        intent: { name: 'ops/factory', indicator: { subjectPrefix: 'custom/acme/change/' } },
      })
      expect(kdlSubject?.declaration).toEqual({ _tag: 'Kdl', text: kdl })
    }),
  )

  const rejections: ReadonlyArray<{
    readonly name: string
    readonly files: Readonly<Record<string, string>>
    readonly message: string
  }> = [
    {
      name: 'a name other than the path',
      files: { 'ops/x/objective.ts': `${header}export default Fractal.objective({ ...${declared}, name: 'ops/y' })` },
      message: 'Declared name ops/y must equal the subject directory ops/x',
    },
    {
      name: 'native telemetry',
      files: {
        'ops/x/objective.ts': `${header}export default Fractal.objective({ about: 'a', indicator: Fractal.indicator.statement(), targets: [{ stat: 'p99', within: '10ms' }, { stat: 'max', within: '100ms' }] })`,
      },
      message: 'A declared objective is an interval over claims named NAMESPACE/NAME; objective/ops/x is not',
    },
    {
      name: 'a one-segment name',
      files: { 'x/objective.ts': `${header}export default ${declared}` },
      message: 'objective/x is not',
    },
    {
      name: 'a path that is not an objective name',
      files: { 'ops/Upper/objective.ts': `${header}export default ${declared}` },
      message: 'Invalid objective declaration',
    },
    {
      name: 'an invalid plain intent',
      files: { 'ops/x/objective.ts': `${header}export default { ...${declared}, window: undefined }` },
      message: 'Invalid objective declaration',
    },
    {
      name: 'an owner reference to a mission',
      files: {
        'ops/x/objective.ts': `${header}export default Fractal.objective({ ...${declared}, onBreach: { kind: 'fault', owner: { _tag: 'SubjectRef', kind: 'mission', url: new URL('../m/mission.ts', import.meta.url).href }, condition: 'missed' } })`,
      },
      message: 'is not an agent',
    },
    {
      name: 'a KDL objective over native telemetry',
      files: {
        'ops/x/objective.kdl': 'version 2\nobjective "ops/x" {\n  about "a"\n  indicator "transaction"\n  target "max" within="1s"\n}\n',
      },
      message: 'objective/ops/x is not',
    },
    {
      name: 'a KDL objective under another name',
      files: { 'ops/x/objective.kdl': 'version 2\nobjective "ops/y" {\n}\n' },
      message: 'exactly one `objective "ops/x"` node',
    },
    {
      name: 'a KDL objective under a path that is not an objective name',
      files: {
        'ops/Upper/objective.kdl': 'version 2\nobjective "ops/Upper" {\n  indicator "interval"\n}\n',
      },
      message: 'objective/ops/Upper is not',
    },
  ]
  it.effect.each(rejections)('rejects $name', ({ files, message }) =>
    Effect.gen(function* () {
      const error = yield* load(fixture(files)).pipe(Effect.flip)
      const cause = error.cause instanceof Error ? ` ${error.cause.message}` : ''
      expect(`${error.message}${cause}`).toContain(message)
    }),
  )
})
