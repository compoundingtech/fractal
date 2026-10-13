// The mission SDK: a mission is an Effect.gen program and each yield is a step, gate, ask or wait.
//
// Agents stay declared in KDL as seats today. A step names the seat that does it, exactly as
// `assigned-to` does. Everything else is ordinary Effect:
//
//   retries          Effect.retry(step, Schedule...)        each attempt is a new op: build, build.2
//   timeouts         Effect.timeout / timeoutOrElse          the deadline is a durable timer op
//   escalation       escalateAfter(...) below                 a timeout whose fallback asks a person
//   on-failure       Effect.catchTag("StepFailed", ...)       or onError / ensuring for cleanup
//   loops            for / while with scope(`round-${n}`)     names each round's ops
//   parallel steps   Effect.all([...], { concurrency })       ops open together; first failure cancels the rest

import { Duration, Effect } from "effect"
import { opEffect, ScopeRef, TimerNameRef } from "./engine.ts"
import type { AskDeclined, GateFailed, StepFailed, WaitFailed } from "./errors.ts"
import type { Json } from "./history.ts"

export * from "./errors.ts"
export type { Program } from "./engine.ts"

// ---- steps ------------------------------------------------------------------------------------

export interface StepSpec {
  readonly title?: string
  readonly goal?: string | ReadonlyArray<string>
  readonly constraint?: string | ReadonlyArray<string>
  // A declared seat: "agent/fleet/example/builder". Exactly the KDL selector.
  readonly assignedTo?: string
  readonly availableTo?: ReadonlyArray<string>
  // st's own per-attempt execution budget, as in KDL `timeout=`.
  readonly timeout?: string
  // st's native retry for transient failure. Program-level retry is Effect.retry.
  readonly retry?: { readonly attempts: number; readonly backoff?: string }
  readonly freshContext?: boolean
  readonly documents?: ReadonlyArray<string>
  // Gates on the step itself, checked by st before the step completes.
  readonly gates?: ReadonlyArray<GateSpec & { readonly name: string }>
  // Values the program wants the worker to see; rendered into the step's goal text.
  readonly context?: Json
}

export interface StepResult {
  readonly summary: string
  readonly evidence: ReadonlyArray<string>
  readonly attempt: number
}

export const step = (id: string, spec: StepSpec): Effect.Effect<StepResult, StepFailed> =>
  opEffect("step", id, spec as unknown as Json) as any

// ---- gates ------------------------------------------------------------------------------------

export type GateSpec =
  | { readonly exec: string; readonly host?: string; readonly workspace?: string; readonly timeLimit?: string }
  | { readonly human: { readonly reviewer: string; readonly question?: string; readonly review?: ReadonlyArray<string> } }
  | { readonly ciPassed: string; readonly repo: string; readonly commit?: string; readonly branch?: string }
  | { readonly merged: string }
  | { readonly document: string }
  | { readonly field: readonly [path: string, subject: string, op: "is" | "starts-with" | "contains", value: Json] }

export interface GateResult {
  readonly answer: "pass"
  readonly evidence: ReadonlyArray<string>
}

// A gate on its own: the program stops here until st answers it. Not yet keeps waiting, as an
// exec gate does today; a failure raises GateFailed.
export const gate = (id: string, spec: GateSpec): Effect.Effect<GateResult, GateFailed> =>
  opEffect("gate", id, spec as unknown as Json) as any

// ---- asks -------------------------------------------------------------------------------------

export interface AskSpec {
  readonly for: string // person/NAME
  readonly title: string
  readonly question: string
  readonly answers?: ReadonlyArray<{ readonly id: string; readonly label: string; readonly consequence?: string }>
  readonly recommendation?: string
}

export interface Answer {
  readonly id: string
  readonly text?: string
}

export const ask = (id: string, spec: AskSpec): Effect.Effect<Answer, AskDeclined> =>
  opEffect("ask", id, spec as unknown as Json) as any

// ---- waits ------------------------------------------------------------------------------------

export type Condition =
  | { readonly merged: string }
  | { readonly ciPassed: string; readonly repo: string; readonly commit?: string; readonly branch?: string }
  | { readonly exists: string }
  | { readonly field: readonly [path: string, subject: string, op: "is" | "starts-with" | "contains", value: Json] }
  | { readonly run: string }

// Wait for something outside the program, with no worker.
export const waitFor = (id: string, condition: Condition): Effect.Effect<Json, WaitFailed> =>
  opEffect("wait", id, condition as unknown as Json) as any

// ---- structure --------------------------------------------------------------------------------

// Name the ops inside: scope("round-2", step("fix")) opens round-2/fix.
export const scope =
  (name: string) =>
  <A, E, R>(self: Effect.Effect<A, E, R>): Effect.Effect<A, E, R> =>
    Effect.flatMap(ScopeRef, (outer) => Effect.provideService(self, ScopeRef, [...outer, name]))

// Name the timers Effect opens inside (retry delays, timeouts): timerName("build-backoff")
// gives build-backoff, build-backoff.2, ... instead of a count shared with every other timer.
export const timerName =
  (name: string) =>
  <A, E, R>(self: Effect.Effect<A, E, R>): Effect.Effect<A, E, R> =>
    Effect.provideService(self, TimerNameRef, name)

// A named durable sleep.
export const sleep = (id: string, duration: Duration.Input): Effect.Effect<void> =>
  Effect.sleep(duration).pipe(timerName(id))

// A deadline whose fallback is a person's decision, not a failure. The deadline is its own named
// timer, so a replay or a new version finds it by name.
export const escalateAfter =
  <B, E2, R2>(name: string, after: Duration.Input, escalate: () => Effect.Effect<B, E2, R2>) =>
  <A, E, R>(self: Effect.Effect<A, E, R>): Effect.Effect<A | B, E | E2, R | R2> =>
    Effect.raceFirst(
      Effect.map(self, (value) => ({ late: false as const, value })),
      Effect.as(sleep(name, after), { late: true as const })
    ).pipe(
      Effect.flatMap((r): Effect.Effect<A | B, E2, R2> => (r.late ? escalate() : Effect.succeed(r.value)))
    )

export interface MissionDefinition<I> {
  readonly name: string
  readonly version: string
  readonly run: (input: I) => Effect.Effect<any, any, never>
}

export const mission = <I>(
  name: string,
  run: (input: I) => Effect.Effect<any, any, never>,
  version = "dev"
): MissionDefinition<I> => ({ name, version, run })
