/**
 * The program form of a mission, `Fractal.mission(intent, program)`, and what its body calls:
 * `Fractal.step` (an `Activity`), worker turns, `Fractal.ask` and `Fractal.run` (`DurableDeferred`s
 * the daemon resolves), `Fractal.command`, `Fractal.env` and `Fractal.cwd`.
 * See docs/st3/program-engine.md §11.
 */
import { Context, Effect, Layer, Schedule, Schema, SchemaAST } from 'effect'
import { Activity, DurableDeferred, type Workflow } from 'effect/workflow'

import { mission as dataMission, type MissionIntent } from './define.ts'
import type { Authority, PersonReference } from './mod.ts'
import {
  keyed,
  Program,
  programEntry,
  ProgramEntry,
  TaggedFailure,
  type CommandOptions,
  type CommandResult,
  type HostContextV1,
  type ProgramExit,
  type ProgramServices,
  type Tagged,
} from './program-engine.ts'
import type { SubjectDefinition } from './subject-ref.ts'
import { StepExecution } from './step-execution.ts'

/** A schema without decoding or encoding services. */
type Codec = Schema.Codec<any, any>

/** A mission input Struct: every field is required and encodes to a string. */
export type InputSchema = Workflow.AnyStructSchema &
  Schema.Codec<object, { readonly [name: string]: string }>

/** A linked child run that completed: its subject and the products its steps declare. */
export const ChildRun = Schema.Struct({
  run: Schema.String,
  status: Schema.Literal('completed'),
  products: Schema.optionalKey(Schema.Record(Schema.String, Schema.Unknown)),
  executions: Schema.optionalKey(Schema.Array(StepExecution)),
})
export type ChildRun = typeof ChildRun.Type

/** A linked child run ended without completing. */
export class ChildFailed extends Schema.TaggedError<ChildFailed>()('ChildFailed', {
  run: Schema.String,
  status: Schema.String,
  reason: Schema.String,
  executions: Schema.optionalKey(Schema.Array(StepExecution)),
}) {}

/** A person's answer to an ask; `resolved_by` is the person or `deadline`. */
export const AskAnswer = Schema.Struct({
  answer: Schema.optionalKey(
    Schema.Struct({
      type: Schema.String,
      outcome: Schema.optionalKey(Schema.String),
      id: Schema.optionalKey(Schema.String),
      label: Schema.optionalKey(Schema.String),
      text: Schema.optionalKey(Schema.String),
    }),
  ),
  summary: Schema.optionalKey(Schema.String),
  resolved_by: Schema.String,
})
export type AskAnswer = typeof AskAnswer.Type

/** The ask was withdrawn while the run went on. */
export class AskCancelled extends Schema.TaggedError<AskCancelled>()('AskCancelled', {
  ask: Schema.String,
}) {}

/** The intent of a program mission; `input` fields become its native text inputs. */
export interface ProgramIntent<
  I extends InputSchema,
  O extends Codec,
  ES extends Codec | undefined,
> {
  readonly id: string
  readonly goal: string
  readonly reportTo: MissionIntent['reportTo']
  /** The mission's and its program step's timeout. */
  readonly timeout?: string
  /** How long the program may go without a journal entry before its run reports it stalled. */
  readonly stalledAfter?: string
  /** At most this many runs at once (`concurrent-runs max=N`). */
  readonly concurrentRuns?: number
  /** What the mission's runs may do as `mission-run/RUN` (docs/st3/program-engine.md §3). */
  readonly authority?: Authority
  readonly input: I
  readonly output?: O
  readonly error?: ES
}

/** A mission whose body is a program; its default export is the bundle's entry. */
export interface ProgramMission<
  I extends InputSchema,
  O extends Codec,
  E,
> extends SubjectDefinition<'mission', ProgramIntent<I, O, Codec | undefined>, never> {
  readonly intent: ProgramIntent<I, O, Codec | undefined>
  readonly program: (input: I['Type']) => Effect.Effect<O['Type'], E, ProgramServices>
  readonly [ProgramEntry]: (host: HostContextV1) => Promise<ProgramExit>
}

type Meta = { readonly meta: { readonly url: string } }

const encodesToString = (ast: SchemaAST.AST): boolean =>
  SchemaAST.isString(ast) ||
  SchemaAST.isTemplateLiteral(ast) ||
  (SchemaAST.isLiteral(ast) && typeof ast.literal === 'string') ||
  (SchemaAST.isEnum(ast) && ast.enums.every(([, value]) => typeof value === 'string')) ||
  (SchemaAST.isUnion(ast) && ast.types.every(encodesToString))

/**
 * The native inputs of a program mission: each field of `input` is one `input "name" kind="text"`,
 * and a run supplies every one, so a field must be required and encode to a string.
 */
export const programInputNames = (input: Workflow.AnyStructSchema): readonly string[] =>
  Object.entries(input.fields).map(([name, field]) => {
    if (
      SchemaAST.isOptional(field.ast) ||
      encodesToString(SchemaAST.toEncoded(field.ast)) === false
    )
      throw new TypeError(`Program input ${name} must be required and encode to a string`)
    return name
  })

