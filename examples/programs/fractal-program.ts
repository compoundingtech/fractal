/**
 * The program side of Fractal's program engine (docs/st3/program-engine.md): `Fractal.mission`,
 * `Fractal.step` and `Fractal.command` for programs the daemon runs. A program is bundled with this
 * module and Effect into one ES module (`bun build --target=bun`), and a mission step runs it with
 * `program "/abs/bundle.mjs" sha256="…"`. The daemon's program host loads the bundle and calls its
 * default export's `fractal.program.v1` entry with a host context; this module turns that context
 * into journaled steps.
 *
 * Replay: the daemon passes the journal (completed steps, and this attempt's typed failures) when
 * it starts or resumes the program. A journaled step returns its recorded value, decoded with its
 * `success` schema, or fails again with its recorded error, which comes back with its `_tag` and
 * fields, so `catchTag` still matches. A new step runs its body; the program continues only after
 * the daemon has made the result durable. Defects and interrupts are not journaled: a resumed
 * program runs that step again, so every step's effect must be idempotent.
 *
 * Spike scope: agentless steps only. Worker turns (`Fractal.Worker`), asks and child runs are the
 * engine's next milestones; a step without a body fails with `StepFailed`.
 */
import { Cause, Context, Data, Effect, Exit, Option, Schema } from 'effect'

export class StepFailed extends Data.TaggedError('StepFailed')<{ readonly step: string; readonly reason: string }> {}

type Entry =
  | { readonly key: string; readonly _tag: 'Success'; readonly value: unknown }
  | { readonly key: string; readonly _tag: 'Failure'; readonly error: unknown }

/** What the program host hands a bundle: version 1 of the host contract. */
export interface HostContext {
  readonly step: string
  readonly attempt: number
  readonly payload: Readonly<Record<string, string>>
  readonly env: Readonly<Record<string, string>>
  readonly cwd: string
  readonly journal: ReadonlyMap<string, Entry>
  readonly signal: AbortSignal
  readonly started: (key: string) => void
  readonly record: (entry: { key: string; status: 'completed' | 'failed'; value?: unknown; error?: unknown; ms: number }) => Promise<void>
  readonly spawn: (
    argv: readonly string[],
    options?: { readonly env?: Readonly<Record<string, string>>; readonly cwd?: string },
  ) => Promise<{ readonly code: number; readonly stdout: string; readonly stderr: string }>
}

/** The program run being executed: its host context and per-execution step occurrences. */
export class Run extends Context.Service<
  Run,
  { readonly host: HostContext; readonly occurrences: Map<string, number>; readonly active: Set<string> }
>()('fractal/program/Run') {}

export interface StepOptions<A> {
  readonly id: string
  readonly goal?: string
  readonly success?: Schema.Codec<A, unknown>
}

const ENTRY = Symbol.for('fractal.program.v1')

/** The n-th occurrence of one step id in an execution has key `id#n`: deterministic under replay. */
const occurrence = (run: Run['Service'], id: string) => {
  const n = (run.occurrences.get(id) ?? 0) + 1
  run.occurrences.set(id, n)
  return n === 1 ? id : `${id}#${n}`
}

const tagOf = (e: unknown) => (e !== null && typeof e === 'object' && '_tag' in e && typeof e._tag === 'string' ? e._tag : undefined)

/** A typed failure keeps its tag and fields in the journal and in the step's failure reason. */
const encodeError = (e: unknown): unknown => {
  const tag = tagOf(e)
  return tag === undefined ? { _tag: 'Defect', message: String(e) } : { ...(e as object), _tag: tag }
}

/** Replayed failures come back structurally; `StepFailed` is rebuilt so `instanceof` holds too. */
const decodeError = (e: unknown) => (tagOf(e) === 'StepFailed' ? new StepFailed(e as never) : e)

