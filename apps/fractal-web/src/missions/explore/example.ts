// EXPLORATION ONLY: invented missions. One long mission (30 hours, 12 live steps, three server attempts, a load test
// that failed, a security review waiting on you for five hours, two plan revisions), plus short ones for list rows.
import type { ExploreMission, PlanStep, Problem } from './model.ts'

const MIN = 60_000
const HOUR = 60 * MIN
const T0 = Date.UTC(2026, 10, 6, 8, 0)
const at = (h: number, m = 0) => T0 + h * HOUR + m * MIN
const now = at(30)

const step = (fields: Partial<PlanStep> & Pick<PlanStep, 'id' | 'title' | 'state'>): PlanStep =>
  ({ after: [], attempts: [], waits: [], since: 1, ...fields })

const steps: readonly PlanStep[] = [
  step({ id: 'spec', title: 'Write the sync v2 spec', state: 'done', expectedMs: 45 * MIN,
    attempts: [{ n: 1, agent: 'planner', start: at(0), end: at(0, 40), outcome: 'done', turn: 'turn/planner/12' }] }),
  step({ id: 'schema', title: 'Change the wire schema', state: 'done', after: ['spec'], expectedMs: 90 * MIN,
    waits: [{ kind: 'queue', start: at(0, 40), end: at(1, 5), reason: 'schema-dev was busy with gateway/schema-v1' }],
    attempts: [{ n: 1, agent: 'schema-dev', start: at(1, 5), end: at(2, 25), outcome: 'done', turn: 'turn/schema-dev/88' }] }),
  step({ id: 'server', title: 'Implement the server side', state: 'done', after: ['schema'], expectedMs: 3 * HOUR,
    attempts: [
      { n: 1, agent: 'server-dev', start: at(2, 30), end: at(4, 50), outcome: 'failed', reason: 'migration test timed out after 20m', turn: 'turn/server-dev/140' },
      { n: 2, agent: 'server-dev', start: at(5, 10), end: at(6, 0), outcome: 'failed', reason: 'flaky lock test (claims::renew) failed twice', turn: 'turn/server-dev/171' },
      { n: 3, agent: 'server-dev', start: at(6, 20), end: at(8, 40), outcome: 'done', turn: 'turn/server-dev/190' },
    ],
    waits: [{ kind: 'queue', start: at(4, 50), end: at(5, 10), reason: 'retry backoff' }, { kind: 'queue', start: at(6, 0), end: at(6, 20), reason: 'retry backoff' }] }),
  step({ id: 'sdk', title: 'Update the client SDK', state: 'done', after: ['schema'], expectedMs: 3 * HOUR,
    attempts: [{ n: 1, agent: 'sdk-dev', start: at(2, 30), end: at(5, 40), outcome: 'done', turn: 'turn/sdk-dev/61' }] }),
  step({ id: 'migration', title: 'Dry-run the data migration', state: 'done', after: ['schema'], expectedMs: 2 * HOUR,
    waits: [{ kind: 'queue', start: at(2, 25), end: at(10, 5), reason: 'rewritten in revision 2; waited for the new step to publish' }],
    attempts: [{ n: 1, agent: 'server-dev', start: at(10, 5), end: at(12, 0), outcome: 'done', turn: 'turn/server-dev/233' }] }),
  step({ id: 'docs', title: 'Document the new sync API', state: 'done', after: ['sdk'], expectedMs: 90 * MIN,
    attempts: [{ n: 1, agent: 'docs-writer', start: at(6, 0), end: at(7, 10), outcome: 'done' }] }),
  step({ id: 'web', title: 'Move the web app onto sync v2', state: 'running', after: ['sdk'], expectedMs: 5 * HOUR,
    waits: [{ kind: 'queue', start: at(5, 40), end: at(26, 0), reason: 'web-dev was on missions-ui until today' }],
    attempts: [{ n: 1, agent: 'web-dev', start: at(26, 0), outcome: 'running', turn: 'turn/web-dev/402' }] }),
  step({ id: 'security', title: 'Security review of the token scope change', state: 'waiting-you', after: ['server'], expectedMs: 2 * HOUR,
    attempts: [{ n: 1, agent: 'sec-review', start: at(22, 0), end: at(25, 0), outcome: 'done', turn: 'turn/sec-review/9' }],
    waits: [{ kind: 'queue', start: at(8, 40), end: at(22, 0), reason: 'sec-review works one review at a time' }, { kind: 'you', start: at(25, 0), reason: 'Approve widening sync tokens to read.projections?' }] }),
  step({ id: 'load', title: 'Load test at 3× peak', state: 'failed', after: ['server', 'migration'], expectedMs: 90 * MIN, since: 2,
    attempts: [{ n: 1, agent: 'perf-bot', start: at(12, 0), end: at(13, 40), outcome: 'failed', reason: 'p99 commit latency 840ms (budget 400ms)', turn: 'turn/perf-bot/17' }] }),
  step({ id: 'canary', title: 'Canary at 5% for two hours', state: 'pending', after: ['security', 'load', 'web'], expectedMs: 2 * HOUR }),
  step({ id: 'rollout', title: 'Roll out to all hosts', state: 'pending', after: ['canary'], expectedMs: 3 * HOUR }),
  step({ id: 'announce', title: 'Announce sync v2', state: 'pending', after: ['rollout'], expectedMs: 30 * MIN }),
  step({ id: 'manual-qa', title: 'Manual QA pass', state: 'cancelled', after: ['server'] }),
]

