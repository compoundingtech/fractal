import { Attention, Mission, MissionDeclaration, UsagePeriod, Work, decodeUnknownSync, type AttentionEncoded, type MissionEncoded, type MissionStepEncoded, type WorkEncoded } from '@fractal/client/schema'
import type * as Native from '@fractal/client/schema'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'

import type { NativeDetailsSource } from '../../data/nativeDetailsSource.ts'
import { observed, unavailable, waiting, type Feed } from '../../data/source.ts'

import { worldNow } from '../../fixtures/world.ts'
import type { UsageRowLike } from './detailModel.ts'

/**
 * A long mission as Fractal serves it today, for the mission detail: 30 hours in, twelve steps over one open run with
 * the selected attempt's `started_at`/`completed_at` per step, a failed load test, a security review waiting on the
 * person, and usage rows attributed by step. `withServedHistory` adds the M2 per-step history (attempts, waits, eta)
 * onto the decoded steps, as a newer daemon serves it. Invented data; the seats are the fixture world's agents.
 */

const MIN = 60_000
const HOUR = 60 * MIN
export const syncNow = worldNow
export const syncStart = syncNow - 30 * HOUR
const at = (h: number, m = 0) => syncStart + h * HOUR + m * MIN
const iso = (ms: number) => new Date(ms).toISOString()

const path = 'platform/sync-v2'
export const syncMissionRef = `mission/${path}`
export const syncRunRef = `mission-run/${path}/2`
const generation = `${path}/2/g1`
export const syncStep = (step: string) => `step-run/${generation}/${step}`
const seat = (slug: string, host = 'build-host-a') => `agent/${host}/${slug}`
export const syncSeats = {
  planner: seat('docs-writer', 'build-host-b'),
  schema: seat('gateway-schema'),
  server: seat('workbench-shell'),
  sdk: seat('sdk-interop', 'build-host-c'),
  web: seat('terminal-renderer'),
  security: seat('missions-ui'),
  load: seat('cas-janitor', 'build-host-b'),
} as const

type Spec = readonly [step: string, title: string, state: MissionStepEncoded['state'], extra: Partial<MissionStepEncoded>]
const specs: readonly Spec[] = [
  ['spec', 'Write the sync v2 spec', 'completed', { claimant: syncSeats.planner, started_at: at(0), completed_at: at(0, 40), since: iso(at(0, 40)) }],
  ['schema', 'Change the wire schema', 'completed', { claimant: syncSeats.schema, started_at: at(1, 5), completed_at: at(2, 25), since: iso(at(2, 25)) }],
  ['server', 'Implement the server side', 'completed', { claimant: syncSeats.server, attempt: 3, started_at: at(6, 20), completed_at: at(8, 40), since: iso(at(8, 40)) }],
  ['sdk', 'Update the client SDK', 'completed', { claimant: syncSeats.sdk, started_at: at(2, 30), completed_at: at(5, 40), since: iso(at(5, 40)) }],
  ['migration', 'Dry-run the data migration', 'completed', { claimant: syncSeats.server, started_at: at(10, 5), completed_at: at(12, 0), since: iso(at(12, 0)) }],
  ['docs', 'Document the new sync API', 'completed', { claimant: syncSeats.planner, started_at: at(6, 0), completed_at: at(7, 10), since: iso(at(7, 10)) }],
  ['web', 'Move the web app onto sync v2', 'claimed', { claimant: syncSeats.web, assignee: syncSeats.web, started_at: at(26, 0), since: iso(at(26, 0)), last_progress: 'Missions and agents feeds moved; attention is next.' }],
  ['security', 'Security review of the token scope change', 'blocked', { claimant: syncSeats.security, assignee: syncSeats.security, started_at: at(22, 0), since: iso(at(25, 0)), blocked_reason: 'waiting on your approval to widen sync tokens' }],
  ['load', 'Load test at 3× peak', 'failed', { claimant: syncSeats.load, started_at: at(12, 0), completed_at: at(13, 40), since: iso(at(13, 40)) }],
  ['canary', 'Canary at 5% for two hours', 'waiting', { since: iso(at(0)) }],
  ['rollout', 'Roll out to all hosts', 'waiting', { since: iso(at(0)) }],
  ['announce', 'Announce sync v2', 'waiting', { since: iso(at(0)) }],
  ['manual-qa', 'Manual QA pass', 'cancelled', { since: iso(at(10)) }],
]
const edges: ReadonlyArray<readonly [string, string]> = [
  ['spec', 'schema'], ['schema', 'server'], ['schema', 'sdk'], ['schema', 'migration'], ['sdk', 'docs'], ['sdk', 'web'],
  ['server', 'security'], ['server', 'load'], ['migration', 'load'], ['security', 'canary'], ['load', 'canary'], ['web', 'canary'],
  ['canary', 'rollout'], ['rollout', 'announce'], ['server', 'manual-qa'],
]