/** An agentless step: `body` runs once per run; its decoded result is journaled and replayed. */
function step<A, E, R>(options: StepOptions<A>, body?: Effect.Effect<unknown, E, R>): Effect.Effect<A, E | StepFailed, R | Run> {
  return Effect.gen(function* () {
    const run = yield* Effect.service(Run)
    const key = occurrence(run, options.id)
    const done = run.host.journal.get(key)
    if (done !== undefined) {
      if (done._tag === 'Failure') return yield* Effect.fail(decodeError(done.error) as E)
      return (options.success ? Schema.decodeUnknownSync(options.success as never)(done.value) : done.value) as A
    }
    if (body === undefined)
      return yield* new StepFailed({ step: key, reason: 'worker steps are not supported by this engine build' })
    // Two concurrent steps with one id would take `id` and `id#2` in scheduling order, which replay
    // cannot reproduce.
    if (run.active.has(options.id))
      return yield* Effect.die(new Error(`two concurrent steps share the id \`${options.id}\`; give each its own id`))
    run.active.add(options.id)
    run.host.started(key)
    const started = Date.now()
    const exit = yield* Effect.exit(body).pipe(Effect.ensuring(Effect.sync(() => run.active.delete(options.id))))
    const ms = Date.now() - started
    if (Exit.isSuccess(exit)) {
      // Decode once at the boundary; the journal holds the encoded form, so replay decodes the same way.
      const value = options.success ? Schema.decodeUnknownSync(options.success as never)(exit.value) : exit.value
      const encoded = options.success ? Schema.encodeUnknownSync(options.success as never)(value) : (value ?? null)
      yield* Effect.promise(() => run.host.record({ key, status: 'completed', value: encoded, ms }))
      return value as A
    }
    const error = Cause.findErrorOption(exit.cause)
    if (Option.isNone(error)) return yield* Effect.failCause(exit.cause as Cause.Cause<never>)
    yield* Effect.promise(() => run.host.record({ key, status: 'failed', error: encodeError(error.value), ms }))
    return yield* Effect.fail(error.value as E)
  })
}

/** Run a command as a step effect, in its own process group that the host ends on cancellation. */
const command = (argv: readonly string[], options?: { readonly env?: Readonly<Record<string, string>>; readonly cwd?: string }) =>
  Effect.flatMap(Effect.service(Run), (run) => Effect.promise(() => run.host.spawn(argv, options)))

/** The run's variables (`ST_MISSION_RUN`, `ST_STEP_RUN`, ...). */
const env = Effect.map(Effect.service(Run), (run) => run.host.env)

export interface MissionOptions<P, A> {
  readonly id: string
  readonly goal: string
  readonly payload: Schema.Codec<P, unknown>
  readonly success?: Schema.Codec<A, unknown>
}

export interface Mission<P, A, E> {
  readonly options: MissionOptions<P, A>
  readonly body: (payload: P) => Effect.Effect<A, E, Run>
  readonly [ENTRY]: (host: HostContext) => Promise<{ _tag: 'Success'; value: unknown } | { _tag: 'Failure'; error: unknown }>
}

const mission = <P, A, E>(options: MissionOptions<P, A>, body: (payload: P) => Effect.Effect<A, E, Run>): Mission<P, A, E> => ({
  options,
  body,
  [ENTRY]: async (host) => {
    const program = Effect.suspend(() => body(Schema.decodeUnknownSync(options.payload as never)(host.payload) as P))
    const exit = await Effect.runPromiseExit(
      program.pipe(Effect.provideService(Run, { host, occurrences: new Map(), active: new Set() })),
      { signal: host.signal },
    )
    if (Exit.isSuccess(exit))
      return { _tag: 'Success', value: options.success ? Schema.encodeUnknownSync(options.success as never)(exit.value) : (exit.value ?? null) }
    const error = Cause.findErrorOption(exit.cause)
    return {
      _tag: 'Failure',
      error: Option.isSome(error) ? encodeError(error.value) : { _tag: 'Defect', message: Cause.pretty(exit.cause).slice(0, 2000) },
    }
  },
})

export const Fractal = { mission, step, command, env }
