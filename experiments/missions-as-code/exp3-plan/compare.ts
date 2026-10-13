// Compare what the two extractors found. A static template such as round-${round}/ci matches
// probe IDs such as round-2/ci; a probe retry suffix (build.2) matches its base ID.

import type { ProbeResult } from "./probe.ts"
import { staticOps, type PlanNode } from "./static.ts"

export interface Comparison {
  readonly matched: Array<string> // static IDs the probe reached
  readonly staticOnly: Array<string> // static IDs the probe never reached
  readonly probeOnly: Array<string> // probe IDs no static ID accounts for: static blind spots
  readonly dynamic: Array<string> // static IDs it could not name at all
}

const pattern = (id: string) =>
  new RegExp(`^${id.split(/\$\{[^}]*\}/).map((part) => part.replace(/[.*+?^$()|[\]\\]/g, "\\$&")).join("[^/]+")}$`)

export const compare = (plan: PlanNode, probed: ProbeResult): Comparison => {
  const ops = staticOps(plan).filter((o) => o.kind !== "timer")
  const named = ops.filter((o) => o.idForm !== "dynamic")
  const reached = probed.ops.filter((o) => o.kind !== "timer").map((o) => o.op.replace(/\.\d+$/, ""))
  const matched = named.filter((o) => reached.some((r) => pattern(o.id).test(r))).map((o) => o.id)
  return {
    matched: [...new Set(matched)],
    staticOnly: [...new Set(named.filter((o) => !matched.includes(o.id)).map((o) => o.id))],
    probeOnly: [...new Set(reached.filter((r) => !named.some((o) => pattern(o.id).test(r))))],
    dynamic: ops.filter((o) => o.idForm === "dynamic").map((o) => o.id)
  }
}