/**
 * Declares a mission. With flat fields it is a data mission (see `Fractal.mission` in `fractal`).
 * With `(intent, program)` it is a program: `program` receives the decoded `intent.input`, its
 * success is encoded with `intent.output` and its typed failure with `intent.error`, or as `_tag`
 * plus fields when the intent declares none.
 */
export function mission(
  intent: Meta & MissionIntent,
): SubjectDefinition<'mission', MissionIntent, never>
export function mission<
  I extends InputSchema,
  O extends Codec = typeof Schema.Void,
  E extends Tagged = never,
>(
  intent: Meta & ProgramIntent<I, O, undefined>,
  program: (input: I['Type']) => Effect.Effect<O['Type'], E, ProgramServices>,
): ProgramMission<I, O, E>
export function mission<
  I extends InputSchema,
  ES extends Codec,
  O extends Codec = typeof Schema.Void,
>(
  intent: Meta & ProgramIntent<I, O, ES> & { readonly error: ES },
  program: (input: I['Type']) => Effect.Effect<O['Type'], ES['Type'], ProgramServices>,
): ProgramMission<I, O, ES['Type']>
export function mission(
  intent: Meta & (MissionIntent | ProgramIntent<InputSchema, Codec, Codec | undefined>),
  program?: (input: object) => Effect.Effect<unknown, unknown, ProgramServices>,
): SubjectDefinition<'mission', unknown, never> {
  if (program === undefined) return dataMission(intent as Meta & MissionIntent)
  const { meta, ...rest } = intent as Meta & ProgramIntent<InputSchema, Codec, Codec | undefined>
  programInputNames(rest.input)
  const definition: ProgramMission<InputSchema, Codec, unknown> = {
    _tag: 'SubjectRef',
    kind: 'mission',
    url: meta.url,
    intent: rest,
    program,
    [ProgramEntry]: programEntry({
      id: rest.id,
      input: rest.input,
      output: rest.output ?? Schema.Void,
      error: rest.error ?? TaggedFailure,
      body: program,
    }),
  }
  return definition
}

/**
 * The child mission a worker turn runs, and its product's path below the child run
 * (`review/verdict` names `resource/mission-run/CHILD/review/verdict`), whose value is the step's.
 */
export class Worker extends Context.Service<
  Worker,
  {
    readonly mission: string
    readonly input?: Readonly<Record<string, string>>
    readonly product?: string
  }
>()('fractal/Worker') {
  /** Runs the worker turns it is provided to as child runs of `mission`. */
  static readonly layer = (options: Worker['Service']) => Layer.succeed(Worker, options)
}

/** Waits for the linked child run `key` names; the daemon journals its end under `key`. */
const child = (id: string, mission: string, input: Readonly<Record<string, string>>) =>
  keyed(id, (key, program) => {
    program.requests.set(key, () => program.host.child({ key, mission, input }))
    return DurableDeferred.await(
      DurableDeferred.make(key, { success: ChildRun, error: ChildFailed }),
    ).pipe(Effect.ensuring(Effect.sync(() => program.requests.delete(key))))
  })

const noInterruptRetry = Schedule.recurs(0)

export interface StepOptions {
  readonly id: string
  readonly goal?: string
}

/**
 * The failure a step without `error` delivers, live and on replay: its tag and data fields as a
 * plain object, without the class's prototype, `Error` members or methods.
 */
export type Journaled<E> = E extends unknown
  ? {
      readonly [K in keyof E as K extends keyof Error | symbol
        ? never
        : E[K] extends (...args: never) => unknown
          ? never
          : K]: E[K]
    }
  : never

/**
 * A step of a program, journaled under its key (`id`, then `id#2`… for repeats, so `Effect.retry`
 * runs it again under a new key). With an effect it is an `Activity`: replay returns the journaled
 * value decoded with `success` (void without one) or fails again with the journaled typed error,
 * rebuilt as its class with `error`, or as its plain `Journaled` shape without. Without an effect
 * it is a worker turn: a child run of the provided `Worker` mission, whose product decoded with
 * `success` is the value.
 */
export function step<A, E extends Tagged = never, R = never>(
  options: StepOptions & { readonly success?: undefined; readonly error?: undefined },
  effect: Effect.Effect<A, E, R>,
): Effect.Effect<void, Journaled<E>, R | ProgramServices>
export function step<S extends Codec, E extends Tagged = never, R = never>(
  options: StepOptions & { readonly success: S; readonly error?: undefined },
  effect: Effect.Effect<S['Type'], E, R>,
): Effect.Effect<S['Type'], Journaled<E>, R | ProgramServices>
export function step<ES extends Codec, S extends Codec = typeof Schema.Void, R = never>(
  options: StepOptions & { readonly success?: S; readonly error: ES },
  effect: Effect.Effect<S['Type'], ES['Type'], R>,
): Effect.Effect<S['Type'], ES['Type'], R | ProgramServices>
export function step<S extends Codec = typeof Schema.Void>(
  options: StepOptions & { readonly success?: S; readonly execution: true },
): Effect.Effect<
  { readonly value: S['Type']; readonly executions?: readonly StepExecution[] },
  ChildFailed,
  Worker | ProgramServices
