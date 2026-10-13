// node exp4-hmr/cli.ts bundle FILE          bundle a program version, print its file and hash
// node exp4-hmr/cli.ts check FROM TO JOURNAL  check a journal (JSON lines from st subject history)
import { readFileSync } from "node:fs"
import { bundle, load } from "./bundle.ts"
import { checkUpgrade } from "./upgrade.ts"

const [command, ...rest] = process.argv.slice(2)
if (command === "bundle") {
  const v = await bundle(rest[0]!)
  console.log(JSON.stringify({ file: v.file, hash: v.hash, program: v.program.name }))
} else if (command === "check") {
  const to = await load(rest[0]!)
  const history = readFileSync(rest[1]!, "utf8").split("\n").filter((l) => l.startsWith("{")).map((l) => JSON.parse(l).body.fields).sort((a, b) => a.seq - b.seq).map((f) => f.event)
  const report = checkUpgrade(to, history)
  console.log(JSON.stringify({ verdict: report.verdict, reason: report.reason, changes: report.changes }, null, 2))
} else {
  console.error("usage: cli.ts bundle FILE | check TO_BUNDLE JOURNAL_JSONL")
  process.exit(2)
}
