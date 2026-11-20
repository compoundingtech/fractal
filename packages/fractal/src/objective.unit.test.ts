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

// Rust's emitted-row test checks this shared corpus against every shipped built-in row.
const nativeRows = JSON.parse(
  readFileSync(new URL('../../../fixtures/objective-native-rows.json', import.meta.url), 'utf8'),
) as { readonly rows: readonly string[]; readonly valid: readonly string[]; readonly invalid: readonly string[] }

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

const throughput = {
  name: 'factory/work-throughput',
  about: "Factory-authored pull requests merged into the roster's default branches",
  indicator: Fractal.indicator.count({
    subjectPrefix: 'custom/factory/merged-pr/',
    event: { kind: 'custom.factory.pr-merged', at: 'merged_at' },
    labels: ['repository'],
    exclude: [{ field: 'author', values: ['renovate'] }],
  }),
  window: { span: '24h' },
  targets: [{ stat: 'per-hour', min: 10 }],
} satisfies ObjectiveInput

/** One claim per run is the whole sample, from its creation to its completion. */
const runs = {
  name: 'acme/merge-latency',
  about: "A merge group's run, from created to completed",
  indicator: Fractal.indicator.interval({
    subjectPrefix: 'github-workflow-run/acme/repo/',
    start: { kind: 'github.workflow-run', at: 'created_at' },
    stop: { kind: 'github.workflow-run', at: 'completed_at' },
    only: [
      { field: 'event', values: ['merge_group'] },
      { field: 'attempt', values: ['1'] },
    ],
  }),
  window: { span: '1d' },
  targets: [{ stat: 'p95', within: '10m' }],
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

  it('lowers a count over claims as docs/st3/objectives.md declares it', () => {
    expect(render(throughput)).toBe(`version 2
objective "factory/work-throughput" {
  about "Factory-authored pull requests merged into the roster's default branches"
  indicator "count" {
    subject-prefix "custom/factory/merged-pr/"
    event "custom.factory.pr-merged" at="merged_at"
    label "repository"
    exclude "author" "renovate"
  }
  window "24h"
  target "per-hour" min=10
}
`)
    const flakes = { ...throughput, name: 'factory/flakes', indicator: Fractal.indicator.count({ subjectPrefix: 'custom/factory/flake/', event: { kind: 'custom.factory.flake', at: 'seen_at' } }), targets: [{ stat: 'per-hour', max: 0.5 }] }
    expect(render(flakes)).toContain('  indicator "count" {\n    subject-prefix "custom/factory/flake/"\n    event "custom.factory.flake" at="seen_at"\n  }\n  window "24h"\n  target "per-hour" max=0.5\n')
  })

  it('lowers one claim as the whole sample and an only per field', () => {
    expect(render(runs)).toBe(`version 2
objective "acme/merge-latency" {
  about "A merge group's run, from created to completed"
  indicator "interval" {
    subject-prefix "github-workflow-run/acme/repo/"
    start "github.workflow-run" at="created_at"
    stop "github.workflow-run" at="completed_at"
    only "event" "merge_group"
    only "attempt" "1"
  }
  window "1d"
  target "p95" within="10m"
}
`)
    const flakes = {
      name: 'acme/flakes',
      about: 'Runs with a flake',
      indicator: Fractal.indicator.count({
        subjectPrefix: 'github-workflow-run/acme/repo/',
        event: { kind: 'github.workflow-run', at: 'completed_at' },
        exclude: [{ field: 'flakes', values: ['0'] }],
        only: [{ field: 'workflow', values: ['Workspace CI'] }],
      }),
      window: { span: '1h' },
      targets: [{ stat: 'per-hour', max: 0 }],
    } satisfies ObjectiveInput
    expect(render(flakes)).toContain(
      '  indicator "count" {\n    subject-prefix "github-workflow-run/acme/repo/"\n    event "github.workflow-run" at="completed_at"\n    exclude "flakes" "0"\n    only "workflow" "Workspace CI"\n  }\n',
    )
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
  ]
  it.each(native)('lowers indicator $indicator.kind', ({ indicator, targets, kdl }) => {
    expect(render({ name: 'shipped', about: 'Measured', indicator, ...(targets === undefined ? {} : { targets }) })).toBe(
      `version 2\nobjective "shipped" {\n  about "Measured"\n${kdl}}\n`,
    )
  })

  it.each([...nativeRows.rows, ...nativeRows.valid])('binds native row %s without changing its identity', (row) => {
    expect(
      render({
        name: 'acme/native-owner',
        about: 'Fleet ownership of a built-in row',
        indicator: Fractal.indicator.native({ objective: row }),
        onBreach: Fractal.response.fault({ owner: 'agent/acme/ops', condition: 'native-budget' }),
      }),
    ).toContain(`  indicator "native" {\n    objective "${row}"\n  }\n`)
  })

  it.each(nativeRows.invalid)('rejects malformed native row %j', (row) => {
    expect(() =>
      render({
        name: 'acme/native-owner',
        about: 'Fleet ownership of a built-in row',
        indicator: Fractal.indicator.native({ objective: row }),
      }),
    ).toThrow('native objective must name a well-formed built-in row')
  })

  it('does not widen the declaration name grammar for native row references', () => {
    expect(() =>
      render({
        name: 'acme/writer_hold',
        about: 'Fleet ownership of a built-in row',
        indicator: Fractal.indicator.native({ objective: 'transaction/writer_hold' }),
      }),
    ).toThrow()
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
    { name: 'an unknown indicator', input: { ...base, indicator: { kind: 'histogram' } }, message: /unknown indicator `histogram`/ },
    { name: 'an empty latency', input: { name: 'x', about: 'a', indicator: { kind: 'latency', paths: [] }, targets: [{ stat: 'p99', within: '1s' }] }, message: /needs a `path`/ },
    { name: 'an extra indicator field', input: { name: 'x', about: 'a', indicator: { kind: 'statement', path: 'p' }, targets: [] }, message: /indicator `statement` has an unknown field `path`/ },
    { name: 'a fractional idle minimum', input: { name: 'cpu', about: 'a', indicator: { kind: 'cpu', idleMinMinutes: 1.5, loadMaxCores: 1 }, targets: [] }, message: /idleMinMinutes is not a positive integer/ },
    { name: 'an idle minimum over an hour', input: { name: 'cpu', about: 'a', indicator: Fractal.indicator.cpu({ idleMinMinutes: 61, loadMaxCores: 1 }), targets: [{ stat: 'cores', max: 1 }, { stat: 'idle-cores', max: 0.5 }] }, message: /idle minimum is not between 1 and 60/ },
    { name: 'idle cores above cores', input: { name: 'cpu', about: 'a', indicator: Fractal.indicator.cpu({ idleMinMinutes: 10, loadMaxCores: 1 }), targets: [{ stat: 'cores', max: 1 }, { stat: 'idle-cores', max: 2 }] }, message: /idle CPU target is above/ },
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
    { name: 'an interval without a window', input: { ...base, window: undefined }, message: /a `interval` indicator needs a `window`/ },
    { name: 'a window on native telemetry', input: { name: 'x', about: 'a', indicator: latency, window: { span: '1d' }, targets: [{ stat: 'p99', within: '1s' }] }, message: /only an interval or a count takes a `window`/ },
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
    { name: 'a response on native telemetry', input: { name: 'x', about: 'a', indicator: latency, targets: [{ stat: 'p99', within: '1s' }], onBreach: rollout.onBreach }, message: /only claim indicators take `onBreach`/ },
    { name: 'a count with min and max', input: { ...throughput, targets: [{ stat: 'per-hour', min: 1, max: 2 }] }, message: /needs one bound/ },
    { name: 'a count with a zero minimum', input: { ...throughput, targets: [{ stat: 'per-hour', min: 0 }] }, message: /no target `per-hour` with that bound/ },
    { name: 'a count with a negative maximum', input: { ...throughput, targets: [{ stat: 'per-hour', max: -1 }] }, message: /no target `per-hour` with that bound/ },
    { name: 'a count with a percentile', input: { ...throughput, targets: [{ stat: 'p95', within: '5m' }] }, message: /no target `p95` with that bound/ },
    { name: 'a count without a window', input: { ...throughput, window: undefined }, message: /a `count` indicator needs a `window`/ },
    { name: 'a count window with a sample minimum', input: { ...throughput, window: { span: '24h', minSamples: 5 } }, message: /takes no `minSamples`/ },
    { name: 'a count without an event', input: { ...throughput, indicator: { ...throughput.indicator, event: undefined } }, message: /a count needs `event`/ },
    { name: 'a count event without a time field', input: { ...throughput, indicator: { ...throughput.indicator, event: { kind: 'custom.factory.pr-merged' } } }, message: /`event` needs `at`/ },
    { name: 'a count with an interval child', input: { ...throughput, indicator: { ...throughput.indicator, start: { kind: 'k' } } }, message: /indicator `count` has an unknown field `start`/ },
    { name: 'a count prefix without a slash', input: { ...throughput, indicator: { ...throughput.indicator, subjectPrefix: 'custom' } }, message: /must name subjects/ },
    { name: 'a min on native telemetry', input: { name: 'x', about: 'a', indicator: Fractal.indicator.divergence(), targets: [{ stat: 'open', min: 1 }] }, message: /no target `open` with that bound/ },
    { name: 'a repeated only field', input: { ...runs, indicator: { ...runs.indicator, only: [{ field: 'event', values: ['merge_group'] }, { field: 'event', values: ['pull_request'] }] } }, message: /`only "event"` is repeated/ },
    { name: 'an only without values', input: { ...runs, indicator: { ...runs.indicator, only: [{ field: 'event', values: [] }] } }, message: /`only` needs a field and at least one value/ },
    { name: 'an only with an empty value', input: { ...runs, indicator: { ...runs.indicator, only: [{ field: 'event', values: [''] }] } }, message: /`only` needs a field and at least one value/ },
    { name: 'an only on a count without values', input: { ...throughput, indicator: { ...throughput.indicator, only: [{ field: 'author', values: [] }] } }, message: /`only` needs a field and at least one value/ },
    { name: 'a same-kind stop at the same field', input: { ...runs, indicator: { ...runs.indicator, stop: { kind: 'github.workflow-run', at: 'created_at' } } }, message: /needs both to name different `at` fields/ },
    { name: 'a same-kind stop without a time field', input: { ...runs, indicator: { ...runs.indicator, stop: { kind: 'github.workflow-run' } } }, message: /needs both to name different `at` fields/ },
    { name: 'a same-kind stop with a member', input: { ...runs, indicator: { ...runs.indicator, stop: { kind: 'github.workflow-run', at: 'completed_at', member: 'host' } } }, message: /takes no `expect` or `member`/ },
    { name: 'a same-kind stop with expect', input: { ...base, indicator: { ...interval, start: { kind: 'custom.acme.change-applied', at: 'ready_at' } } }, message: /takes no `expect` or `member`/ },
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

  it('lets one claim start and stop the sample, and keeps each only', () => {
    expect(Fractal.objective(runs).indicator).toEqual({
      kind: 'interval',
      subjectPrefix: 'github-workflow-run/acme/repo/',
      start: { kind: 'github.workflow-run', at: 'created_at' },
      stop: { kind: 'github.workflow-run', at: 'completed_at' },
      only: [
        { field: 'event', values: ['merge_group'] },
        { field: 'attempt', values: ['1'] },
      ],
    })
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

  it('are the 18 native objectives, one segment each, so none is declarable', () => {
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
  it.effect('loads numeric budgets and native owner bindings from module and KDL declarations', () =>
    Effect.gen(function* () {
      const policy = `{ allowance: 0.05, horizon: '30d', minCoverage: 0.9, minSamples: 1, fast: { burnRate: 14.4, long: '1h', short: '5m' }, slow: { burnRate: 6, long: '6h', short: '30m' } }`
      const root = fixture({
        'acme/ratio/objective.ts': `${header}export default Fractal.objective({ about: 'Requests', indicator: Fractal.indicator.ratio({ subjectPrefix: 'custom/acme/request/', event: { kind: 'custom.acme.request', at: 'at' }, field: 'ms', badAbove: 100 }), budget: ${policy} })`,
        'acme/gauge/objective.ts': `${header}export default Fractal.objective({ about: 'Free capacity', indicator: Fractal.indicator.gauge({ subjectPrefix: 'custom/acme/capacity/', event: { kind: 'custom.acme.capacity', at: 'at' }, field: 'free', badBelow: 100, maxAge: '2h', cadence: '1h' }), budget: ${policy} })`,
        'acme/native/objective.ts': `${header}export default Fractal.objective({ about: 'Native ownership', indicator: Fractal.indicator.native({ objective: 'person-read' }), onBreach: Fractal.response.fault({ owner: 'agent/acme/ops', condition: 'read-budget' }) })`,
        'acme/native-kdl/objective.kdl': 'version 2\nobjective "acme/native-kdl" {\n about "Native ownership"\n indicator "native" { objective "transaction/read"; }\n}\n',
      })
      const subjects = (yield* load(root)).filter(({ kind }) => kind === 'objective')
      // Canonical declaration paths sort lexically: "native-kdl/" precedes "native/".
      expect(subjects.map(({ id }) => id)).toEqual(['acme/gauge', 'acme/native-kdl', 'acme/native', 'acme/ratio'])
    }),
  )

  it.effect('loads GitHub measurements from the same module and KDL declaration tree', () =>
    Effect.gen(function* () {
      const root = fixture({
        'delivery/merged/objective.ts': `${header}export default Fractal.objective({ about: 'All-author merges', indicator: Fractal.indicator.githubMergedCount({ source: 'github-source/example', roster: 'github-scope/example', timezone: 'UTC', cut: 'now', measurement: 'targeted' }), targets: [{ stat: 'per-hour', min: 7 }] })`,
        'delivery/ready/objective.kdl': 'version 2\nobjective "delivery/ready" {\n about "Measured final-ready latency"\n indicator "github-ready-to-merged" {\n source "github-source/example"\n roster "github-scope/example"\n timezone "UTC"\n cut "watermark"\n measurement "only"\n }\n}\n',
      })
      const subjects = (yield* load(root)).filter(({ kind }) => kind === 'objective')
      expect(subjects.map(({ id }) => id)).toEqual(['delivery/merged', 'delivery/ready'])
    }),
  )

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

  it.effect('declare a count objective from objective.ts and objective.kdl', () =>
    Effect.gen(function* () {
      const root = fixture({
        'factory/work-throughput/objective.ts': `${header}export default Fractal.objective({ about: 'Merged', indicator: Fractal.indicator.count({ subjectPrefix: 'custom/factory/merged-pr/', event: { kind: 'custom.factory.pr-merged', at: 'merged_at' } }), window: { span: '24h' }, targets: [{ stat: 'per-hour', min: 10 }] })`,
        'factory/flakes/objective.kdl': 'version 2\nobjective "factory/flakes" {\n  about "Flakes"\n  indicator "count" {\n    subject-prefix "custom/factory/flake/"\n    event "custom.factory.flake" at="seen_at"\n  }\n  window "24h"\n  target "per-hour" max=1\n}\n',
      })
      const subjects = (yield* load(root)).filter(({ kind }) => kind === 'objective')
      expect(subjects.map(({ id, declaration }) => ({ id, tag: declaration._tag }))).toEqual([
        { id: 'factory/flakes', tag: 'Kdl' },
        { id: 'factory/work-throughput', tag: 'Module' },
      ])
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
      message: 'A declared objective is an interval, count, ratio, gauge, GitHub measurement or native binding named NAMESPACE/NAME; objective/ops/x is not',
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

describe('native source-backed GitHub objectives', () => {
  const native = {
    source: 'github-source/test',
    roster: 'github-scope/test',
    timezone: 'America/New_York',
    cut: 'watermark',
    measurement: 'only',
  } as const

  it('lowers the exact measured-only fixture accepted by the native reader', () => {
    expect(render({
      name: 'delivery/ready-to-merged',
      about: 'Final ready to merge, measured without a target',
      indicator: Fractal.indicator.githubReadyToMerged(native),
      targets: [],
    })).toBe(`version 2
objective "delivery/ready-to-merged" {
  about "Final ready to merge, measured without a target"
  indicator "github-ready-to-merged" {
    source "github-source/test"
    roster "github-scope/test"
    timezone "America/New_York"
    cut "watermark"
    measurement "only"
  }
}
`)
  })

  it('accepts omitted targets for both measured-only GitHub indicators like the native reader', () => {
    for (const indicator of [
      Fractal.indicator.githubReadyToMerged(native),
      Fractal.indicator.githubMergedCount(native),
    ]) {
      const input = { name: 'delivery/measured', about: 'Measured GitHub evidence', indicator }
      const text = render(input)
      expect(text).toBe(render({ ...input, targets: [] }))
      expect(text).toContain('measurement "only"')
      expect(text).not.toContain('target "')
    }
    expect(() => render({
      name: 'delivery/merged', about: 'Targeted GitHub throughput',
      indicator: Fractal.indicator.githubMergedCount({ ...native, measurement: 'targeted' }),
    })).toThrow('has no `target`')
  })

  it('keeps merged-count target values policy-owned and omits author filters', () => {
    const text = render({
      name: 'delivery/merged',
      about: 'All-author merged PRs',
      indicator: Fractal.indicator.githubMergedCount({ ...native, cut: 'now', measurement: 'targeted' }),
      targets: [{ stat: 'per-hour', min: 7 }],
    })
    expect(text).toContain('indicator "github-merged-count"')
    expect(text).toContain('target "per-hour" min=7')
    expect(text).not.toContain('only "author"')
  })

  it('rejects remote scope for GitHub targets just like the native reader', () => {
    expect(() => render({
      name: 'delivery/merged',
      about: 'All-author merged PRs',
      indicator: Fractal.indicator.githubMergedCount({ ...native, measurement: 'targeted' }),
      targets: [{ stat: 'per-hour', min: 7, scope: 'remote' }],
    })).toThrow('only a latency target has a remote scope')
  })

  it('rejects claim budgets on measured-only and targeted GitHub declarations', () => {
    const budget = {
      allowance: 0.05, horizon: '30d', minCoverage: 0.9, minSamples: 1,
      fast: { burnRate: 14.4, long: '1h', short: '5m' },
      slow: { burnRate: 6, long: '6h', short: '30m' },
    } as const
    for (const measurement of ['only', 'targeted'] as const) {
      expect(() => render({
        name: 'delivery/merged', about: 'Merge count',
        indicator: Fractal.indicator.githubMergedCount({ ...native, measurement }),
        targets: measurement === 'targeted' ? [{ stat: 'per-hour', min: 7 }] : [], budget,
      })).toThrow('GitHub measurements do not take claim budgets')
    }
    expect(() => render({
      name: 'delivery/ready', about: 'Final-ready latency',
      indicator: Fractal.indicator.githubReadyToMerged(native), targets: [], budget,
    })).toThrow('GitHub measurements do not take claim budgets')
  })

  it('rejects a hidden default target, a latency target and an implicit cut', () => {
    const base = { name: 'delivery/merged', about: 'Merged', targets: [] } as const
    expect(() => render({ ...base, indicator: Fractal.indicator.githubMergedCount({ ...native, measurement: 'targeted' }) })).toThrow('needs one per-hour minimum')
    expect(() => render({
      ...base,
      indicator: Fractal.indicator.githubReadyToMerged(native),
      targets: [{ stat: 'p95', within: '5m' }],
    })).toThrow('neither targets')
    expect(() => Fractal.objective({
      ...base,
      indicator: { ...Fractal.indicator.githubMergedCount(native), cut: undefined },
    } as unknown as ObjectiveInput)).toThrow('cut must be')
  })
})

describe('native numeric budgets', () => {
  const budget = {
    allowance: 0.05,
    horizon: '30d',
    minCoverage: 0.9,
    minSamples: 20,
    fast: { burnRate: 14.4, long: '1h', short: '5m' },
    slow: { burnRate: 6, long: '6h', short: '30m' },
  } as const
  const input = {
    name: 'acme/request-budget',
    about: 'Request bad-event allowance',
    indicator: Fractal.indicator.ratio({
      subjectPrefix: 'custom/acme/request/',
      event: { kind: 'custom.acme.request', at: 'observed_at' },
      field: 'duration_ms',
      badAbove: 100,
      labels: ['route'],
      only: [{ field: 'class', values: ['interactive'] }],
    }),
    targets: [],
    budget,
  } as const

  it('lowers ratio fields, filtering and both burn pairs to kernel KDL', () => {
    expect(render(input)).toBe(`version 2
objective "acme/request-budget" {
  about "Request bad-event allowance"
  indicator "ratio" {
    subject-prefix "custom/acme/request/"
    event "custom.acme.request" at="observed_at"
    value "duration_ms" bad-above=100
    label "route"
    only "class" "interactive"
  }
  budget allowance=0.05 horizon="30d" min-coverage=0.9 min-samples=20 {
    fast burn-rate=14.4 long="1h" short="5m"
    slow burn-rate=6 long="6h" short="30m"
  }
}
`)
    expect(Object.isFrozen(Fractal.objective(input).budget?.fast)).toBe(true)
  })

  it('lowers gauge lower limits and honest coverage/cadence metadata', () => {
    const indicator = Fractal.indicator.gauge({
      subjectPrefix: 'custom/acme/capacity/',
      event: { kind: 'custom.acme.capacity', at: 'observed_at' },
      field: 'free_bytes',
      badBelow: 1_000,
      maxAge: '2h',
      cadence: '1h',
    })
    const text = render({ ...input, indicator })
    expect(text).toContain('indicator "gauge"')
    expect(text).toContain('value "free_bytes" bad-below=1000')
    expect(text).toContain('max-age "2h"')
    expect(text).toContain('cadence "1h"')
    expect(text).toContain('short="5m"')
  })

  it.each([
    { ...input, budget: undefined },
    { ...input, window: { span: '1h' } },
    { ...input, targets: [{ stat: 'p99', within: '1s' }] },
    { ...input, indicator: { ...input.indicator, badAbove: Number.NaN } },
    { ...input, indicator: { ...input.indicator, event: { kind: 'custom.acme.request' } } },
    { ...input, budget: { ...budget, allowance: 1 } },
    { ...input, budget: { ...budget, minCoverage: 0 } },
    { ...input, budget: { ...budget, minSamples: 0 } },
    { ...input, budget: { ...budget, fast: { ...budget.fast, short: '1h' } } },
    { ...input, budget: { ...budget, slow: { ...budget.slow, long: '31d' } } },
    { ...input, budget: { ...budget, fast: { ...budget.fast, burnRate: 2 } } },
    { ...input, indicator: { ...input.indicator, kind: 'gauge', maxAge: '1m', cadence: '1h' } },
    { ...input, indicator: { ...input.indicator, kind: 'gauge', maxAge: '2h', cadence: '1h', badBelow: 1 } },
  ])('rejects invalid numeric semantics %#', (value) => {
    expect(() => Fractal.objective(value as ObjectiveInput)).toThrow()
  })
})

describe('native objective ownership bindings', () => {
  const input = {
    name: 'acme/person-read',
    about: 'Fleet ownership of native latency',
    indicator: Fractal.indicator.native({ objective: 'person-read/remote' }),
    onBreach: Fractal.response.fault({ owner: 'agent/acme/ops', condition: 'person-read-budget' }),
  } as const

  it('lowers a native row reference without redefining targets or budget', () => {
    expect(render(input)).toBe(`version 2
objective "acme/person-read" {
  about "Fleet ownership of native latency"
  indicator "native" {
    objective "person-read/remote"
  }
  on-breach "fault" condition="person-read-budget" owner="agent/acme/ops"
}
`)
  })

  it('rejects target, window and budget overrides and malformed row references', () => {
    for (const value of [
      { ...input, indicator: { kind: 'native', objective: '../bad' } },
      { ...input, targets: [{ stat: 'p99', within: '1s' }] },
      { ...input, window: { span: '1h' } },
      { ...input, budget: { allowance: 0.05 } },
    ]) {
      expect(() => Fractal.objective(value as ObjectiveInput)).toThrow()
    }
  })
})
