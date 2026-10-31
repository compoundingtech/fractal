import { Agent, AgentId, Revision, RuntimeId, decodeUnknownSync, type AgentEncoded } from '@fractal/client/schema'
import { decodeResourceRow, openAgent, type DecodedAgent } from '@fractal/sdk/effect'
import { expect, test, vi } from 'vitest'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { SidebarAgentRowData } from '@fractal/ui/assistant-ui'

import { fixtureAttention, fixtureMissions } from '../missions/fixtures.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import { AgentRowDetails, sidebarRowAccessibleName, sidebarRowDescription } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarAgentRow.tsx'
import { createProjections, fleetFromAgents, terminalSubjectForAgent } from './projections.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

/** The live roster path: the SDK keeps a row with open leaves or reports it undecodable. */
const decodeRosterResource = (raw: unknown) => {
  const { value } = decodeResourceRow(raw)
  if (value === undefined) throw new TypeError('Expected a decodable roster row')
  return value
}

test.each([
  { state: 'running', harness_state: 'idle', blocked_on: null, fault: null, activity: 'idle' },
  {
    state: 'running',
    harness_state: 'idle',
    blocked_on: 'human',
    fault: null,
    activity: 'waiting',
  },
  {
    state: 'failed',
    harness_state: 'idle',
    blocked_on: null,
    fault: 'runtime exited',
    activity: 'errored',
  },
  {
    state: 'down',
    harness_state: 'indeterminate',
    blocked_on: null,
    fault: null,
    activity: 'errored',
  },
] as const)(
  'observed harness activity respects human blocking and failure ($activity)',
  ({ activity, ...observation }) => {
    const row = openAgent(decodeUnknownSync(
      Agent,
      'strict',
    )({
      kind: 'agent',
      id: 'agent/folder-row',
      name: 'Folder row',
      host_id: 'host/example',
      runtime_ids: [],
      reachability: 'reachable',
      revision: '1',
      updated_at: '2026-10-04T12:00:00.000Z',
      ...observation,
    } satisfies AgentEncoded))
    expect(fleetFromAgents([row]).agents[0]?.activity).toBe(activity)
  },
)

const row = openAgent(decodeUnknownSync(
  Agent,
  'strict',
)({
  kind: 'agent',
  id: 'agent/first',
  name: 'First',
  host_id: 'host/example',
  runtime_ids: [],
  reachability: 'reachable',
  state: 'running',
  harness_state: 'idle',
  blocked_on: null,
  fault: null,
  revision: '1',
  updated_at: '2026-10-04T12:00:00.000Z',
} satisfies AgentEncoded))

test('projects authoritative native subagents, distinguishes absence and retains equivalent row identity', () => {
  const decode = decodeUnknownSync(Agent)
  const encoded = {
    kind: 'agent', id: 'agent/example-parent', name: 'Parent', host_id: 'host/example',
    runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'working',
    blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-18T12:00:00Z',
    subagents: [{
      id: 'reviewer', driver: 'omp', description: 'Review changes', session_id: 'session/review',
      started_at: '2026-10-18T12:00:00Z', lease_expires_at: '2026-10-18T12:01:00Z',
    }],
  } satisfies AgentEncoded
  const projections = createProjections()
  const first = projections.fleetFromAgents([openAgent(decode(encoded))]).agents[0]!
  expect(first.subagents?.[0]).toMatchObject({
    id: 'reviewer', driver: 'omp', description: 'Review changes', session_id: 'session/review',
    started_at: '2026-10-18T12:00:00.000Z', lease_expires_at: '2026-10-18T12:01:00.000Z',
  })
  expect(first.subagents?.[0]).not.toHaveProperty('activity')
  expect(first.subagents?.[0]).not.toHaveProperty('usage')
  expect(projections.fleetFromAgents([openAgent(decode(encoded))]).agents[0]).toBe(first)
  expect(projections.fleetFromAgents([openAgent(decode({ ...encoded, subagents: [] }))]).agents[0]).not.toBe(first)
  expect(fleetFromAgents([row]).agents[0]?.subagents).toBeUndefined()
})