>
export function step<S extends Codec = typeof Schema.Void>(
  options: StepOptions & { readonly success?: S; readonly execution?: false },
): Effect.Effect<S['Type'], ChildFailed, Worker | ProgramServices>
export function step(
  options: StepOptions & {
    readonly success?: Codec | undefined
    readonly error?: Codec | undefined
    readonly execution?: boolean | undefined
  },
  effect?: Effect.Effect<unknown, unknown, unknown>,
): Effect.Effect<unknown, unknown, unknown> {
  const { success } = options
  if (effect !== undefined)
    return keyed(options.id, (key) =>
      Activity.make({
        name: key,
        success: success ?? Schema.Void,
        error: options.error ?? TaggedFailure,
        execute: success === undefined ? Effect.asVoid(effect) : effect,
        interruptRetryPolicy: noInterruptRetry,
      }),
    )
  return Effect.flatMap(Effect.service(Worker), (worker) =>
    Effect.flatMap(child(options.id, worker.mission, worker.input ?? {}), (done) => {
      const decode = () => {
        if (success === undefined) return Effect.void
        // The daemon keys products by full subject: `resource/<child run>/<path>`.
        const subject = `resource/${done.run}/${worker.product}`
        const value = worker.product === undefined ? undefined : done.products?.[subject]
        if (value === undefined)
          return Effect.die(
            new Error(`worker turn ${options.id}: ${done.run} has no product ${subject}`),
          )
        return Effect.orDie(Schema.decodeUnknownEffect(success)(value))
      }
      return options.execution === true
        ? Effect.map(decode(), (value) => ({
          value,
          ...(done.executions === undefined ? {} : { executions: done.executions }),
        }))
        : decode()
    }),
  )
}

/**
 * Asks a person on the program step and waits for the answer, journaled under `id`. An ask that a
 * cancelled run withdraws is never answered; one withdrawn while the run goes on fails with
 * `AskCancelled`.
 */
export const ask = (options: {
  readonly id: string
  readonly for: PersonReference
  readonly title: string
  readonly request: Schema.Json
  readonly deadline?: string
  readonly default?: 'approve' | 'reject'
}): Effect.Effect<AskAnswer, AskCancelled, ProgramServices> =>
  keyed(options.id, (key, program) => {
    const { id: _, for: person, ...request } = options
    program.requests.set(key, () => program.host.ask({ ...request, key, for: person.subject }))
    return DurableDeferred.await(
      DurableDeferred.make(key, { success: AskAnswer, error: AskCancelled }),
    ).pipe(Effect.ensuring(Effect.sync(() => program.requests.delete(key))))
  })

/**
 * Starts a linked child run of `mission` and waits for its end, journaled under `options.id`
 * (default: the mission's id). Cancelling the parent run cancels the child; a child that does not
 * complete fails with `ChildFailed`.
 */
export function run<I extends InputSchema>(
  mission: ProgramMission<I, any, any>,
  input: I['Type'],
  options?: { readonly id?: string },
): Effect.Effect<ChildRun, ChildFailed, ProgramServices>
export function run(
  mission: string,
  input: Readonly<Record<string, string>>,
  options?: { readonly id?: string },
): Effect.Effect<ChildRun, ChildFailed, ProgramServices>
export function run(
  mission: string | ProgramMission<InputSchema, Codec, unknown>,
  input: object,
  options?: { readonly id?: string },
): Effect.Effect<ChildRun, ChildFailed, ProgramServices> {
  if (typeof mission === 'string')
    return child(options?.id ?? mission, mission, input as Readonly<Record<string, string>>)
  const { id, input: schema } = mission.intent
  return Effect.flatMap(Effect.orDie(Schema.encodeUnknownEffect(schema)(input)), (encoded) =>
    child(options?.id ?? id, id, encoded),
  )
}

/**
 * Runs a command as its own process group, which the host ends when the program is cancelled.
 * `priority: 'background'` runs it as background work. Commands are not journaled: run them
 * inside a step.
 */
export const command = (
  argv: readonly string[],
  options?: CommandOptions,
): Effect.Effect<CommandResult, never, Program> =>
  Effect.flatMap(Effect.service(Program), (program) =>
    Effect.promise(() => program.host.spawn(argv, options)),
  )

/** The run's `ST_*` variables (`ST_MISSION_RUN`, `ST_STEP_RUN`, …). */
export const env: Effect.Effect<Readonly<Record<string, string>>, never, Program> = Effect.map(
  Effect.service(Program),
  (program) => program.host.env,
)

/** The run's workspace directory. */
export const cwd: Effect.Effect<string, never, Program> = Effect.map(
  Effect.service(Program),
  (program) => program.host.cwd,
)
