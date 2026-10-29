import type { Attention, Capabilities, Device, Launch, LaunchVariant, Machine, Message, Mission, Session, Work, WorkState } from '@fractal/client'

import type { CastAgent, CastHost, CastMission, CastPerson } from './cast.ts'
import type { FactoryContext } from './context.ts'
import { scenarioId } from './context.ts'

/** Resource builders the slices share; values are client-v0 wire values. */

export const machine = (ctx: FactoryContext, host: CastHost, runtimeIds: string[], work: string[]): Machine => ({
  id: host.machine,
  kind: 'machine',
  revision: `mh-${host.name}`,
  updated_at: ctx.t.at(-30_000),
  host_id: host.id,
  name: host.name,
  state: ctx.cast.hosts[0] === host ? 'local' : 'reachable',
  fleet_id: `fleet/example-${ctx.cast.project}`,
  capacity: { state: 'unknown', reason: 'no capacity observation' },
  occupancy: { running_runtimes: runtimeIds.length },
  projects: [ctx.cast.repository],
  work,
  transports: [{ protocol: ctx.cast.hosts[0] === host ? 'unix' : 'fabric', status: ctx.cast.hosts[0] === host ? 'local' : 'up', last_success_at: ctx.t.at(-30_000) }],
  runtime_ids: runtimeIds,
})

export const session = (ctx: FactoryContext, member: CastAgent, state: Session['state'], startedMs: number): Session => ({
  id: member.session,
  kind: 'session',
  revision: 's-1',
  updated_at: ctx.t.at(-4_000),
  owner_id: member.id,
  state,
  started_at: ctx.t.at(startedMs),
  runtime_incarnation: member.incarnation,
  timeline_cursor: `timeline/${member.session}/0`,
})

export const device = (ctx: FactoryContext, person: CastPerson, name: string): Device => ({
  id: scenarioId(ctx, 'device', person.id),
  kind: 'device',
  revision: 'd-1',
  updated_at: ctx.t.at(-60_000),
  person_id: person.id,
  grant_kind: 'person',
  name,
  session_actor: `${person.id}/session/scenario`,
  state: 'active',
  scopes: ['read'],
  expires_at: ctx.t.at(24 * 60 * 60_000),
})

export const launch = (ctx: FactoryContext, value: CastMission, planner: CastAgent, variants: string[]): Launch => ({
  id: scenarioId(ctx, 'launch', 'api-rename'),
  kind: 'launch',
  revision: 'l-1',
  updated_at: ctx.t.at(-12 * 60_000),
  title: value.title,
  phase: 'review',
  request: 'Rename fetchUser to loadUser while preserving session propagation.',
  planner: planner.id,
  planner_config: { provider: 'codex' },
  target: { type: 'mission-run', mission_id: value.id, mission_run_id: value.run, generation_id: value.generation },
  variants,
  decisions: [],
  approvals: [],
})

export const launchVariant = (ctx: FactoryContext, value: CastMission, launchId: string, ordinal: number): LaunchVariant => ({
  id: scenarioId(ctx, 'launch-variant', `api-rename-${ordinal}`),
  kind: 'launch-variant',
  revision: 'lv-1',
  updated_at: ctx.t.at(-12 * 60_000),
  launch_id: launchId,
  ordinal,
  candidate_revision: 1,
  status: ordinal === 0 ? 'previewable' : 'rejected',
  target_generation: value.generation,
  normalized_mission: {
    title: value.title,
    goal: ordinal === 0 ? 'Rename the API and preserve every session argument' : 'Compare a staged compatibility rename',
    steps: value.steps.map((step) => ({ path: step.path, title: step.title })),
  },
  diagnostics: [],
})

export const mission = (ctx: FactoryContext, value: CastMission, state: Mission['state'], updatedMs: number): Mission => ({
  id: value.id,
  kind: 'mission',
  revision: `mr-${value.steps.length}`,
  updated_at: ctx.t.at(updatedMs),
  title: value.title,
  state,
  mission_revision: `sha256:${ctx.cast.project}-mission-1`,
  runs: [value.run],
  run_generations: { [value.run]: value.generation },
  active_runs: state === 'running' ? 1 : 0,
  total_runs: 1,
})

