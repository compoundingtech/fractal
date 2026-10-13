// The alert demo (an-alert-needs-an-answer.kdl) as an Effect program. The beats file has to
// assume Robin answers "tonight"; the program follows whichever answer Robin clicks.

import { Effect } from "effect"
import { mission } from "../../sdk/mission.ts"
import { after, ask, claim, complete, focus, onKey, open, progress, say, tool, type } from "./demo.ts"

const migrator = "agent/marrowpine/migrator"
const oncall = "agent/marrowpine/oncall"
const run = "marrowpine/invoice-index/1"

export default mission("demo/an-alert-needs-an-answer", () =>
  Effect.gen(function* () {
    yield* onKey(open(migrator))
    yield* claim(migrator, run, "migrate").pipe(after("1s"))
    yield* tool(migrator, "Bash", {
      input: "./rehearse-migration --snapshot nightly",
      takes: "3s",
      output: "index created in 3m 52s\ninvoices table locked for writes the whole time"
    }).pipe(after("1s"))
    yield* say(migrator, "The rehearsal worked, but the invoices table is **locked for writes for about four minutes**.\n\nDuring business hours that blocks anyone saving an invoice. I'll ask Robin before running it.").pipe(after("1s"))
    yield* progress(migrator, run, "migrate", "Rehearsed: a 4 minute write lock on invoices. Asking Robin when to run it.").pipe(after("0.5s"))
    const answer = yield* ask(
      migrator,
      "The due-date index locks the invoices table for about four minutes. Run it now, or tonight at 02:00 when nobody is saving invoices?",
      { run, step: "migrate", title: "Run the invoice migration now?", why: "It locks invoice saving for about four minutes; only you can choose when customers notice." },
      [
        { id: "tonight", label: "Tonight at 02:00", then: "The migrator schedules it for 02:00 and tells on-call." },
        { id: "now", label: "Run it now", outcome: "decline", then: "The migrator runs it at once; saving invoices waits about four minutes." }
      ]
    ).pipe(after("1.5s"))

    // Robin answers the alert by clicking it, then presses F8.
    if (answer === "tonight") {
      yield* onKey(focus(oncall))
      yield* say(oncall, "Heads up: the invoice migration runs tonight at 02:00. I'll watch the write latency then.").pipe(after("1s"))
      yield* focus(migrator).pipe(after("2s"))
      yield* onKey(type("Thanks. Post the timing in the morning, please.", { send: true }))
      yield* say(migrator, "Will do. Scheduled for 02:00; I'll post how long the lock took when it's done.").pipe(after("1.5s"))
    } else {
      // The path the beats file cannot show: Robin chose now.
      yield* say(migrator, "Running it now. Saving invoices will wait for about four minutes.").pipe(after("1s"))
      yield* tool(migrator, "Bash", { input: "./run-migration --index due_date", takes: "4s", output: "index created in 3m 49s" }).pipe(after("0.5s"))
      yield* complete(migrator, run, "migrate", "The due-date index is live; the write lock lasted 3m 49s.").pipe(after("1s"))
      yield* onKey(focus(oncall))
      yield* say(oncall, "Saw the write latency spike and settle. All clear.").pipe(after("1s"))
    }
  })
)