const subagentWire = {
  kind: 'agent', id: 'agent/subagent-report', name: 'Report', host_id: 'host/example',
  runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle',
  revision: '1', updated_at: '2026-10-18T12:00:00Z',
} satisfies AgentEncoded

const decodeSubagentRoster = (wire: AgentEncoded) => {
  const decoded = decodeRosterResource(wire)
  if (decoded.kind !== 'agent') throw new TypeError('Expected an agent roster row')
  return decoded
}

const subagentDetailsText = (result: SidebarAgentRowData) =>
  renderToStaticMarkup(createElement(AgentRowDetails, { row: result }))
    .match(/>Subagents<\/dt><dd[^>]*>([^<]*)/)?.[1]

for (const [path, decode] of [
  ['decodeResourceRow', decodeSubagentRoster],
  ['openAgent', (wire: AgentEncoded) => openAgent(decodeUnknownSync(Agent, 'strict')(wire))],
] as const) {
  test.each([
    { name: 'opencode', driver: 'opencode', fact: { _tag: 'NonObserving', name: 'opencode' }, harness: undefined, cause: 'Harness' },
    { name: 'pi', driver: 'pi', fact: { _tag: 'NonObserving', name: 'pi' }, harness: undefined, cause: 'Harness' },
    { name: 'claude', driver: 'claude', fact: { _tag: 'Observing', name: 'claude' }, harness: 'claude', cause: 'DaemonVersion' },
    { name: 'unknown exec', driver: 'exec', fact: { _tag: 'Unknown', name: 'exec' }, harness: undefined, cause: 'Harness' },
    { name: 'absent driver', driver: undefined, fact: { _tag: 'Unknown' }, harness: undefined, cause: 'Harness' },
  ] as const)(`${path} preserves $name capability through visible missing-report copy`, ({ driver, fact, harness, cause }) => {
    const projected = fleetFromAgents([decode({
      ...subagentWire, ...(driver === undefined ? {} : { driver }),
    })]).agents[0]!
    expect(projected.driver).toEqual(fact)
    expect(projected.harness).toBe(harness)
    expect(projected.subagents).toBeUndefined()
    const result = sidebarRow({ agent: projected, stale: false })
    const text = cause === 'Harness' ? 'Not reported by this harness' : 'Not reported by this daemon version'
    expect(result.subagentObservation).toEqual({ _tag: 'NotReported', reason: { _tag: cause } })
    expect(result.subagentCount).toBeUndefined()
    expect(sidebarRowAccessibleName(result)).toBe('Report, Idle')
    expect(subagentDetailsText(result)).toBe(text)
    expect(sidebarRowDescription(result)).toBe(`Report. Host: example. Status: Idle; live observation. ${harness === undefined ? '' : `Harness: ${harness}. `}Terminal: terminal/subagent-report. ${text}. ID: agent/subagent-report.`)
  })

  test.each(['desired', 'starting'] as const)(`${path} keeps observing %s missing reports pending`, state => {
    const projected = fleetFromAgents([decode({ ...subagentWire, driver: 'claude', state })]).agents[0]!
    expect(projected.driver).toEqual({ _tag: 'Observing', name: 'claude' })
    const result = sidebarRow({ agent: projected, stale: false })
    expect(result.subagentObservation).toEqual({ _tag: 'Unavailable' })
    expect(result.subagentCount).toBeUndefined()
    expect(sidebarRowAccessibleName(result)).toBe('Report, ? subagents, Pending')
    expect(subagentDetailsText(result)).toBe('Not reported yet')
    expect(sidebarRowDescription(result)).toBe('Report. Host: example. Status: Pending; live observation. Harness: claude. Terminal: terminal/subagent-report. Not reported yet. ID: agent/subagent-report.')
  })

  test.each(['desired', 'starting', 'running'] as const)(`${path} preserves explicit zero while %s, including stale retention`, state => {
    const project = createProjections()
    const wire = { ...subagentWire, driver: 'claude', state, subagents: [] } satisfies AgentEncoded
    const projected = project.fleetFromAgents([decode(wire)]).agents[0]!
    expect(projected.driver).toEqual({ _tag: 'Observing', name: 'claude' })
    expect(project.fleetFromAgents([decode({ ...wire, revision: '2' })]).agents[0]).toBe(projected)
    for (const stale of [false, true]) {
      const result = sidebarRow({ agent: projected, stale })
      const status = `${stale ? 'Last verified · ' : ''}${state === 'running' ? 'Idle' : 'Pending'}`
      expect(result.subagentObservation).toEqual({ _tag: 'Reported' })
      expect(result.subagentCount).toBe(0)
      expect(sidebarRowAccessibleName(result)).toBe(`Report, ${status}`)
      expect(subagentDetailsText(result)).toBe('0 collapsed children')
      expect(sidebarRowDescription(result)).toBe(`Report. Host: example. Status: ${status}; ${stale ? 'stale' : 'live'} observation. Harness: claude. Terminal: terminal/subagent-report. Subagents: 0. ID: agent/subagent-report.`)
    }
  })

  test(`${path} retains a stale reported child without inventing nested reporting`, () => {
    const projected = fleetFromAgents([decode({
      ...subagentWire, driver: 'omp', subagents: [{
        id: 'reviewer', driver: 'omp', description: 'Review',
        lease_expires_at: '2026-10-18T12:01:00Z',
      }],
    })]).agents[0]!
    expect(projected.driver).toEqual({ _tag: 'Observing', name: 'omp' })
    const result = sidebarRow({ agent: projected, stale: true })
    expect(result.subagentObservation).toEqual({ _tag: 'Reported' })
    expect(result.subagentCount).toBe(1)
    expect(subagentDetailsText(result)).toBe('1 collapsed children')
    expect(sidebarRowAccessibleName(result)).toBe('Report, 1 subagent, Last verified · Idle')
    expect(sidebarRowDescription(result)).toBe('Report. Host: example. Status: Last verified · Idle; stale observation. Harness: omp. Terminal: terminal/subagent-report. Subagents: 1. ID: agent/subagent-report.')
    const child = result.children[0]!
    expect(child.freshness).toBe('stale')
    expect(child.subagentObservation).toEqual({ _tag: 'NotReported', reason: { _tag: 'Harness' } })
    expect(subagentDetailsText(child)).toBe('Not reported by this harness')
    expect(sidebarRowDescription(child)).toBe(`Review. Host: example. Status: Not observed; stale observation. Harness: omp. Not reported by this harness. ID: ${child.ref}.`)
  })

  test(`${path} invalidates retained rows only when driver facts change`, () => {
    const project = createProjections()
    const opencode = project.fleetFromAgents([decode({ ...subagentWire, driver: 'opencode' })])
    expect(project.fleetFromAgents([decode({ ...subagentWire, driver: 'opencode', revision: '2' })])).toBe(opencode)
    const pi = project.fleetFromAgents([decode({ ...subagentWire, driver: 'pi' })])
    expect(pi.agents[0]).not.toBe(opencode.agents[0])
    expect(pi.agents[0]?.driver).toEqual({ _tag: 'NonObserving', name: 'pi' })
    expect(project.fleetFromAgents([decode({ ...subagentWire, driver: 'pi' })])).toBe(pi)
    const unknown = project.fleetFromAgents([decode({ ...subagentWire, driver: 'exec' })])
    expect(unknown.agents[0]?.driver).toEqual({ _tag: 'Unknown', name: 'exec' })
    expect(project.fleetFromAgents([decode({ ...subagentWire, driver: 'exec' })])).toBe(unknown)
    const absent = project.fleetFromAgents([decode(subagentWire)])
    expect(absent.agents[0]).not.toBe(unknown.agents[0])
    expect(absent.agents[0]?.driver).toEqual({ _tag: 'Unknown' })
    expect(project.fleetFromAgents([decode(subagentWire)])).toBe(absent)
  })
}

