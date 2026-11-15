/**
 * The bundle side of the program host contract (docs/st3/program-engine.md §2, §5, §11): the
 * `fractal.program.v1` entry, the journal keys and the `WorkflowEngine` that runs one program
 * attempt inside the daemon's program host.
 *
 * The engine replays from the journal the daemon hands over: a journaled step returns its recorded
 * value, or fails again with its recorded typed error, without running. A new step runs, and the
 * program continues only once the daemon has acknowledged its result. Typed failures are journaled
 * with their `_tag` and fields; defects and interrupts are not, so a resumed attempt runs that step
 * again. Asks and child runs wait in the host until the daemon resolves them; it journals their
 * result under the same key.
 */
import { Cause, Context, Effect, Exit, Option, Schema, SchemaGetter, SchemaIssue } from 'effect'
import { Workflow, WorkflowEngine } from 'effect/workflow'
import type { LaneMembership } from './resources.ts'

/** An outcome as the host protocol carries it: a program's, or one journaled key's. */
export type ProgramExit =
  | { readonly _tag: 'Success'; readonly value: unknown }
  | { readonly _tag: 'Failure'; readonly error: unknown }

/** The key of the entry a bundle's default export carries: version 1 of the host contract. */
export const ProgramEntry: unique symbol = Symbol.for('fractal.program.v1')

export interface CommandOptions {
  readonly env?: Readonly<Record<string, string>>
  readonly cwd?: string
  /** Background work runs at background priority; the default is the daemon's live priority. */
  readonly priority?: 'background'
}

export interface CommandResult {
  readonly code: number
  readonly signal: string | null
  readonly stdout: string
  readonly stderr: string
}

export const CommandResultSchema = Schema.Struct({
  code: Schema.Number,
  signal: Schema.NullOr(Schema.String),
  stdout: Schema.String,
  stderr: Schema.String,
})

/** Admission failures are distinct from a command's exit status or a child run's failure. */
export const ResourceError = Schema.TaggedStruct('ResourceError', {
  code: Schema.Literals([
    'admission-expired', 'unsupported-coverage', 'impossible-capacity', 'clock-uncertain',
    'cancelled', 'retired', 'retiring', 'uncertain', 'invalid-state',
    'resource-operation-refused', 'execution-timeout', 'stale-membership', 'execution-refused',
  ]),
  message: Schema.String,
})
export type ResourceError = typeof ResourceError.Type

/** A step's result as the bundle hands it to the daemon to journal. */
export interface JournalEntry {
  readonly key: string
  readonly status: 'completed' | 'failed'
  readonly value?: unknown
  readonly error?: unknown
  readonly ms: number
}

/** A native resource operation. A grant is never fulfilled from the program journal alone. */
export interface ResourceRequest {
  readonly key: string
  readonly operation:
    | 'resource.lane.acquire'
    | 'resource.lane.release'
    | 'resource.lane.exec'
    | 'resource.lane.run'
  readonly payload: unknown
}

/** What the program host hands a bundle's entry for one attempt of one program step. */
export interface HostContextV1 {
  /** The program's step run subject. */
  readonly step: string
  readonly attempt: number
  /** The mission run subject. */
  readonly run: string
  /** Inherited from a lane-bound linked run; every native command consumes this context. */
  readonly resourceMembership?: LaneMembership
  /** The run's native inputs, by name. */
  readonly payload: Readonly<Record<string, string>>
  /** The run's `ST_*` variables. */
  readonly env: Readonly<Record<string, string>>
  readonly cwd: string
  /** This attempt's completed and failed results by key. */
  readonly journal: ReadonlyMap<string, ProgramExit>
  /** Aborted when the daemon cancels the attempt. */
  readonly signal: AbortSignal
  started(key: string): void
  /** Resolves once the daemon has made the entry durable; rejects on nack or cancel. */
  record(entry: JournalEntry): Promise<void>
  /**
   * Starts or adopts the linked child run for `key` (protocol message `run`); the daemon journals
   * its end under `key`.
   */
  child(request: {
    readonly key: string
    readonly mission: string
    readonly input: Readonly<Record<string, string>>
  }): Promise<ProgramExit>
  /** Opens or adopts the person ask for `key` on the program step; journaled under `key`. */
  ask(request: {
    readonly key: string
    readonly for: string
    readonly title: string
    readonly request: unknown
    readonly deadline?: string
    readonly default?: 'approve' | 'reject'
  }): Promise<ProgramExit>
  /** Runs a command as its own process group, which the host ends on cancel. */
  spawn(argv: readonly string[], options?: CommandOptions): Promise<CommandResult>
  /** Host-authoritative admission, execution and confirmed retirement; never local spawn. */
  resource(request: ResourceRequest, signal?: AbortSignal): Promise<ProgramExit>
}

