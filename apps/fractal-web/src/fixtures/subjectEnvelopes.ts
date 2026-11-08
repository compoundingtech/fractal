import {
  agentByRef,
  ciRuns,
  decisionD12,
  gatewayHost,
  minutesAgo,
  missionByRef,
  operator,
  pullRequests,
  type MustAct as WorldMustAct,
  type WorldCiRun,
  type WorldPullRequest,
} from './world.ts'
import type { Fence } from '../data/subjectEnvelope.ts'
import type { EnvelopeOf } from './subjectEnvelopeSchemas.ts'

/** Local envelope fixtures projected from the shared fixture world. */

const pick = <T>({ value, what }: { readonly value: T | undefined; readonly what: string }): T => {
  if (value === undefined) throw new Error(`subject envelope fixtures: world has no ${what}`)
  return value
}

const mission = pick({
  value: missionByRef('mission/webfractal/foundation'),
  what: 'mission/webfractal/foundation',
})
const shellAgent = pick({
  value: agentByRef('agent/build-host-a/workbench-shell'),
  what: 'agent/build-host-a/workbench-shell',
})
const repoRef = (repo: string) => `resource/github/${repo}`

const host = `host/${gatewayHost}`
const snapshot = 'snap-01JDQ7Y4M2'
const observedAt = minutesAgo(0.03)

const mustAct = { human: 'you', agent: 'agent', none: 'nobody' } as const satisfies Record<
  WorldMustAct,
  string
>

const fence = ({
  subject,
  revision,
  extra = {},
}: {
  readonly subject: string
  readonly revision: string
  readonly extra?: Partial<Fence>
}): Fence => ({
  snapshot_id: snapshot,
  subject_revisions: { [subject]: revision },
  ...extra,
})

const projection = { source: 'projection', snapshot_id: snapshot, host_id: host } as const

const latestRun = `mission-run/01JDQ6ZK${String(mission.runs).padStart(2, '0')}`
const previousRun = `mission-run/01JDQ6ZK${String(mission.runs - 1).padStart(2, '0')}`

/** `st3.mission@1` for the world's webfractal foundation mission: one running run waiting on you. */
export const missionEnvelope = {
  schema: 'st3.mission@1',
  ref: mission.ref,
  family: 'mission',
  revision: 'r-41',
  observed_at: observedAt,
  provenance: projection,
  live: { collection: 'missions', key: mission.ref, subscribe: { limit: 200 } },
  actions: [
    {
      id: 'mission.revise',
      label: 'Revise',
      fence: fence({ subject: mission.ref, revision: 'r-41' }),
      enabled: true,
      risk: 'confirm',
    },
    {
      id: 'mission.cancel',
      label: 'Cancel mission',
      fence: fence({
        subject: mission.ref,
        revision: 'r-41',
        extra: { mission_generation: `gen-${mission.runs}` },
      }),
      enabled: true,
      risk: 'destructive',
    },
  ],
  data: {
    title: mission.title,
    state: 'running',
    mission_revision: 'mrev-7c1e',
    must_act: mustAct[mission.mustAct],
    active_runs: 1,
    runs: Array.from(
      { length: mission.runs },
      (_, index) => `mission-run/01JDQ6ZK${String(mission.runs - index).padStart(2, '0')}`,
    ),
    updated_at: mission.updatedAt,
    run_details: [
      {
        id: latestRun,
        phase: 'build',
        status: 'running',
        must_act: 'you',
        progress: { done: 4, total: 7 },
        requester: operator.ref,
        state_since: minutesAgo(148),
        last_progress: `${decisionD12.title} — waiting on ${operator.name}`,
        current_steps: [
          {
            id: `step-run/gen-${mission.runs}/${decisionD12.step}`,
            title: decisionD12.title,
            state: 'waiting',
            since: decisionD12.raisedAt,
            assignee: operator.ref,
          },
        ],
      },
      {
        id: previousRun,
        phase: 'scaffold',
        status: 'completed',
        must_act: 'nobody',
        progress: { done: 3, total: 3 },
        requester: operator.ref,
        state_since: minutesAgo(1246),
        current_steps: [],
        outcome: { status: 'completed', reason: 'all steps done', at: minutesAgo(1246) },
      },
    ],
  },
} satisfies EnvelopeOf<'st3.mission@1'>