export interface WorkInput {
  readonly mission: CastMission
  readonly step: number
  readonly state: WorkState
  readonly updatedMs: number
  readonly claimant?: CastAgent
  readonly blockedReason?: string
  readonly goals: string[]
}

export const work = (ctx: FactoryContext, input: WorkInput): Work => {
  const step = input.mission.steps[input.step]!
  return {
    id: step.work,
    kind: 'work',
    revision: `w-${input.step + 1}`,
    updated_at: ctx.t.at(input.updatedMs),
    mission_id: input.mission.id,
    mission_run_id: input.mission.run,
    generation_id: input.mission.generation,
    definition_id: `def-${step.path}`,
    path: step.path,
    title: step.title,
    state: input.state,
    attempt: 1,
    readiness_epoch: input.step + 1,
    claimant: input.claimant?.id ?? null,
    claim_incarnation: input.claimant?.incarnation ?? null,
    blocked_reason: input.blockedReason ?? null,
    blockers: [],
    goals: input.goals,
    constraints: ['Keep the public API stable until the docs land'],
  }
}

export interface AttentionInput {
  readonly key: string
  readonly kind: 'agent-request' | 'unread-message' | 'human-gate' | 'fault'
  readonly person: CastPerson
  readonly source: string
  readonly title: string
  readonly detail: string
  readonly priority: Attention['priority']
  readonly requestedMs: number
  readonly actions: Attention['actions']
  readonly mission?: CastMission
  readonly requester?: CastAgent
}

export const attention = (ctx: FactoryContext, input: AttentionInput): Attention => ({
  id: scenarioId(ctx, 'attention', input.key),
  kind: 'attention',
  revision: 'a1',
  updated_at: ctx.t.at(input.requestedMs),
  attention_kind: input.kind,
  source_id: input.source,
  person_id: input.person.id,
  ...(input.requester === undefined ? {} : { requester_id: input.requester.id }),
  ...(input.mission === undefined ? {} : { mission_id: input.mission.id, mission_run_id: input.mission.run }),
  title: input.title,
  detail: input.detail,
  priority: input.priority,
  state: 'open',
  requested_at: ctx.t.at(input.requestedMs),
  targets: [input.source],
  actions: input.actions,
})

export interface MessageInput {
  readonly key: string
  readonly from: string
  readonly to: string
  readonly title: string
  readonly content: string
  readonly sentMs: number
  readonly state?: Message['state']
}

export const message = (ctx: FactoryContext, input: MessageInput): Message => ({
  id: scenarioId(ctx, 'message', input.key),
  kind: 'message',
  revision: 'm1',
  updated_at: ctx.t.at(input.sentMs),
  from: input.from,
  to: input.to,
  title: input.title,
  content: input.content,
  state: input.state ?? 'delivered',
  sent_at: ctx.t.at(input.sentMs),
  in_reply_to: null,
  tags: [],
})

/** Capabilities value of a healthy daemon; `collections` v1 raises the subscription cap to 16. */
export const capabilities = (ctx: FactoryContext, options: { readonly collectionsV1?: boolean; readonly omit?: readonly string[] } = {}): Capabilities => {
  const rows = [
    { id: 'read.projections', version: 0, state: 'granted' as const },
    { id: 'terminal.read', version: 0, state: 'granted' as const },
    { id: 'terminal.attach', version: 0, state: 'granted' as const },
    { id: 'control.messages', version: 0, state: 'granted' as const },
    { id: 'read.arrangements', version: 0, state: 'granted' as const },
    { id: 'arrangements', version: 1, state: 'granted' as const },
    ...(options.collectionsV1 === false ? [] : [{ id: 'collections', version: 1, state: 'granted' as const }]),
  ]
  return {
    kind: 'capabilities',
    session_actor: `${ctx.cast.people[0]?.id ?? 'person/ada'}/session/device-1`,
    transport: 'fabric-loopback',
    capabilities: rows.filter((row) => !(options.omit ?? []).includes(row.id)),
    limits: { max_page_items: 200, max_event_items: 500, max_response_bytes: 8_388_608, max_wait_ms: 30_000 },
    event_cursor: `event-cursor/scenario-${ctx.world}/1`,
    oldest_event_cursor: `event-cursor/scenario-${ctx.world}/0`,
    schemas: ['../schemas/client-v0.schema.json', '../schemas/operations.json'],
  }
}