/** One running program attempt: its host, the keys taken so far and its open asks and runs. */
export class Program extends Context.Service<
  Program,
  {
    readonly host: HostContextV1
    readonly occurrences: Map<string, number>
    /** By id: the fiber that uses it. */
    readonly owners: Map<string, number>
    readonly requests: Map<string, () => Promise<ProgramExit>>
  }
>()('fractal/Program') {}

/** Native resource requests inside a replayable activity use its identity, not global call order. */
export class ResourceCommandScope extends Context.Service<ResourceCommandScope, string>()('fractal/ResourceCommandScope') {}

/** What a program body may require: the attempt and the workflow it runs as. */
export type ProgramServices =
  | Program
  | WorkflowEngine.WorkflowEngine
  | WorkflowEngine.WorkflowInstance

/**
 * Runs `use` under the next key of `id`: `id`, then `id#2`, `id#3`… in one execution. Only one
 * fiber may use an id: repeats within a fiber (a loop, `Effect.retry`) take their keys in program
 * order, while concurrent branches would take them in completion order, which replay does not
 * reproduce, so a second fiber using the id is a defect.
 */
export const keyed = <A, E, R>(
  id: string,
  use: (key: string, program: Program['Service']) => Effect.Effect<A, E, R>,
): Effect.Effect<A, E, R | Program> =>
  Effect.withFiber((fiber) =>
    Effect.flatMap(Effect.service(Program), (program): Effect.Effect<A, E, R> => {
      const owner = program.owners.get(id)
      if (owner !== undefined && owner !== fiber.id)
        return Effect.die(
          new Error(`steps in concurrent branches share the id \`${id}\`; give each its own id`),
        )
      program.owners.set(id, fiber.id)
      const n = (program.occurrences.get(id) ?? 0) + 1
      program.occurrences.set(id, n)
      return use(n === 1 ? id : `${id}#${n}`, program)
    }),
  )

/** A typed failure: an error with a string `_tag`. */
export type Tagged = { readonly _tag: string }

const isTagged = (u: unknown): u is Tagged =>
  typeof u === 'object' && u !== null && '_tag' in u && typeof u._tag === 'string'

/**
 * The error codec of steps and programs that declare none: a tagged error is journaled as its
 * `_tag` plus its own fields and replays as that plain object, so `catchTag` matches; anything
 * else fails to encode, which makes it a defect.
 */
export const TaggedFailure = Schema.Json.pipe(
  Schema.decodeTo(Schema.Unknown, {
    decode: SchemaGetter.transformEffect((json: Schema.Json) =>
      isTagged(json)
        ? Effect.succeed<unknown>(json)
        : Effect.fail(
            new SchemaIssue.InvalidValue({ message: 'a typed failure has a _tag' }, json),
          ),
    ),
    encode: SchemaGetter.transformEffect((error: unknown) =>
      isTagged(error)
        ? Effect.succeed(JSON.parse(JSON.stringify({ ...error, _tag: error._tag })) as Schema.Json)
        : Effect.fail(
            new SchemaIssue.InvalidValue({ message: 'an untagged error is a defect' }, error),
          ),
    ),
  }),
)

const toExit = (exit: ProgramExit): Exit.Exit<unknown, unknown> =>
  exit._tag === 'Success' ? Exit.succeed(exit.value) : Exit.fail(exit.error)

