// EXPLORATION ONLY: the long example mission with the context the refined detail adds: spend per step, conversation
// excerpts, linked work, agents with live state, and the published definition.
import { longMission } from './example.ts'
import type { ExploreMission, PlanStep } from './model.ts'

const MIN = 60_000
const HOUR = 60 * MIN
const at = (h: number, m = 0) => longMission.startedAt + h * HOUR + m * MIN
const repo = { owner: 'acme', repo: 'garden' }

const extras: Readonly<Record<string, Partial<PlanStep>>> = {
  spec: { usage: { costUsd: 0.84, tokens: 61_000 }, work: [{ kind: 'doc', title: 'docs/sync-v2.md' }],
    excerpts: [{ kind: 'decision', at: at(0, 30), who: 'planner', text: 'Keep v1 readers working for 60 days; v2 adds a cursor, never changes a field.', turn: 'turn/planner/12' }] },
  schema: { usage: { costUsd: 1.92, tokens: 140_000 }, work: [{ kind: 'pull-request', ...repo, number: 7, title: 'Add the sync v2 cursor to the wire schema' }] },
  server: { usage: { costUsd: 11.4, tokens: 910_000 },
    work: [{ kind: 'pull-request', ...repo, number: 7, title: 'Server side of sync v2' }, { kind: 'run', ...repo, runId: 4012, title: 'CI: migration-tests' }],
    excerpts: [
      { kind: 'failure', at: at(4, 50), who: 'server-dev', text: 'migration-tests timed out at 20m: the backfill holds one transaction for the whole table.', turn: 'turn/server-dev/140' },
      { kind: 'decision', at: at(5, 5), who: 'server-dev', text: 'Batch the backfill in 10k-row transactions and retry.', turn: 'turn/server-dev/151' },
      { kind: 'failure', at: at(6, 0), who: 'server-dev', text: 'claims::renew failed twice under load; it is the known flaky lock test, quarantining it for this run.', turn: 'turn/server-dev/171' },
    ] },
  sdk: { usage: { costUsd: 3.1, tokens: 260_000 }, work: [{ kind: 'pull-request', ...repo, number: 7, title: 'SDK: follow sync v2 cursors' }] },
  migration: { usage: { costUsd: 2.2, tokens: 150_000 } },
  docs: { usage: { costUsd: 0.6, tokens: 48_000 }, work: [{ kind: 'doc', title: 'docs/sync-v2-api.md' }] },
  web: { usage: { costUsd: 4.8, tokens: 390_000 }, work: [{ kind: 'issue', ...repo, number: 7, title: 'Web: move feeds to sync v2' }],
    excerpts: [{ kind: 'note', at: at(27, 10), who: 'web-dev', text: 'Missions and agents feeds moved; attention is next.', turn: 'turn/web-dev/410' }] },
  security: { usage: { costUsd: 1.3, tokens: 95_000 },
    excerpts: [
      { kind: 'decision', at: at(24, 40), who: 'sec-review', text: 'The scope change is safe if sync tokens stay bound to one device; asking you to approve.', turn: 'turn/sec-review/9' },
    ] },
  load: { usage: { costUsd: 2.7, tokens: 120_000 }, work: [{ kind: 'run', ...repo, runId: 4019, title: 'Load test 3× peak' }],
    excerpts: [
      { kind: 'failure', at: at(13, 40), who: 'perf-bot', text: 'p99 commit latency 840ms against a 400ms budget; the cursor index is missing on hosts with old schemas.', turn: 'turn/perf-bot/17' },
      { kind: 'answer', at: at(14, 30), who: 'you', text: 'Hold the canary until the index fix lands, then rerun the load test.', turn: 'turn/you/3' },
    ] },
}

const kdl = `mission "platform/sync-v2" {
  title "Ship sync v2"
  goal "Clients follow collections with a v2 cursor; v1 readers keep working for 60 days."
  revision 3

  step "spec" { goal "Write the sync v2 spec"; assignee "agent/planner" }
  step "schema" { after "spec"; goal "Change the wire schema"; assignee "agent/schema-dev" }
  step "server" { after "schema"; goal "Implement the server side"; assignee "agent/server-dev"; retry max=3 }
  step "sdk" { after "schema"; goal "Update the client SDK"; assignee "agent/sdk-dev" }
  step "migration" { after "schema"; goal "Dry-run the data migration"   // rewritten in revision 2
    assignee "agent/server-dev" }
  step "docs" { after "sdk"; goal "Document the new sync API"; assignee "agent/docs-writer" }
  step "web" { after "sdk"; goal "Move the web app onto sync v2"; assignee "agent/web-dev" }
  step "security" { after "server"; goal "Security review of the token scope change"
    assignee "agent/sec-review"; gate "human" { ask "Approve widening sync tokens to read.projections?" } }
  step "load" { after "server" "migration"; goal "Load test at 3× peak"   // added in revision 2
    assignee "agent/perf-bot"; budget p99="400ms" }
  step "canary" { after "security" "load" "web"; goal "Canary at 5% for two hours" }  // rewritten in revision 3
  step "rollout" { after "canary"; goal "Roll out to all hosts" }
  step "announce" { after "rollout"; goal "Announce sync v2" }
}`

export const richMission: ExploreMission = {
  ...longMission,
  steps: longMission.steps.map(step => ({ ...step, ...extras[step.id] })),
  agents: [
    { id: 'agent/web-dev', name: 'web-dev', state: 'working', step: 'web' },
    { id: 'agent/sec-review', name: 'sec-review', state: 'needs-you', step: 'security' },
    { id: 'agent/perf-bot', name: 'perf-bot', state: 'failed', step: 'load' },
    { id: 'agent/server-dev', name: 'server-dev', state: 'idle' },
    { id: 'agent/sdk-dev', name: 'sdk-dev', state: 'idle' },
    { id: 'agent/planner', name: 'planner', state: 'idle' },
    { id: 'agent/schema-dev', name: 'schema-dev', state: 'stopped' },
    { id: 'agent/docs-writer', name: 'docs-writer', state: 'idle' },
  ],
  kdl,
  source: { repo: 'acme/factory', path: 'missions/platform/sync-v2.ts', commit: '4be91c0' },
}