/** `st3.agent@1` for the workbench-shell agent mid tool call. */
export const agentEnvelope = {
  schema: 'st3.agent@1',
  ref: shellAgent.ref,
  family: 'agent',
  revision: 'r-218',
  observed_at: observedAt,
  provenance: projection,
  live: { collection: 'agents', key: shellAgent.ref, subscribe: { limit: 200 } },
  actions: [
    {
      id: 'runtime.stop',
      label: 'Stop',
      fence: fence({
        subject: shellAgent.ref,
        revision: 'r-218',
        extra: { runtime_incarnation: 'inc-5' },
      }),
      enabled: true,
      risk: 'destructive',
    },
  ],
  data: {
    name: shellAgent.name,
    state: 'running',
    reachability: 'local',
    driver: shellAgent.session.harness,
    harness_state: 'tool_call',
    last_activity_at: shellAgent.session.lastActivityAt,
    queued_work_count: 1,
    current_work: [
      {
        id: `step-run/gen-${mission.runs}/workbench-shell`,
        mission_id: mission.ref,
        mission_run_id: latestRun,
        path: 'build/workbench-shell',
        since: shellAgent.session.startedAt,
        state: 'claimed',
        title: shellAgent.status,
        goal: mission.goal,
      },
    ],
  },
} satisfies EnvelopeOf<'st3.agent@1'>

/** `st3.pty@1` for the workbench-shell terminal running a Storybook build. */
export const ptyEnvelope = {
  schema: 'st3.pty@1',
  ref: shellAgent.terminal,
  family: 'pty',
  revision: 'r-77',
  observed_at: observedAt,
  provenance: { source: 'runtime', snapshot_id: snapshot, host_id: host },
  live: {
    collection: 'terminal',
    key: shellAgent.terminal,
    subscribe: { terminal: shellAgent.terminal, incarnation: 'inc-2' },
    requires: 'terminal.attach',
  },
  actions: [
    {
      id: 'runtime.stop',
      label: 'Stop',
      fence: fence({
        subject: shellAgent.terminal,
        revision: 'r-77',
        extra: { runtime_incarnation: 'inc-2', terminal_sequence: 4182 },
      }),
      enabled: true,
      risk: 'destructive',
    },
  ],
  data: {
    runtime_id: shellAgent.slug,
    state: 'running',
    owner_id: shellAgent.ref,
    terminal_id: shellAgent.terminal,
    incarnation_id: 'inc-2',
    title: 'storybook build · webfractal',
    columns: 120,
    rows: 32,
    preview: [
      `${shellAgent.session.cwd} $ ../../node_modules/.bin/storybook build --disable-telemetry`,
      'storybook v10.6.0',
      '',
      'info => Cleaning outputDir: storybook-static',
      'info => Loading presets',
      'info => Building manager..',
      'info => Manager built (212 ms)',
      'info => Building preview..',
      'vite v7.1.4 building for production...',
      'transforming (1184) ../../node_modules/@overeng/example-kit/src/components/Table/Table.tsx',
      '✓ 2391 modules transformed.',
      'rendering chunks (38)...',
    ],
  },
} satisfies EnvelopeOf<'st3.pty@1'>