test.each([
  ['agent/example', 'terminal/example'],
  ['agent/example-seat', 'terminal/example-seat'],
  ['agent/team/seat', 'terminal/team/seat'],
])('projects native agent identity %s through the shared terminal subject contract', (id, terminal) => {
  const nativeId = decodeUnknownSync(AgentId)(id)
  expect(terminalSubjectForAgent(nativeId)).toBe(terminal)
  expect(fleetFromAgents([{ ...row, id: nativeId }]).agents[0]?.terminal).toBe(terminalSubjectForAgent(nativeId))
})

test.each(['example-seat', 'team/seat', 'terminal/example', '', 'agent/', 'agent/example seat'])(
  'does not invent a terminal subject for the rejected native identity %s',
  id => {
    expect(() => decodeUnknownSync(AgentId)(id)).toThrow()
    expect(terminalSubjectForAgent(id)).toBeUndefined()
  },
)

test('terminal failure contract: projects authoritative empty runtime ownership as disabled', () => {
  const project = createProjections()
  const empty = project.fleetFromAgents([row])
  expect(empty.agents[0]).toHaveProperty('terminalDisabledReason', 'This agent has no terminal.')
  const running = { ...row, runtime_ids: [decodeUnknownSync(RuntimeId)('runtime/example')] }
  const enabled = project.fleetFromAgents([running])
  expect(enabled).not.toBe(empty)
  expect(enabled.agents[0]).not.toHaveProperty('terminalDisabledReason')
  expect(project.fleetFromAgents([row]).agents[0]).toHaveProperty('terminalDisabledReason', 'This agent has no terminal.')
})

