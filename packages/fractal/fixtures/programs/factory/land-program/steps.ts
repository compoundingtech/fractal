/**
 * The landing step catalogue as program steps. Agentless steps run `steps/factory.py <verb>` in the
 * run's workspace, which writes the step's JSON result to `rundata/<run>/<result>.json`. A review
 * is a worker turn of the factory's `factory/step/review` mission; the owner's acknowledgement is a
 * native ask.
 */
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { Data, Effect, Schema } from 'effect'

import { Fractal } from '../../../../src/effect.ts'
import type { PersonReference } from '../../../../src/index.ts'

export class StepFailed extends Data.TaggedError('StepFailed')<{
  readonly step: string
  readonly reason: string
}> {}

export type Mode = 'declaration' | 'full'
export const Class = Schema.Literals(['routine', 'authority', 'code', 'removal', 'cross-owner'])
export type Class = typeof Class.Type

/** One `factory.py <verb>` effect; its JSON result file, decoded with `schema`, is the value. */
const tool = <S extends Schema.Codec<any, any>>(
  verb: string,
  env: Readonly<Record<string, string>>,
  result: string,
  schema: S,
) =>
  Effect.gen(function* () {
    const run = (yield* Fractal.env).ST_MISSION_RUN ?? 'adhoc'
    const cwd = yield* Fractal.cwd
    const out = yield* Fractal.command(['python3', 'steps/factory.py', verb], {
      env: { ...env, FACTORY_SCRATCH_ROOT: cwd },
    })
    if (out.code !== 0)
      return yield* new StepFailed({
        step: verb,
        reason: out.stderr.trim().split('\n').slice(-3).join(' '),
      })
    const file = join(cwd, 'rundata', run.replaceAll('/', '_'), `${result}.json`)
    const json = yield* Effect.promise(() => readFile(file, 'utf8'))
    return yield* Schema.decodeUnknownEffect(Schema.fromJsonString(schema))(json).pipe(
      Effect.mapError((error) => new StepFailed({ step: verb, reason: error.message })),
    )
  })

export const Classification = Schema.Struct({
  change: Schema.String,
  author: Schema.String,
  classes: Schema.Array(Class),
  reasons: Schema.Array(Schema.String),
})
/** classify: every class this change belongs to; the program routes, so every class is accepted. */
export const classify = ({ change }: { readonly change: string }) =>
  Fractal.step(
    { id: 'classify', success: Classification },
    tool('classify', { CHANGE: change, ACCEPT: Class.literals.join(',') }, 'class', Classification),
  )

export const CheckResult = Schema.Struct({
  mode: Schema.String,
  errors: Schema.Array(Schema.String),
})
/** check: declaration mode (render, offline validate, guard, `Removes:`) or full (plus the suite). */
export const check = ({ change, mode }: { readonly change: string; readonly mode: Mode }) =>
  Fractal.step(
    { id: 'check', success: CheckResult },
    tool('check', { CHANGE: change, MODE: mode }, 'check', CheckResult),
  )

export const Merged = Schema.Struct({
  commit: Schema.String,
  parent: Schema.String,
  attempts: Schema.Number,
})
/** merge: squash onto main by compare-and-swap; re-checks the exact merge result when main moved. */
export const merge = ({ change, mode }: { readonly change: string; readonly mode: Mode }) =>
  Fractal.step(
    { id: 'merge', success: Merged },
    tool('merge', { CHANGE: change, MODE: mode }, 'merged', Merged),
  )

export const Published = Schema.Struct({
  host: Schema.String,
  head: Schema.String,
  changed: Schema.Array(Schema.String),
})
/** publish: self-admission on one host (render main, dry-run every changed subject), then apply. */
export const publish = ({ host }: { readonly host: string }) =>
  Fractal.step(
    { id: `publish-${host}`, success: Published },
    tool('publish', { HOST: host }, 'publish', Published),
  )

export const Verified = Schema.Struct({ head: Schema.String, stale: Schema.Array(Schema.String) })
/** verify: every subject this run applied is live at its rendered revision. */
export const verify = ({ host }: { readonly host: string }) =>
  Fractal.step(
    { id: `verify-${host}`, success: Verified },
    tool('verify', { HOST: host }, 'verify', Verified),
  )

export const Reverted = Schema.Struct({
  why: Schema.String,
  reverted: Schema.Array(Schema.Struct({ reverted: Schema.String, revert_commit: Schema.String })),
})
/** revert: undo exactly the offending commits on main; the program says which step failed and why. */
export const revert = ({ failed, reason }: { readonly failed: string; readonly reason: string }) =>
  Fractal.step(
    { id: 'revert', success: Reverted },
    tool('revert', { FAILED: failed, FAILED_REASON: reason }, 'revert', Reverted),
  )

export type Checklist = 'authority' | 'publisher-code' | 'removal' | 'cross-owner'
export const Verdict = Schema.Struct({
  verdict: Schema.Literals(['GO', 'NO-GO']),
  head: Schema.String,
  findings: Schema.Array(Schema.String),
})
export type Verdict = typeof Verdict.Type

/**
 * review: a worker turn by a fresh run-scoped reviewer that `factory/step/review` declares, never
 * the author. The checklist picks the prompt; the verdict is the child run's product.
 */
export const review = (options: {
  readonly id: string
  readonly change: string
  readonly checklist: Checklist
  readonly mode: 'pre-merge' | 'post-hoc'
  readonly commit?: string
  readonly reviewer: 'llm' | 'scripted'
}) =>
  Fractal.step({
    id: options.id,
    goal: `Review the exact change against the ${options.checklist} checklist.`,
    success: Verdict,
  }).pipe(
    Effect.provide(
      Fractal.Worker.layer({
        mission: 'factory/step/review',
        product: 'review/verdict',
        input: {
          change: options.change,
          checklist: options.checklist,
          mode: options.mode,
          commit: options.commit ?? options.change,
          reviewer: options.reviewer,
        },
      }),
    ),
  )

/** owner-ack: the owning person accepts a cross-owner write; the deadline default declines. */
export const ownerAck = (options: {
  readonly owner: PersonReference
  readonly change: string
  readonly deadline: string
}) =>
  Fractal.ask({
    id: 'owner-ack',
    for: options.owner,
    title: `Accept change ${options.change.slice(0, 10)} into your subjects`,
    request: {
      version: 1,
      type: 'decision',
      question: `Accept the write into your subjects by change ${options.change.slice(0, 10)}?`,
      why_person: 'The owner decides writes into their own namespace.',
      answers: [
        { id: 'approve', label: 'Accept', consequence: 'The change lands.', outcome: 'accept' },
        {
          id: 'reject',
          label: 'Decline',
          consequence: 'The landing fails as refused.',
          outcome: 'decline',
        },
      ],
    },
    deadline: options.deadline,
    default: 'reject',
  }).pipe(Effect.map((answer) => (answer.answer?.id === 'approve' ? 'Approved' : 'Rejected')))
