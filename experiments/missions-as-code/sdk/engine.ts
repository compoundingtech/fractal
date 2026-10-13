// decide(program, history) -> Decision: the pure core.
//
// It replays a run's history through an Effect program and reports what the program waits on.
// It performs no I/O and reads no clock or randomness of its own:
//
// - Scheduler: a synchronous FIFO queue drained by hand, so the program runs until every live
//   fiber waits on an op, and the same history always produces the same interleaving.
// - Clock: "now" is the time of the history event being applied. Clock.sleep is a durable timer
//   op, so Effect.sleep, Effect.timeout and Schedule delays survive a restart.
// - Random: seeded from the run's start event.
//
// Events are applied one at a time in history order, draining between each, which is what makes
// a race (a step against its timeout) resolve the same way on every replay.

import { Cause, Clock, Context, Duration, Effect, Exit, Random, Scheduler } from "effect"
import type {
  Command,
  Decision,
  Divergence,
  HistoryEvent,
  Json,
  OpError,
  OpKind,
  Status
} from "./history.ts"
import { hashJson } from "./hash.ts"
import { failureFor } from "./errors.ts"

export interface Program {
  readonly name: string
  readonly version: string
  readonly run: (input: any) => Effect.Effect<any, any, never>
}

interface OpState {
  readonly op: string
  readonly kind: OpKind
  readonly spec: Json
  readonly hash: string
  readonly resume: (effect: Effect.Effect<Json, unknown>) => void
  // open: the program waits on it. settled: a result reached the program.
  // abandoned: the program stopped waiting (interrupted by a race, a timeout or its end).
  state: "open" | "settled" | "abandoned"
  scheduled: boolean
  finished: boolean // the history holds its result or its cancellation
}

export class Engine {
  now: number
  readonly ops = new Map<string, OpState>()
  readonly order: Array<string> = []
  private readonly counters = new Map<string, number>()
  private readonly queue: Array<() => void> = []
  private draining = false
  cursor = 0
  drains = 0
  divergence: Divergence | undefined
  private seed: number

  constructor(startAt: number, seedText: string) {
    this.now = startAt
    this.seed = parseInt(hashJson(seedText).slice(0, 8), 16) >>> 0
  }

  readonly scheduler: Scheduler.Scheduler = {
    executionMode: "sync",
    shouldYield: () => false,
    makeDispatcher: () => ({
      scheduleTask: (task) => {
        this.queue.push(task)
      },
      flush: () => this.drain()
    })
  }

  readonly clock: Clock.Clock = {
    currentTimeMillisUnsafe: () => this.now,
    currentTimeMillis: Effect.sync(() => this.now),
    currentTimeNanosUnsafe: () => BigInt(this.now) * 1_000_000n,
    currentTimeNanos: Effect.sync(() => BigInt(this.now) * 1_000_000n),
    monotonicTimeNanosUnsafe: () => BigInt(this.now) * 1_000_000n,
    monotonicTimeNanos: Effect.sync(() => BigInt(this.now) * 1_000_000n),
    sleep: (duration) => {
      const ms = Duration.toMillis(duration)
      if (ms <= 0) return Effect.void
      return Effect.withFiber((fiber) =>
        opEffect("timer", fiber.getRef(TimerNameRef), { ms, fireAt: this.now + ms }, { ms })
      ).pipe(
        Effect.asVoid,
        Effect.orDie
      )
    }
  }

  // mulberry32, seeded per run.
  readonly random: Random.Random = {
    nextIntUnsafe: () => Math.floor(this.nextDouble() * 2 ** 31),
    nextDoubleUnsafe: () => this.nextDouble()
  }

  private nextDouble(): number {
    let t = (this.seed += 0x6d2b79f5)
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }

  drain(): void {
    if (this.draining) return
    this.draining = true
    try {
      this.drains++
      while (this.queue.length > 0) {
        const task = this.queue.shift()!
        task()
      }
    } finally {
      this.draining = false
    }
  }

  // Op IDs are the author's name inside the current scope. A name used again in the same scope
  // (a retry, a loop without its own scope) gets .2, .3, ... in the order the program asks. The
  // suffix uses only characters st allows in a subject name.
  allocate(scope: ReadonlyArray<string>, name: string): string {
    const base = [...scope, name].join("/")
    const n = (this.counters.get(base) ?? 0) + 1
    this.counters.set(base, n)
    return n === 1 ? base : `${base}.${n}`
  }

  register(op: string, kind: OpKind, spec: Json, hash: string, resume: OpState["resume"]): void {
    this.ops.set(op, { op, kind, spec, hash, resume, state: "open", scheduled: false, finished: false })
    this.order.push(op)
  }

  abandon(op: string): void {
    const state = this.ops.get(op)
    if (state && state.state === "open") state.state = "abandoned"
  }

  diverge(op: string, reason: string): void {
    if (!this.divergence) this.divergence = { at: this.cursor, op, reason }
  }