test.each([
  { name: 'Declared standing', declaration: { lifecycle: 'standing' }, expected: 'Standing' },
  { name: 'Declared owner', declaration: { lifecycle: 'owner' }, expected: 'Owner' },
  { name: 'Declared bounded', declaration: { lifecycle: 'bounded' }, expected: 'Bounded' },
  { name: 'Absent declaration', declaration: {}, expected: 'Unknown' },
  { name: 'Standing maintenance agent', declaration: {}, expected: 'Unknown' },
] as const)('roster lifecycle decodes $name as $expected', ({ name, declaration, expected }) => {
  const decoded = decodeRosterResource({
    kind: 'agent',
    id: 'agent/lifecycle',
    name,
    runtime_ids: [],
    reachability: 'reachable',
    state: 'running',
    revision: '1',
    updated_at: '2026-10-04T12:00:00.000Z',
    ...declaration,
  } satisfies AgentEncoded)
  if (decoded.kind !== 'agent') throw new TypeError('Expected an agent roster row')
  expect(fleetFromAgents([decoded]).agents[0]).toHaveProperty('lifecycle', { _tag: expected })
})

test('roster lifecycle future-kind stays Unknown without dropping the row', () => {
  const decoded = decodeRosterResource({
    kind: 'agent', id: 'agent/lifecycle', name: 'Future declaration', runtime_ids: [],
    reachability: 'reachable', state: 'running', revision: '1',
    updated_at: '2026-10-04T12:00:00.000Z', lifecycle: 'future-kind',
  })
  if (decoded.kind !== 'agent') throw new TypeError('Expected an agent roster row')
  const fleet = fleetFromAgents([decoded])
  expect(fleet.agents).toHaveLength(1)
  expect(fleet.agents[0]).toMatchObject({ ref: 'agent/lifecycle', lifecycle: { _tag: 'Unknown' } })
})

test('roster lifecycle tolerance still enforces other field schemas', () => {
  expect(() => decodeRosterResource({
    kind: 'agent', id: 'agent/lifecycle', name: 'Future declaration', runtime_ids: [],
    reachability: 'reachable', state: 'running', revision: '1',
    updated_at: 'not-a-timestamp', lifecycle: 'future-kind',
  })).toThrow()
})

test('roster lifecycle tolerance preserves forward-compatible fields and enum cases', () => {
  const decoded = decodeRosterResource({
    kind: 'agent', id: 'agent/lifecycle', name: 'Future declaration', runtime_ids: [],
    reachability: 'reachable', state: 'future-state', revision: '1',
    updated_at: '2026-10-04T12:00:00.000Z', lifecycle: 'future-kind',
    future_field: { detail: 'Additional server evidence' },
  })
  if (decoded.kind !== 'agent') throw new TypeError('Expected an agent roster row')
  expect(decoded.state).toEqual({ _tag: 'Unrecognized', raw: 'future-state' })
  expect(fleetFromAgents([decoded]).agents[0])
    .toMatchObject({ ref: 'agent/lifecycle', lifecycle: { _tag: 'Unknown' } })
})

