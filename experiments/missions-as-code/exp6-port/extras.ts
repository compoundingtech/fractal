// What the ports needed that sdk/mission.ts does not have yet, built locally on the same op
// primitive. Each is a KDL feature; the SDK should grow a real form for each.

import { Data, Effect } from "effect"
import { opEffect } from "../sdk/engine.ts"
import type { GateFailed, StepFailed, StepResult } from "../sdk/mission.ts"
import { scope } from "../sdk/mission.ts"

// KDL: an agentless step with a `message` declaration.
export const send = (
  id: string,
  message: { readonly to: string; readonly title: string; readonly content: string }
): Effect.Effect<StepResult, StepFailed> =>
  opEffect("step", id, { agentless: true, message }) as any

// KDL: `attention` inside on-exhausted. Here an agentless step the driver turns into a card;
// the program does not wait for anyone to read it.
export const attention = (
  id: string,
  card: { readonly title: string; readonly reviewer: string; readonly severity: "warning" | "error" }
): Effect.Effect<StepResult, StepFailed> =>
  opEffect("step", id, { agentless: true, attention: card }) as any

export class LoopExhausted extends Data.TaggedError("LoopExhausted")<{
  readonly loop: string
  readonly rounds: number
  readonly feedback: string
}> {}

// KDL `loop` with `max-rounds`, `until` and a `round`, as st runs it: each round runs its work
// first and then checks the exit gate; the next round reads the last gate failure as feedback.
export const loopUntil = <E>(
  name: string,
  options: {
    readonly maxRounds: number
    readonly round: (round: number, feedback: string) => Effect.Effect<unknown, E>
    readonly until: (round: number) => Effect.Effect<unknown, GateFailed>
  }
): Effect.Effect<number, E | LoopExhausted> =>
  Effect.gen(function* () {
    let feedback = ""
    for (let round = 1; round <= options.maxRounds; round++) {
      const result = yield* Effect.gen(function* () {
        yield* options.round(round, feedback)
        return yield* Effect.result(options.until(round))
      }).pipe(scope(`${name}/${round}`))
      if (result._tag === "Success") return round
      feedback = result.failure.reason
    }
    return yield* Effect.fail(new LoopExhausted({ loop: name, rounds: options.maxRounds, feedback }))
  })

// KDL: an agentless step with a `stop` declaration (stop a run-owned seat).
export const stopAgent = (id: string, agent: string): Effect.Effect<StepResult, StepFailed> =>
  opEffect("step", id, { agentless: true, stop: agent }) as any