const steps: MissionStepEncoded[] = specs.map(([step, title, state, extra]) => ({ id: syncStep(step), path: step, title, state, attempt: 1, since: iso(at(0)), ...extra }))

const wire = ({ finishedEarlier = false }: { readonly finishedEarlier?: boolean } = {}): MissionEncoded => ({
  id: syncMissionRef,
  kind: 'mission',
  revision: 'r-sync-3',
  updated_at: iso(syncNow - 4 * MIN),
  title: 'Ship sync v2',
  state: 'running',
  mission_revision: 'sha256:5ab0c41e9d',
  must_act: 'you',
  active_runs: 1,
  runs: [`mission-run/${path}/1`, syncRunRef],
  run_generations: { [`mission-run/${path}/1`]: `run-generation/${path}/1/g1`, [syncRunRef]: `run-generation/${generation}` },
  resources: [
    { kind: 'uri', name: 'server', resolution: 'resolved', subject: 'resource/uri/0a1b', uri: 'https://github.com/acme/garden/pull/7' },
    { kind: 'uri', name: 'tracking', resolution: 'resolved', subject: 'resource/uri/0a1c', uri: 'https://github.com/acme/garden/issues/9' },
    { kind: 'uri', name: 'spec', resolution: 'resolved', subject: 'resource/uri/0a1d', uri: 'https://docs.acme.dev/sync-v2' },
  ],
  run_details: [
    ...(finishedEarlier ? [{
      id: `mission-run/${path}/1`, generation_id: `run-generation/${path}/1/g1`, status: 'cancelled', phase: 'cancelled', must_act: 'nobody' as const,
      requester: 'person/operator', state_since: iso(syncStart - 2 * HOUR), started_at: syncStart - 6 * HOUR, completed_at: syncStart - 2 * HOUR,
      progress: { done: 1, total: 2 }, current_steps: [], last_progress: null, outcome: null,
      steps: [
        { id: `step-run/${path}/1/g1/spec`, path: 'spec', title: 'Write the sync v2 spec', state: 'completed' as const, attempt: 1, since: iso(syncStart - 5 * HOUR), started_at: syncStart - 6 * HOUR, completed_at: syncStart - 5 * HOUR, claimant: syncSeats.planner },
        { id: `step-run/${path}/1/g1/schema`, path: 'schema', title: 'Change the wire schema', state: 'cancelled' as const, attempt: 1, since: iso(syncStart - 2 * HOUR) },
      ],
    }] : []),
    {
      id: syncRunRef, generation_id: `run-generation/${generation}`, status: 'running', phase: 'security', must_act: 'you',
      requester: 'person/operator', state_since: iso(at(25)), started_at: syncStart, completed_at: null,
      progress: { done: steps.filter(step => step.state === 'completed').length, total: steps.length },
      current_steps: [], last_progress: 'Missions and agents feeds moved; attention is next.', outcome: null,
      eta: { as_of: syncNow, basis: { method: 'critical-path', samples: 4 }, remaining_low_ms: 6 * HOUR, remaining_high_ms: 9 * HOUR },
      steps,
    },
  ],
  usage: null,
  visualization: {
    version: 'st3.visualization.v0', views: ['graph'], mission: syncMissionRef, decisions: [], diffs: [], gates: [], groups: [], live_progress: {}, resources: [], revision: {}, risk: {}, swimlanes: [], timeline: { entries: [] },
    nodes: specs.map(([step, title]) => ({ id: step, kind: 'step', label: step, path: step, goals: [title], constraints: [], runtime: {}, timeout_ms: null })),
    edges: edges.map(([from, to]) => ({ id: `${from}->${to}`, from, to, kind: 'dependency', gate: null })),
    goals: ['Clients follow collections with a v2 cursor; v1 readers keep working for 60 days.'], constraints: [],
  },
})

