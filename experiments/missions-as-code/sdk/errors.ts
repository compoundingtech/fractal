// Typed failures an op can raise. Each carries the op ID and the driver's reason, so
// Effect.catchTag("StepFailed", ...) reads like the KDL it replaces.

import { Data } from "effect"
import type { OpError, OpKind } from "./history.ts"

export class StepFailed extends Data.TaggedError("StepFailed")<{
  readonly op: string
  readonly reason: string
  readonly attempt?: number | undefined
  readonly evidence?: ReadonlyArray<string> | undefined
}> {}

export class GateFailed extends Data.TaggedError("GateFailed")<{
  readonly op: string
  readonly reason: string
}> {}

export class AskDeclined extends Data.TaggedError("AskDeclined")<{
  readonly op: string
  readonly reason: string
}> {}

export class WaitFailed extends Data.TaggedError("WaitFailed")<{
  readonly op: string
  readonly reason: string
}> {}

export const failureFor = (kind: OpKind, op: string, error: OpError) => {
  switch (kind) {
    case "step":
      return new StepFailed({ op, reason: error.reason, attempt: error.attempt, evidence: error.evidence })
    case "gate":
      return new GateFailed({ op, reason: error.reason })
    case "ask":
      return new AskDeclined({ op, reason: error.reason })
    default:
      return new WaitFailed({ op, reason: error.reason })
  }
}