test('incremental roster lifecycle changes invalidate the retained sidebar row', () => {
  const project = createProjections()
  const first = project.fleetFromAgents([row])
  const standing = { ...row, lifecycle: 'standing' as const }
  const changed = project.fleetFromAgents([standing])
  expect(changed).not.toBe(first)
  expect(changed.agents[0]).toHaveProperty('lifecycle', { _tag: 'Standing' })
  expect(project.fleetFromAgents([{ ...standing }])).toBe(changed)
  expect(project.fleetFromAgents([{ ...row, lifecycle: 'owner' }]).agents[0])
    .toHaveProperty('lifecycle', { _tag: 'Owner' })
  expect(project.fleetFromAgents([row]).agents[0])
    .toHaveProperty('lifecycle', { _tag: 'Unknown' })
})

test('incremental fleet projection retains unchanged rows, hosts and collection identity', () => {
  const project = createProjections()
  const second: DecodedAgent = { ...row, id: 'agent/second', name: 'Second' }
  const first = project.fleetFromAgents([row, second])
  expect(project.fleetFromAgents([row, second])).toBe(first)
  expect(
    project.fleetFromAgents([
      { ...row, revision: decodeUnknownSync(Revision)('2') },
      { ...second, revision: decodeUnknownSync(Revision)('2') },
    ]),
  ).toBe(first)
  const changed = project.fleetFromAgents([row, { ...second, name: 'Renamed' }])
  expect(changed).not.toBe(first)
  expect(changed.agents[0]).toBe(first.agents[0])
  expect(changed.agents[1]).not.toBe(first.agents[1])
  expect(changed.hosts).toBe(first.hosts)
  const reordered = project.fleetFromAgents([second, row])
  expect(reordered.agents[1]).toBe(first.agents[0])
  expect(reordered.agents[0]).toBe(first.agents[1])
  expect(project.fleetFromAgents([]).agents).toEqual([])
})

test('mission joins retain unrelated mission views and update only linked attention', () => {
  const project = createProjections()
  const mission = fixtureMissions[0]!
  const second: typeof mission = { ...mission, id: 'mission/second' }
  const card = { ...fixtureAttention[0]!, mission_id: mission.id, state: 'open' as const }
  const first = project.missionViews({ missions: [mission, second], attention: [card] })
  expect(project.missionViews({ missions: [mission, second], attention: [card] })).toBe(first)
  const changed = project.missionViews({
    missions: [mission, second],
    attention: [{ ...card, title: 'Changed request' }],
  })
  expect(changed[0]).not.toBe(first[0])
  expect(changed[1]).toBe(first[1])
  const closed = project.missionViews({ missions: [mission, second], attention: [] })
  expect(closed[0]?.attention).toEqual([])
  expect(closed[1]).toBe(first[1])
})

test('subject identities follow only their row and attention membership', () => {
  const project = createProjections()
  const second: DecodedAgent = { ...row, id: 'agent/second', name: 'Second' }
  const fleet = project.fleetFromAgents([row, second])
  const first = project.subjectList({ fleet, attention: [] })
  expect(project.subjectList({ fleet, attention: [] })).toBe(first)
  const changed = project.subjectList({
    fleet: project.fleetFromAgents([row, { ...second, name: 'Renamed' }]),
    attention: [],
  })
  expect(changed[0]).toBe(first[0])
  expect(changed[1]).toBe(first[1])
  expect(changed[2]).not.toBe(first[2])
  const card = { ...fixtureAttention[0]!, source_id: row.id, state: 'open' as const, attention_kind: 'person-step', actions: ['work.done' as const] }
  const attentive = project.subjectList({ fleet, attention: [card] })
  expect(attentive[0]).toMatchObject({ attention: true })
  expect(attentive[1]).toBe(first[1])
  expect(attentive[2]).toBe(first[2])
  const cleared = project.subjectList({ fleet, attention: [] })
  expect(cleared[0]?.attention).toBeUndefined()
  expect(cleared[2]).toBe(first[2])
})