const notInPrograms = (what: string) =>
  Effect.die(new Error(`${what} is not available in a Fractal program`))

/** The engine of one program attempt; it runs the one workflow registered with it. */
const hostEngine = (
  program: Program['Service'],
  refuse: (key: string, cause: unknown) => void,
): WorkflowEngine.Encoded => {
  const { host, requests } = program
  /** Waits for the ack; a refused entry ends the attempt and never resumes the step. */
  const durable = (entry: JournalEntry) =>
    Effect.promise(() =>
      host.record(entry).catch((cause: unknown) => {
        refuse(entry.key, cause)
        return new Promise<never>(() => {})
      }),
    )
  const workflows = new Map<
    string,
    (
      payload: object,
      executionId: string,
    ) => Effect.Effect<
      unknown,
      unknown,
      WorkflowEngine.WorkflowInstance | WorkflowEngine.WorkflowEngine
    >
  >()
  return {
    register: (workflow, execute) => Effect.sync(() => void workflows.set(workflow._tag, execute)),
    execute: <const Discard extends boolean>(
      workflow: Workflow.Any,
      options: {
        readonly executionId: string
        readonly payload: object
        readonly discard: Discard
      },
    ) => {
      const execute = workflows.get(workflow._tag)
      if (execute === undefined || options.discard) return notInPrograms('Workflow.execute')
      return execute(options.payload, options.executionId).pipe(
        Workflow.intoResult,
        Effect.provideService(
          WorkflowEngine.WorkflowInstance,
          WorkflowEngine.WorkflowInstance.initial(workflow, options.executionId),
        ),
        // Asks and child runs wait in the host, so nothing can resume a suspended program.
        Effect.flatMap((result) =>
          result._tag === 'Suspended'
            ? Effect.die(new Error('the program suspended on a wait the host cannot resume'))
            : Effect.succeed(result),
        ),
        // The workflow engine service reaches the body through `register`.
      ) as Effect.Effect<Discard extends true ? void : Workflow.Result<unknown, unknown>>
    },
    poll: () => notInPrograms('Workflow.poll'),
    interrupt: () => notInPrograms('Workflow.interrupt'),
    interruptUnsafe: () => notInPrograms('Workflow.interrupt'),
    resume: () => notInPrograms('Workflow.resume'),
    scheduleClock: () => notInPrograms('DurableClock'),
    deferredDone: () => notInPrograms('DurableDeferred.done'),
    activityExecute: (activity) =>
      Effect.gen(function* () {
        const key = activity.name
        const journaled = host.journal.get(key)
        if (journaled !== undefined) return new Workflow.Complete({ exit: toExit(journaled) })
        const instance = yield* WorkflowEngine.WorkflowInstance
        host.started(key)
        const started = Date.now()
        const result: Workflow.Result<unknown, unknown> = yield* activity.executeEncoded.pipe(
          Workflow.intoResult,
          Effect.provideService(
            WorkflowEngine.WorkflowInstance,
            WorkflowEngine.WorkflowInstance.initial(instance.workflow, instance.executionId),
          ),
        )
        if (result._tag === 'Suspended') return result
        const ms = Date.now() - started
        const { exit } = result
        if (Exit.isSuccess(exit)) {
          yield* durable({ key, status: 'completed', value: exit.value ?? null, ms })
        } else if (
          Cause.hasDies(exit.cause) === false &&
          Cause.hasInterrupts(exit.cause) === false
        ) {
          const error = Cause.findErrorOption(exit.cause)
          if (Option.isSome(error))
            yield* durable({ key, status: 'failed', error: error.value, ms })
        }
        return result
      }),
    deferredResult: (deferred) => {
      const journaled = host.journal.get(deferred.name)
      if (journaled !== undefined) return Effect.succeed(Option.some(toExit(journaled)))
      const request = requests.get(deferred.name)
      if (request === undefined) return Effect.succeed(Option.none())
      return Effect.map(Effect.promise(request), (exit) => Option.some(toExit(exit)))
    },
  }
}

