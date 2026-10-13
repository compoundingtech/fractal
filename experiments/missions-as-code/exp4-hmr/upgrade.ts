// Can a running mission move to a new version? Replay its history through the new version and
// see. The answer is one of four verdicts; only the first two move the run.
//
//   compatible               the new version replays the history exactly
//   compatible-with-changes  some finished ops are defined differently now; their results stand
//   open-op-changed          an op the run still waits on is defined differently; the run already
//                            started it with the old definition, so moving would lie about it
//   incompatible             the history does not replay: the new version asks for different
//                            ops, in a different order, or not at all
//
// A run that cannot move stays on the version it has: that version's bundle is immutable, so the
// run keeps replaying exactly as before.

import type { Decision, HistoryEvent } from "../sdk/history.ts"
import type { Version } from "./bundle.ts"

export type Verdict = "compatible" | "compatible-with-changes" | "open-op-changed" | "incompatible"

export interface UpgradeReport {
  readonly verdict: Verdict
  readonly reason: string
  readonly changes: NonNullable<Decision["changes"]>
  readonly decision: Decision
}

export const checkUpgrade = (to: Pick<Version, "program" | "decide">, history: ReadonlyArray<HistoryEvent>): UpgradeReport => {
  const decision = to.decide(to.program, history, { onChangedSpec: "report" })
  const changes = decision.changes ?? []
  if (decision.divergence) {
    const d = decision.divergence
    return { verdict: "incompatible", reason: `event ${d.at}, op ${d.op}: ${d.reason}`, changes, decision }
  }
  const open = changes.filter((c) => !c.settled)
  if (open.length > 0) {
    return { verdict: "open-op-changed", reason: `still waiting on ${open.map((c) => c.op).join(", ")}, started under the old definition`, changes, decision }
  }
  if (changes.length > 0) {
    return { verdict: "compatible-with-changes", reason: `finished ops defined differently now: ${changes.map((c) => c.op).join(", ")}`, changes, decision }
  }
  return { verdict: "compatible", reason: "the new version replays the history exactly", changes, decision }
}