test('agent roster preserves usage scope, requested checkout, workspace and human attention facts', () => {
  const usage = {
    total_tokens: 123, input_tokens: 80, output_tokens: 43, cached_tokens: 20,
    cache_write_tokens: 5, cost: 0.12, currency: 'USD', incarnation_count: 2,
    aggregation: 'rollup-per-slot-else-cumulative-per-incarnation-else-response-deltas' as const,
  }
  const observed = openAgent(decodeUnknownSync(Agent, 'strict')({
    kind: 'agent', id: 'agent/facts', name: 'Facts', runtime_ids: [],
    reachability: 'reachable', state: 'running', revision: '1',
    updated_at: '2026-10-04T12:00:00.000Z',
    usage, checkout: { repository: 'owner/repo', base: 'main', branch: 'feature' },
    workspace: '/work/feature', blocked_on: 'human', ask: 'permission',
    last_activity_at: '2026-10-04T11:59:00.000Z',
    since: '2026-10-04T11:00:00.000Z',
  } satisfies AgentEncoded))
  expect(fleetFromAgents([observed]).agents[0]).toMatchObject({
    usage: { _tag: 'Known', value: usage },
    checkout: { _tag: 'Known', value: { repository: 'owner/repo', base: 'main', branch: 'feature' } },
    workspace: { _tag: 'Known', value: '/work/feature' },
    blockedOn: { _tag: 'Known', value: 'human' }, ask: { _tag: 'Known', value: 'permission' },
    lastActivityAt: { _tag: 'Known', value: Date.parse('2026-10-04T11:59:00.000Z') },
    startedAt: { _tag: 'Unknown' }, endedAt: { _tag: 'Unknown' },
  })
})

test('omitted Fractal roster facts remain Unknown, including session timestamps', () => {
  expect(fleetFromAgents([row]).agents[0]).toMatchObject({
    usage: { _tag: 'Unknown' }, checkout: { _tag: 'Unknown' }, workspace: { _tag: 'Unknown' },
    blockedOn: { _tag: 'Unknown' }, ask: { _tag: 'Unknown' },
    startedAt: { _tag: 'Unknown' }, endedAt: { _tag: 'Unknown' }, lastActivityAt: { _tag: 'Unknown' },
  })
})

test('usage without cost remains known token evidence without fabricated spend', () => {
  const observed = openAgent(decodeUnknownSync(Agent, 'strict')({
    kind: 'agent', id: 'agent/tokens', name: 'Tokens', runtime_ids: [],
    reachability: 'reachable', state: 'running', revision: '1', updated_at: '2026-10-04T12:00:00.000Z',
    usage: { total_tokens: 0, input_tokens: 0, output_tokens: 0, cached_tokens: 0,
      incarnation_count: 1, aggregation: 'rollup-per-slot-else-cumulative-per-incarnation-else-response-deltas' },
  } satisfies AgentEncoded))
  const projected = fleetFromAgents([observed]).agents[0]!
  expect(projected.usage._tag).toBe('Known')
  if (projected.usage._tag === 'Known') {
    expect(projected.usage.value.total_tokens).toBe(0)
    expect(projected.usage.value.cost).toBeUndefined()
    expect(projected.usage.value.currency).toBeUndefined()
  }
})

test('roster fact changes update the row while structurally equal usage and checkout retain identity', () => {
  const usage = { total_tokens: 10, input_tokens: 8, output_tokens: 2, cached_tokens: 0,
    incarnation_count: 1, aggregation: 'rollup-per-slot-else-cumulative-per-incarnation-else-response-deltas' as const }
  const wire = {
    kind: 'agent' as const, id: 'agent/facts', name: 'Facts', runtime_ids: [],
    reachability: 'reachable' as const, state: 'running' as const, revision: '1',
    updated_at: '2026-10-04T12:00:00.000Z',
    usage, checkout: { repository: 'owner/repo', base: 'main', branch: 'feature' },
  } satisfies AgentEncoded
  const project = createProjections()
  const decodeOpen = (encoded: AgentEncoded) => openAgent(decodeUnknownSync(Agent, 'strict')(encoded))
  const first = project.fleetFromAgents([decodeOpen(wire)])
  expect(project.fleetFromAgents([decodeOpen({ ...wire, revision: '2' })])).toBe(first)
  expect(project.fleetFromAgents([decodeOpen({
    ...wire, usage: { ...usage, total_tokens: 11 },
  })]).agents[0]).not.toBe(first.agents[0])
  expect(project.fleetFromAgents([decodeOpen({
    ...wire, checkout: { ...wire.checkout, branch: 'changed' },
  })]).agents[0]?.checkout).toEqual({ _tag: 'Known', value: { ...wire.checkout, branch: 'changed' } })
})