/** `st3.attention@1` for decision D12, open and high priority. */
export const attentionEnvelope = {
  schema: 'st3.attention@1',
  ref: decisionD12.ref,
  family: 'attention',
  revision: 'r-2',
  observed_at: observedAt,
  provenance: projection,
  live: { collection: 'attention', key: decisionD12.ref, subscribe: { person: decisionD12.owner } },
  actions: [
    {
      id: 'attention.resolve',
      label: 'Resolve',
      fence: fence({ subject: decisionD12.ref, revision: 'r-2' }),
      enabled: true,
      risk: 'confirm',
    },
  ],
  data: {
    attention_kind: 'agent-request',
    title: decisionD12.title,
    detail: decisionD12.question,
    priority: 'high',
    state: 'open',
    requested_at: decisionD12.raisedAt,
    person_id: decisionD12.owner,
    source_id: decisionD12.raisedBy,
    because: `${pick({ value: agentByRef(decisionD12.raisedBy), what: decisionD12.raisedBy }).name} escalated ${decisionD12.id}; recommends ${decisionD12.recommendation}`,
    what: decisionD12.options
      .map((option) => `${option.id}. ${option.label} — ${option.tradeoff}`)
      .join('\n'),
    mission_id: decisionD12.mission,
  },
} satisfies EnvelopeOf<'st3.attention@1'>

const observer = (provider: string) =>
  ({
    source: 'observer',
    snapshot_id: snapshot,
    host_id: host,
    observer: `observer/${gatewayHost}/github`,
    provider,
  }) as const

/** `vcs.pull-request` envelope for any world PR; checks come from its latest CI run's jobs. */
export const pullRequestEnvelopeFor = (pr: WorldPullRequest) => {
  const latest = ciRuns.find((candidate) => candidate.pullRequest === pr.ref)
  return {
    schema: 'st3.resource:vcs.pull-request@1',
    ref: pr.ref,
    family: 'resource',
    revision: 'r-9',
    observed_at: minutesAgo(2.8),
    provenance: observer('github-pr'),
    live: null,
    actions: [],
    data: {
      number: pr.number,
      title: pr.title,
      state: pr.state,
      url: pr.url,
      repository: repoRef(pr.repo),
      author: pr.author,
      branch: pr.branch,
      base: `${repoRef(pr.repo)}/ref/main`,
      head_sha: pr.headSha,
      draft: pr.state === 'draft',
      merged: pr.state === 'merged',
      created_at: pr.openedAt,
      updated_at: pr.updatedAt,
      opened_by: pr.agent,
      ...(latest === undefined
        ? {}
        : {
            checks: latest.jobs.map((job) => ({
              name: job.name,
              status: job.conclusion === 'pending' ? 'in_progress' : 'completed',
              conclusion: job.conclusion === 'pending' ? null : job.conclusion,
            })),
          }),
      reviews: pr.reviews.map((review) => ({ reviewer: review.reviewer, state: review.state })),
    },
  } satisfies EnvelopeOf<'st3.resource:vcs.pull-request@1'>
}

/** `ci.run` envelope for any world CI run. */
export const ciRunEnvelopeFor = (ci: WorldCiRun) =>
  ({
    schema: 'st3.resource:ci.run@1',
    ref: ci.ref,
    family: 'resource',
    revision: 'r-4',
    observed_at: minutesAgo(2.8),
    provenance: observer('github-ci'),
    live: null,
    actions: [],
    data: {
      name: ci.workflow,
      status: ci.status,
      provider: 'github-actions',
      url: ci.url,
      repository: repoRef(ci.repo),
      external_id: String(ci.id),
      pull_request: ci.pullRequest,
      started_at: ci.startedAt,
      ...(ci.status === 'completed'
        ? { conclusion: ci.conclusion, completed_at: ci.updatedAt }
        : {}),
    },
  }) satisfies EnvelopeOf<'st3.resource:ci.run@1'>

/** Every world PR as an envelope, in world order. */
export const pullRequestEnvelopes = pullRequests.map(pullRequestEnvelopeFor)
/** Every world CI run as an envelope, in world order. */
export const ciRunEnvelopes = ciRuns.map(ciRunEnvelopeFor)