/** Ship sync v2 as a mission detail read serves it today. */
export const syncMission: Mission = decodeUnknownSync(Mission, 'strict')(wire())
/** The same mission with an earlier, cancelled run whose steps only the detail read carries. */
export const syncMissionWithHistory: Mission = decodeUnknownSync(Mission, 'strict')(wire({ finishedEarlier: true }))

/** The human gate on the security review. */
export const syncAttention: readonly Attention[] = [decodeUnknownSync(Attention, 'strict')({
  id: 'attention/01JSYNCSEC', kind: 'attention', revision: 'a1', updated_at: iso(at(25)), attention_kind: 'human-gate',
  title: 'Approve widening sync tokens to read.projections?', detail: 'The scope change is safe if sync tokens stay bound to one device.',
  priority: 'high', state: 'open', requested_at: iso(at(25)), person_id: 'person/operator', source_id: syncStep('security'),
  mission_id: syncMissionRef, mission_run_id: syncRunRef, step_run_id: syncStep('security'), actions: ['work.done'],
} satisfies AttentionEncoded)]

/** Usage rows a `usage.period` read since the run's start returns: by step, one run-level row, and another mission's. */
export const syncUsageRows: readonly UsageRowLike[] = [
  ...([['spec', 0.84, 61_000], ['schema', 1.92, 140_000], ['server', 6.4, 510_000], ['server', 5.0, 400_000], ['sdk', 3.1, 260_000], ['migration', 2.2, 150_000],
    ['docs', 0.6, 48_000], ['web', 4.8, 390_000], ['security', 1.3, 95_000], ['load', 2.7, 120_000]] as const)
    .map(([step, usd, tokens]) => ({ step: syncStep(step), mission_run: syncRunRef, cost_microusd: Math.round(usd * 1_000_000), total_tokens: tokens })),
  { mission_run: syncRunRef, cost_microusd: 400_000, total_tokens: 20_000 },
  { step: 'step-run/other/1/g1/build', mission_run: 'mission-run/other/1', cost_microusd: 9_000_000, total_tokens: 800_000 },
]

/** The person's answer recorded on the security step's work. */
export const syncSecurityWork: Work = decodeUnknownSync(Work)({
  id: syncStep('security'), kind: 'work', revision: 'w3', updated_at: iso(at(25)), mission_id: syncMissionRef, mission_run_id: syncRunRef,
  generation_id: `run-generation/${generation}`, definition_id: 'definition/sync-v2', path: 'security', state: 'blocked', title: 'Security review of the token scope change',
  attempt: 1, readiness_epoch: 1, blockers: [], goals: [], constraints: [],
  person_answers: [{ ask: 'ask/sec-1', answered_at_unix_ms: at(14, 30), respondent: 'person/operator', status: 'completed', summary: 'Hold the canary until the index fix lands, then rerun the load test.', answer: { type: 'feedback', outcome: 'feedback', choices: [], words: 'Hold the canary until the index fix lands.' } }],
} as unknown as WorkEncoded)

/**
 * The M2 history a newer daemon serves on a detail read, attached onto already-decoded steps the way a regenerated
 * schema would hand it over (raw arrays; Option-wrapped scalars are read the same).
 */
export const withServedHistory = (mission: Mission): Mission => {
  const history: Readonly<Record<string, object>> = {
    server: {
      attempts: [
        { n: 1, claimant: syncSeats.server, started_at: at(2, 30), ended_at: at(4, 50), outcome: 'failed', reason: 'migration test timed out after 20m' },
        { n: 2, claimant: syncSeats.server, started_at: at(5, 10), ended_at: at(6, 0), outcome: 'failed', reason: 'flaky lock test (claims::renew) failed twice' },
        { n: 3, claimant: syncSeats.server, started_at: at(6, 20), ended_at: at(8, 40), outcome: 'done' },
      ],
      waits: [
        { kind: 'dependency', attempt: 1, start: at(0), end: at(2, 25) },
        { kind: 'retry', attempt: 1, start: at(4, 50), end: at(5, 10), reason: 'retry backoff' },
        { kind: 'retry', attempt: 2, start: at(6, 0), end: at(6, 20), reason: 'retry backoff' },
      ],
      transitions: [], history_complete: true,
    },
    security: {
      attempts: [{ n: 1, claimant: syncSeats.security, started_at: at(22, 0), ended_at: null, outcome: 'running' }],
      waits: [
        { kind: 'queue', attempt: 1, start: at(8, 40), end: at(22, 0), reason: 'missions-ui works one review at a time' },
        { kind: 'you', attempt: 1, start: at(25, 0), end: null, reason: 'Approve widening sync tokens to read.projections?' },
      ],
      transitions: [], history_complete: true,
    },
    canary: { attempts: [], waits: [{ kind: 'dependency', attempt: 1, start: at(0), end: null }], transitions: [], history_complete: true, eta: { low_ms: 90 * MIN, high_ms: 150 * MIN, samples: 3 } },
  }
  return Object.assign({}, mission, {
    run_details: (mission.run_details ?? []).map(run => Object.assign({}, run, {
      steps: Option.map(run.steps, list => list.map(step => history[step.path] === undefined ? step : Object.assign({}, step, history[step.path]))),
    })),
  }) as Mission
}