const problems: readonly Problem[] = [
  { at: at(4, 50), kind: 'retry', step: 'server', agent: 'server-dev', text: 'Attempt 1 failed: migration test timed out after 20m', turn: 'turn/server-dev/140', resolvedAt: at(8, 40) },
  { at: at(6, 0), kind: 'retry', step: 'server', agent: 'server-dev', text: 'Attempt 2 failed: flaky lock test (claims::renew) failed twice', turn: 'turn/server-dev/171', resolvedAt: at(8, 40) },
  { at: at(13, 40), kind: 'failure', step: 'load', agent: 'perf-bot', text: 'Load test failed: p99 commit latency 840ms against a 400ms budget', turn: 'turn/perf-bot/17' },
  { at: at(5, 40), kind: 'stall', step: 'web', agent: 'web-dev', text: 'Ready for 20h: web-dev was busy with missions-ui', resolvedAt: at(26, 0) },
  { at: at(25, 0), kind: 'wait-you', step: 'security', text: 'Waiting on you: approve widening sync tokens to read.projections?' },
]

export const longMission: ExploreMission = {
  id: 'mission/platform/sync-v2',
  title: 'Ship sync v2',
  startedAt: T0,
  now,
  expectedMs: 34 * HOUR,
  steps,
  problems,
  revisions: [
    { n: 1, at: T0, by: 'planner', why: 'First plan', changes: steps.filter(s => s.since === 1).map(s => ({ kind: 'added' as const, step: s.id, after: s.title })) },
    { n: 2, at: at(10), by: 'you', why: 'Replace manual QA with a load test; dry-run the migration first', changes: [
      { kind: 'removed', step: 'manual-qa', before: 'Manual QA pass' },
      { kind: 'added', step: 'load', after: 'Load test at 3× peak' },
      { kind: 'rewritten', step: 'migration', before: 'Run the data migration', after: 'Dry-run the data migration' },
    ] },
    { n: 3, at: at(20), by: 'planner', why: 'Smaller, longer canary after the load test failure', changes: [
      { kind: 'rewritten', step: 'canary', before: 'Canary at 10% for one hour', after: 'Canary at 5% for two hours' },
    ] },
  ],
}

/** Short missions for list rows: one per interesting state. */
const short = (id: string, title: string, startH: number, specs: ReadonlyArray<[string, PlanStep['state'], number, number?]>, expectedMs: number): ExploreMission => {
  let cursor = at(startH)
  const built = specs.map(([sid, state, minutes, failedFirst], index): PlanStep => {
    const start = cursor
    cursor += minutes * MIN
    const attempts = state === 'pending' || state === 'ready' ? [] : [
      ...(failedFirst === undefined ? [] : [{ n: 1, agent: 'agent', start, end: start + failedFirst * MIN, outcome: 'failed' as const, reason: 'tests failed' }]),
      { n: failedFirst === undefined ? 1 : 2, agent: 'agent', start: failedFirst === undefined ? start : start + failedFirst * MIN, ...(state === 'running' || state === 'waiting-you' ? {} : { end: cursor }), outcome: state === 'failed' ? 'failed' as const : state === 'running' ? 'running' as const : 'done' as const },
    ]
    return step({ id: sid, title: sid, state, after: index === 0 ? [] : [specs[index - 1]![0]], attempts, expectedMs: minutes * MIN,
      waits: state === 'waiting-you' ? [{ kind: 'you', start, reason: 'needs your answer' }] : [] })
  })
  return { id, title, startedAt: at(startH), now, expectedMs, steps: built, problems: [], revisions: [] }
}

export const listMissions: readonly ExploreMission[] = [
  longMission,
  short('mission/webfractal/command-palette', 'Command palette', 27, [['design', 'done', 40], ['build', 'done', 70, 20], ['review', 'running', 60], ['ship', 'pending', 20]], 4 * 3_600_000),
  short('mission/docs/runbooks', 'Refresh on-call runbooks', 24, [['audit', 'done', 50], ['rewrite', 'waiting-you', 60], ['publish', 'pending', 10]], 3 * 3_600_000),
  short('mission/platform/flake-triage', 'Triage flaky CI checks', 28, [['collect', 'done', 30], ['bisect', 'failed', 50, 20], ['quarantine', 'pending', 20]], 2 * 3_600_000),
  short('mission/gateway/schema-v1', 'Freeze client schema v1', 29, [['diff', 'done', 20], ['regenerate', 'running', 30], ['publish', 'pending', 15]], 90 * 60_000),
  short('mission/platform/secrets', 'Rotate secrets', 2, [['inventory', 'done', 30], ['rotate', 'done', 90], ['verify', 'done', 20]], 3 * 3_600_000),
]
