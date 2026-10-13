// The release-day demo (release-day.kdl, from the demo-scripts mission) as an Effect program.
// The seed is unchanged; this replaces the beats block. compile.ts turns it back into beats.

import { Effect } from "effect"
import { mission } from "../../sdk/mission.ts"
import { after, claim, complete, focus, onKey, open, progress, say, send, tool } from "./demo.ts"

const builder = "agent/copperwren/builder"
const reviewer = "agent/copperwren/reviewer"
const run = "copperwren/release/2-3"

// Written once, used twice: the full suite, green.
const suite = (takes: string) =>
  tool(builder, "Bash", { input: "cargo test --workspace", takes, output: "test result: ok. 412 passed; 0 failed; 0 ignored" })

export default mission("demo/release-day", () =>
  Effect.gen(function* () {
    // The presenter starts recording, then presses F8.
    yield* onKey(open(builder))
    yield* claim(builder, run, "build").pipe(after("1.5s"))
    yield* open(`mission-run/${run}`, { split: "below", by: builder }).pipe(after("1s"))
    yield* say(builder, "Picked up the build step for 2.3. Building and running the full suite now.").pipe(after("1s"))
    yield* suite("3s").pipe(after("0.5s"))
    yield* progress(builder, run, "build", "412 tests pass; packaging the release build.").pipe(after("0.5s"))
    yield* send(builder, reviewer, "The 2.3 build is green: 412 tests pass. The rainfall chart is behind the new-charts flag, as you asked.", { subject: "2.3 is built" }).pipe(after("2s"))

    yield* onKey(open(reviewer, { split: "right", by: builder }))
    yield* say(reviewer, "Thanks. Reading the rainfall chart change now.\n\nThe flag is right. One fix before it ships: the y axis says **mm** but the values are inches.").pipe(after("1.5s"))
    yield* send(reviewer, builder, "The rainfall y axis label says mm; the series is inches. Change the label to in, please.", { subject: "Axis units" }).pipe(after("1.5s"))

    yield* onKey(focus(builder))
    yield* say(builder, "Good catch. Fixing the label to **in** and rebuilding.").pipe(after("1.5s"))
    yield* tool(builder, "Edit", { input: "web/charts/rainfall.ts", takes: "1.2s", output: "1 line changed" }).pipe(after("0.5s"))
    yield* suite("2.5s").pipe(after("0.3s"))
    yield* complete(builder, run, "build", "2.3 built with the axis fix; 412 tests pass.").pipe(after("1s"))

    yield* claim(reviewer, run, "review").pipe(after("1.5s"))
    yield* say(reviewer, "The axis now reads in. Reviewing the release notes next.").pipe(after("1s"))
    yield* onKey(open("doc/copperwren/release-notes-2-3", { split: "right", by: reviewer }))
  })
)