/** A `usage.period` read over rows: what the gateway returns for the mission detail's spend. */
export const usagePeriodOf = (rows: readonly UsageRowLike[], since = syncStart): Native.UsagePeriod => decodeUnknownSync(UsagePeriod)({
  since_ms: since, until_ms: syncNow,
  rows: rows.map(row => ({ agent: syncSeats.server, input_tokens: row.total_tokens, output_tokens: 0, cached_tokens: 0, cache_write_tokens: 0, cache_write_1h_tokens: 0, reported_cost_microusd: 0, unpriced_tokens: 0, ...row })),
})

/** The native details a fixture source carries for the mission detail: spend observed, still loading, or unavailable. */
export const missionDetailsFixture = (spend: 'observed' | 'waiting' | 'unavailable' = 'observed', rows: readonly UsageRowLike[] = syncUsageRows): NativeDetailsSource => {
  const notHere = Atom.make<Feed<never>>(unavailable({ reason: 'unsupported', detail: 'Not in this fixture' }))
  const period = Atom.make<Feed<Native.UsagePeriod>>(spend === 'observed' ? observed({ value: usagePeriodOf(rows) }) : spend === 'waiting' ? waiting : unavailable({ reason: 'unsupported', detail: 'This gateway does not serve usage.' }))
  return { usagePeriod: () => notHere, missionUsage: () => period, rawDeclarations: () => notHere, declarations: () => notHere, retry: () => undefined, refresh: () => undefined }
}

const kdl = `mission "platform/sync-v2" {
  title "Ship sync v2"
  goal "Clients follow collections with a v2 cursor; v1 readers keep working for 60 days."

  step "spec" { goal "Write the sync v2 spec"; assignee "agent/build-host-b/docs-writer" }
  step "schema" { after "spec"; goal "Change the wire schema"; assignee "agent/build-host-a/gateway-schema" }
  step "server" { after "schema"; goal "Implement the server side"; assignee "agent/build-host-a/workbench-shell"; retry max=3 }
  step "sdk" { after "schema"; goal "Update the client SDK"; assignee "agent/build-host-c/sdk-interop" }
  step "migration" { after "schema"; goal "Dry-run the data migration"; assignee "agent/build-host-a/workbench-shell" }
  step "docs" { after "sdk"; goal "Document the new sync API"; assignee "agent/build-host-b/docs-writer" }
  step "web" { after "sdk"; goal "Move the web app onto sync v2"; assignee "agent/build-host-a/terminal-renderer" }
  step "security" { after "server"; goal "Security review of the token scope change"
    assignee "agent/build-host-a/missions-ui"; gate "human" { ask "Approve widening sync tokens to read.projections?" } }
  step "load" { after "server" "migration"; goal "Load test at 3x peak"; assignee "agent/build-host-b/cas-janitor" }
  step "canary" { after "security" "load" "web"; goal "Canary at 5% for two hours" }
  step "rollout" { after "canary"; goal "Roll out to all hosts" }
  step "announce" { after "rollout"; goal "Announce sync v2" }
}
`
/** The canonical KDL Fractal serves for the mission. */
export const syncDeclaration: MissionDeclaration = decodeUnknownSync(MissionDeclaration)({ mission: syncMissionRef, revision: 'sha256:5ab0c41e9d', published_at: at(0), text: kdl, legacy: false })