test('retains current and bounded upcoming work and invalidates every sidebar work field', () => {
  const work = {
    id: 'work/example/review', mission_id: 'mission/example/ship',
    mission_run_id: 'mission-run/example/ship', path: 'review', title: null,
    state: 'claimed' as const, since: '2026-10-20T12:00:00Z',
  }
  const wire = {
    kind: 'agent' as const, id: 'agent/example-worker', name: 'Worker', host_id: 'host/example-worker',
    runtime_ids: [], reachability: 'reachable' as const, state: 'running' as const,
    revision: '1', updated_at: '2026-10-20T12:00:00Z', current_work: [work],
    upcoming_work: [{ ...work, id: 'work/example/next', mission_id: 'mission/example/next' }],
  } satisfies AgentEncoded
  const project = createProjections()
  const decode = (encoded: AgentEncoded) => openAgent(decodeUnknownSync(Agent, 'strict')(encoded))
  const first = project.fleetFromAgents([decode(wire)])
  expect(first.agents[0]).toMatchObject({
    mission: work.mission_id,
    currentWork: [{ id: work.id, missionId: work.mission_id, runId: work.mission_run_id, path: 'review', state: 'claimed' }],
    upcomingWork: [{ missionId: 'mission/example/next' }],
  })
  expect(first.agents[0]?.currentWork?.[0]?.title).toBeUndefined()
  expect(project.fleetFromAgents([decode({ ...wire, revision: '2' })])).toBe(first)
  for (const change of [
    { title: 'Review changes' }, { path: 'approve' }, { state: 'waiting-person' as const },
    { mission_run_id: 'mission-run/example/new' }, { id: 'work/example/new' },
  ]) {
    expect(project.fleetFromAgents([decode({ ...wire, current_work: [{ ...work, ...change }] })]).agents[0])
      .not.toBe(first.agents[0])
  }
  expect(project.fleetFromAgents([decode({ ...wire, upcoming_work: [] })]).agents[0]?.upcomingWork).toEqual([])
  expect(project.fleetFromAgents([decode({ ...wire, current_work: [] })]).agents[0]?.mission).toBeUndefined()
})

test.each([
  [{}, 'Plant the beds'],
  [{ waiting_on: 'pull request acme/garden#12' }, 'Plant the beds · waiting on pull request acme/garden#12'],
  [{ stalled_at: '2026-10-04T11:00:00.000Z' }, 'Plant the beds · stalled: no progress and no declared wait'],
])('an agent holding a step says what it waits on or that it stalled (%o)', (hold, status) => {
  const held = openAgent(decodeUnknownSync(
    Agent,
    'strict',
  )({
    kind: 'agent',
    id: 'agent/gardener',
    name: 'Gardener',
    host_id: 'host/example',
    runtime_ids: [],
    reachability: 'reachable',
    state: 'running',
    harness_state: 'idle',
    blocked_on: null,
    fault: null,
    revision: '1',
    updated_at: '2026-10-04T12:00:00.000Z',
    current_work_ids: ['step-run/beds/plant'],
    current_work: [{
      id: 'step-run/beds/plant',
      mission_id: 'mission/garden',
      mission_run_id: 'mission-run/beds',
      path: 'plant',
      title: 'Plant the beds',
      state: 'claimed',
      since: '2026-10-04T08:00:00.000Z',
      ...hold,
    }],
  } satisfies AgentEncoded))
  expect(fleetFromAgents([held]).agents[0]?.status).toBe(status)
})