/** A program as its bundle runs it. */
export interface ProgramDefinition<P, A, E> {
  readonly id: string
  readonly input: Workflow.AnyStructSchema & Schema.Codec<P, Readonly<Record<string, string>>>
  readonly output: Schema.Codec<A, unknown>
  readonly error: Schema.Codec<E, unknown>
  readonly body: (input: P) => Effect.Effect<A, E, ProgramServices>
}

const encodeJson = <T>(schema: Schema.Codec<T, unknown>, value: T) =>
  Effect.orDie(Schema.encodeUnknownEffect(Schema.toCodecJson(schema))(value))

/**
 * The `fractal.program.v1` entry: decodes the run's inputs, runs the body as a workflow whose
 * execution is the step run, and returns the success encoded with `output` or the typed failure
 * encoded with `error`. Inputs that do not decode refuse the run with `InvalidInput`; a defect or
 * a refused journal entry returns `Defect`. After a cancel the host discards the exit.
 */
export const programEntry =
  <P, A, E>(definition: ProgramDefinition<P, A, E>) =>
  async (host: HostContextV1): Promise<ProgramExit> => {
    // Ends the attempt when the daemon cancels it or refuses a journal entry. Interrupting the root
    // fiber, rather than failing the step, keeps the body from continuing past a result that is
    // not durable.
    const stop = new AbortController()
    const cancel = () => stop.abort()
    host.signal.addEventListener('abort', cancel, { once: true })
    if (host.signal.aborted) stop.abort()
    let refused: string | undefined
    const refuse = (key: string, cause: unknown) => {
      refused ??= `the daemon refused the journal entry for \`${key}\`: ${cause instanceof Error ? cause.message : String(cause)}`
      stop.abort()
    }
    const program: Program['Service'] = {
      host,
      occurrences: new Map(),
      owners: new Map(),
      requests: new Map(),
    }
    const engine = WorkflowEngine.makeUnsafe(hostEngine(program, refuse))
    const workflow = Workflow.make(definition.id, {
      payload: definition.input,
      success: definition.output,
      error: definition.error,
      idempotencyKey: () => host.step,
    })
    const attempt = Effect.gen(function* () {
      const decoded = yield* Effect.result(
        Schema.decodeUnknownEffect(definition.input, { onExcessProperty: 'error' })(host.payload),
      )
      if (decoded._tag === 'Failure')
        return {
          _tag: 'Failure',
          error: { _tag: 'InvalidInput', message: decoded.failure.message },
        } satisfies ProgramExit
      yield* engine.register(workflow, (input) => definition.body(input))
      const exit = yield* Effect.exit(
        engine.execute(workflow, { executionId: host.step, payload: decoded.success }),
      )
      if (Exit.isSuccess(exit))
        return {
          _tag: 'Success',
          value: (yield* encodeJson(definition.output, exit.value)) ?? null,
        } satisfies ProgramExit
      const error = Cause.findErrorOption(exit.cause)
      if (Cause.hasDies(exit.cause) || Cause.hasInterrupts(exit.cause) || Option.isNone(error))
        return yield* Effect.failCause(exit.cause)
      return {
        _tag: 'Failure',
        error: yield* encodeJson(definition.error, error.value),
      } satisfies ProgramExit
    }).pipe(
      Effect.scoped,
      Effect.provideService(Program, program),
      Effect.provideService(WorkflowEngine.WorkflowEngine, engine),
    )
    const exit = await Effect.runPromiseExit(attempt, { signal: stop.signal }).finally(() =>
      host.signal.removeEventListener('abort', cancel),
    )
    const message = Exit.isSuccess(exit) ? '' : Cause.pretty(exit.cause).slice(0, 2000)
    if (host.signal.aborted) return { _tag: 'Failure', error: { _tag: 'Interrupted', message } }
    if (refused !== undefined)
      return { _tag: 'Failure', error: { _tag: 'Defect', message: refused } }
    if (Exit.isSuccess(exit)) return exit.value
    return { _tag: 'Failure', error: { _tag: 'Defect', message } }
  }