  apply(event: HistoryEvent): void {
    switch (event.type) {
      case "started":
      case "upgraded":
        return
      case "scheduled": {
        const state = this.ops.get(event.op)
        if (!state) return this.diverge(event.op, "the history scheduled an op this program never asked for here")
        if (state.kind !== event.kind) return this.diverge(event.op, `the op was a ${event.kind} and is now a ${state.kind}`)
        if (state.hash !== event.hash) return this.diverge(event.op, "the op's definition changed since it was scheduled")
        state.scheduled = true
        return
      }
      case "completed":
      case "failed": {
        const state = this.ops.get(event.op)
        if (!state) return this.diverge(event.op, "the history holds a result for an op this program has not asked for yet")
        state.finished = true
        if (state.state !== "open") return // a late result for an op the program stopped waiting on
        state.state = "settled"
        state.resume(
          event.type === "completed"
            ? Effect.succeed(event.value)
            : Effect.fail(failureFor(state.kind, event.op, event.error))
        )
        this.drain()
        return
      }
      case "cancelled": {
        const state = this.ops.get(event.op)
        if (state) state.finished = true
        return
      }
    }
  }
}

export const EngineRef = Context.Reference<Engine | undefined>("missions-as-code/Engine", {
  defaultValue: () => undefined
})

// The name Clock.sleep gives the timers it opens. Effect.timeout and Effect.retry open timers
// through Clock.sleep, so without a name they are counted (timer, timer.2, ...), and a count
// changes meaning when another path opens a timer first. timerName(...) in mission.ts names them.
export const TimerNameRef = Context.Reference<string>("missions-as-code/TimerName", {
  defaultValue: () => "timer"
})

export const ScopeRef = Context.Reference<ReadonlyArray<string>>("missions-as-code/Scope", {
  defaultValue: () => []
})

// The one primitive every SDK op uses.
export const opEffect = (
  kind: OpKind,
  name: string,
  spec: Json,
  hashed: Json = spec
): Effect.Effect<Json, unknown> =>
  Effect.withFiber((fiber) => {
    const engine = fiber.getRef(EngineRef)
    if (!engine) return Effect.die(new Error(`${kind} "${name}" can only run inside decide()`))
    const op = engine.allocate(fiber.getRef(ScopeRef), name)
    const hash = hashJson({ kind, spec: hashed })
    return Effect.callback<Json, unknown>((resume) => {
      engine.register(op, kind, spec, hash, resume)
      return Effect.sync(() => engine.abandon(op))
    })
  })

const statusOf = (exit: Exit.Exit<unknown, unknown> | undefined): Status => {
  if (!exit) return { state: "running" }
  if (Exit.isSuccess(exit)) return { state: "completed", value: toJson(exit.value) }
  const failure = exit.cause.reasons.find(Cause.isFailReason)
  if (failure) return { state: "failed", error: toJson(failure.error) }
  const defect = exit.cause.reasons.find(Cause.isDieReason)
  if (defect) return { state: "failed", error: { defect: String((defect as any).defect) } }
  return { state: "failed", error: { interrupted: true } }
}

const toJson = (value: unknown): Json => {
  if (value === undefined) return null
  return JSON.parse(JSON.stringify(value, (_k, v) => (typeof v === "bigint" ? v.toString() : v)))
}

export const decide = (program: Program, history: ReadonlyArray<HistoryEvent>): Decision => {
  const start = history[0]
  if (!start || start.type !== "started") throw new Error("a history starts with a started event")
  const engine = new Engine(start.at, `${start.program}:${start.at}`)
  const runnable = program.run(start.input).pipe(
    Effect.provideService(EngineRef, engine),
    Effect.provideService(Clock.Clock, engine.clock),
    Effect.provideService(Random.Random, engine.random)
  )
  const fiber = Effect.runFork(runnable, { scheduler: engine.scheduler })
  engine.drain()
  for (let i = 1; i < history.length && !engine.divergence; i++) {
    const event = history[i]!
    engine.cursor = i
    engine.now = event.at
    engine.apply(event)
  }
  const commands: Array<Command> = []
  const open: Array<Decision["open"][number]> = []
  for (const op of engine.order) {
    const state = engine.ops.get(op)!
    if (state.state === "open") {
      open.push({ op, kind: state.kind, spec: state.spec, scheduled: state.scheduled })
      if (!state.scheduled) commands.push({ type: "schedule", op, kind: state.kind, spec: state.spec, hash: state.hash })
    } else if (state.state === "abandoned" && state.scheduled && !state.finished) {
      commands.push({ type: "cancel", op })
    }
  }
  const status = engine.divergence ? { state: "running" as const } : statusOf(fiber.pollUnsafe())
  return {
    status,
    open: engine.divergence ? [] : open,
    commands: engine.divergence ? [] : commands,
    ...(engine.divergence ? { divergence: engine.divergence } : {}),
    stats: { events: history.length, ops: engine.order.length, drains: engine.drains }
  }
}

export type { OpError }
