import { describe, expect, it, vi } from 'vitest'
import { sidebarRow, sidebarMissionFeeds } from './sidebarRow.ts'
import { sidebarRowAccessibleName } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarAgentRow.tsx'
import type { Agent } from '../data/source.ts'
import { fixtureAttention, fixtureMissions } from '../missions/fixtures.ts'
import * as Option from 'effect/Option'
import type { Attention, MissionRunSummary } from '@fractal/client/schema'
import fc from 'fast-check'
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

describe('mission feed evidence', () => {
  it('keeps legacy live feeds live and leaves missing observation timestamps absent', () => {
    const live = { _tag: 'Observed', freshness: 'live', value: [] } as const
    expect(sidebarMissionFeeds({ missions: live, attention: live }).missionInfo).toEqual({ _tag: 'Live' })
    const stale = sidebarMissionFeeds({ missions: { ...live, freshness: 'stale' }, attention: live })
    expect(stale.missionInfo).toEqual({ _tag: 'Stale', updatedAt: undefined })
    expect(stale.missionUnavailable).toBe(false)
  })

  it('distinguishes retained feed failures from failures before a value exists', () => {
    const live = { _tag: 'Observed', freshness: 'live', value: [] } as const
    const failed = sidebarMissionFeeds({ missions: {
      ...live, freshness: 'stale', error: { reason: 'failed', detail: 'Follow failed' },
    }, attention: live })
    expect(failed.missionInfo).toEqual({ _tag: 'Failed', updatedAt: undefined })
    expect(failed.missionUnavailable).toBe(false)
    expect(sidebarMissionFeeds({ missions: {
      ...live, freshness: 'stale', error: { reason: 'failed', detail: 'Follow failed' },
    }, attention: { _tag: 'Waiting' } }).missionUnavailable).toBe(false)
    const missing = sidebarMissionFeeds({
      missions: { _tag: 'Unavailable', reason: 'failed', detail: 'No observation' }, attention: live,
    })
    expect(missing.missionInfo).toEqual({ _tag: 'Failed', updatedAt: undefined })
    expect(missing.missionUnavailable).toBe(true)
  })
})

const agent: Agent = { ref:'agent/example-readonly', terminal:'', name:'Reported name', lifecycle: {_tag:'Unknown'}, host:'build-1', connected:true, activity:'idle', status:'idle', usage: {_tag:'Unknown'}, checkout: {_tag:'Unknown'}, workspace: {_tag:'Unknown'}, startedAt: {_tag:'Unknown'}, endedAt: {_tag:'Unknown'}, blockedOn: {_tag:'Unknown'}, ask: {_tag:'Unknown'}, lastActivityAt: {_tag:'Unknown'} }
const row = (change: Partial<Agent> = {}, stale = false) => sidebarRow({agent:{...agent,...change},stale})
describe('controlled native sidebar observations', () => {
  it('never exposes an unknown raw status in hover copy', () => {
    for (const status of ['indeterminate', 'unexpected-runtime-value']) {
      expect(row({ status }).statusLabel).toBe('Idle')
      expect(row({ status }, true).statusLabel).toBe('Last verified · Idle')
      expect(row({ status }).status).toBe('idle')
      expect(row({ status, connected: false })).toMatchObject({ status: 'offline', statusLabel: 'Offline' })
    }
  })
  it('does not invent usage, duration, attention, unread or children', () => {
    const result=row()
    expect(result.usage).toEqual({_tag:'Unknown'})
    expect(result.duration).toEqual({_tag:'Unknown'})
    expect(result.lastTurn).toEqual({_tag:'Unknown'})
    expect(result.needsMe).toBeUndefined()
    expect(result.unread).toBeUndefined()
    expect(result.childrenKnown).toBe(false)
    expect(result.terminal).toBeUndefined()
  })
  it('keeps generic activity distinct from a completed turn', () => {
    expect(row({lastActivityAt:{_tag:'Known',value:9000}}).lastTurn).toEqual({_tag:'Known',kind:'activity',at:9000})
    expect(row({lastActivityAt:{_tag:'Known',value:9000}}).statusSince).toBeUndefined()
  })
  it('rejects nonfinite and non-date timestamps before render', () => {
    for(const at of [NaN,Infinity,-1,8640000000000001]) {
      expect(row({lastActivityAt:{_tag:'Known',value:at},statusSince:at}).lastTurn).toEqual({_tag:'Unknown'})
      expect(row({lastActivityAt:{_tag:'Known',value:at},statusSince:at}).statusSince).toBeUndefined()
    }
  })
  it('preserves structurally valid timestamps for the subscribed leaf to validate against its clock', () => {
    expect(row({lastActivityAt:{_tag:'Known',value:5500},statusSince:5500})).toMatchObject({lastActivityAt:5500,statusSince:5500})
  })
  it('retains distinct native lifecycle states and freshness', () => {
    for(const [state,status] of [['retired','retired'],['suspended','suspended'],['stopped','ended'],['starting','pending']] as const)
      expect(row({state}).status).toBe(status)
    expect(row({connected:false}).status).toBe('offline')
    expect(row({activity:'working'},true)).toMatchObject({status:'working',freshness:'stale'})
    expect(row({activity:'waiting'}).status).toBe('waiting')
    expect(row({activity:'waiting'}).needsMe).toBeUndefined()
  })
  it('uses the same canonical sentence-case state for the glyph and status word', () => {
    for (const state of ['stopped', 'ended']) {
      expect(row({ state, activity: 'idle', status: 'idle' })).toMatchObject({ status: 'ended', statusLabel: 'Ended' })
    }
    expect(row({ activity: 'working', status: 'working' })).toMatchObject({ status: 'working', statusLabel: 'Working' })
    expect(row()).toMatchObject({ status: 'idle', statusLabel: 'Idle' })
    expect(row({ activity: 'waiting', status: 'waiting' })).toMatchObject({ status: 'waiting', statusLabel: 'Needs you' })
    expect(row({ activity: 'waiting', status: 'waiting for you (question)' })).toMatchObject({ status: 'waiting', statusLabel: 'Needs you' })
    expect(row({ activity: 'errored', status: 'errored' })).toMatchObject({ status: 'stale', statusLabel: 'Stale observation' })
    expect(row({ state: 'stopped' }, true)).toMatchObject({ status: 'ended', statusLabel: 'Last verified · Ended' })
  })
  it('retains canonical identity and reported metadata without a new read', () => {
    expect(row({description:'Actual description',harness:'codex',terminal:'terminal/native',mission:'mission/native'})).toMatchObject({ref:agent.ref,id:agent.ref,title:agent.name,host:agent.host,description:'Actual description',harness:'codex',terminal:'terminal/native',mission:'mission/native'})
  })
  it('shows only authoritative spend and tokens and explicit attention', () => {
    const usage = { aggregation: 'cumulative-per-incarnation-else-response-deltas', cached_tokens: 0, incarnation_count: 1, input_tokens: 100, output_tokens: 20, total_tokens: 120 } as const
    expect(row({ usage: { _tag: 'Known', value: usage } }).usage).toEqual({ _tag: 'Known', scope: 'reported-agent', tokens: 120 })
    expect(row({ usage: { _tag: 'Known', value: { ...usage, currency: 'USD', cost: 0.25 } } }).usage).toEqual({ _tag: 'Known', scope: 'reported-agent', tokens: 120, usd: 0.25 })
    expect(row({ usage: { _tag: 'Known', value: { ...usage, currency: 'EUR', cost: 0.25 } } }).usage).not.toHaveProperty('usd')
    expect(sidebarRow({ agent, stale: false, needsMe: true }).needsMe).toBe(true)
  })
  it('projects native children without borrowing parent activity, time, usage or attention', () => {
    const result = row({ activity: 'working', subagents: [{
      id: 'reviewer', driver: 'omp', description: 'Review', session_id: 'session/review',
      started_at: '2026-10-18T12:00:00Z', lease_expires_at: '2026-10-18T12:01:00Z',
    }] })
    expect(result.childrenKnown).toBe(true)
    expect(result.children[0]).toMatchObject({ title: 'Review', parentRef: agent.ref, status: 'unobserved', statusLabel: 'Not observed', usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' } })
    expect(result.children[0]?.lastActivityAt).toBeUndefined()
    expect(result.children[0]?.statusSince).toBeUndefined()
    expect(result.children[0]?.needsMe).toBeUndefined()
    expect(result.children[0]?.unread).toBeUndefined()
    expect(result.children[0]?.terminal).toBeUndefined()
    expect(row({ subagents: [] }).childrenKnown).toBe(true)
  })
})

describe('retained mission row context', () => {
  const mission = fixtureMissions[0]!
  const work = { id: 'work/current', missionId: mission.id, runId: 'mission-run/example/run', path: 'review', state: 'claimed', title: 'Review changes' } as const
  const seat = { ...agent, mission: mission.id, currentWork: [work] }
  const run: MissionRunSummary = {
    id: work.runId, requester: 'agent/example-worker', status: 'running', phase: 'working',
    state_since: mission.updated_at, progress: { done: 2, total: 5 }, current_steps: [],
    after: Option.none(), blocker: Option.none(), deadline: Option.none(),
    last_progress: Option.none(), outcome: Option.none(), steps: Option.none(), must_act: 'agent',
  }
  const hydrated = { ...mission, run_details: [run], run_generations: {} }
  const project = (missions = [hydrated], attention: typeof fixtureAttention = []) =>
    sidebarRow({ agent: seat, stale: false, missions, attention }).missionContext

  it('joins exact run progress and preserves the observed step title', () => {
    expect(project()).toMatchObject({ _tag: 'Mission', id: mission.id, slug: mission.id.split('/').at(-1),
      progress: { _tag: 'Known', done: 2, total: 5 }, step: { _tag: 'Step', name: 'Review changes' } })
    expect(project([])).toMatchObject({ progress: { _tag: 'Unknown' } })
    expect(project([{ ...hydrated, run_details: [{ ...run, id: 'mission-run/example/other' }] }]))
      .toMatchObject({ progress: { _tag: 'Unknown' } })
  })

  it('does not borrow progress from a different generation', () => {
    expect(project([{ ...hydrated, run_generations: { [run.id]: 'run-generation/current' },
      run_details: [{ ...run, generation_id: 'run-generation/previous' }] }]))
      .toMatchObject({ progress: { _tag: 'Unknown' } })
  })

  it('uses only the slug and step path when titles are unavailable', () => {
    expect(sidebarRow({ agent: { ...seat, description: 'Not a mission title', currentWork: [{ ...work, title: undefined }] },
      stale: false }).missionContext).toMatchObject({
      slug: mission.id.split('/').at(-1), step: { _tag: 'Step', name: 'review' },
    })
    expect(project([{ ...hydrated, title: 'A title is not the mission slug' }])).toMatchObject({
      slug: mission.id.split('/').at(-1),
    })
    expect(row().missionContext).toEqual({ _tag: 'None' })
  })

  it('shows actual attention question without manufacturing a q handle and follows updates', () => {
    const card: Attention = { ...fixtureAttention[0]!, attention_kind: 'agent-request', state: 'open',
      actions: ['work.done'], conversation_id: 'agent/example-readonly', detail: 'Ship this change?', update: undefined }
    expect(project([hydrated], [card])).toMatchObject({ step: { _tag: 'Waiting', question: 'Ship this change?' } })
    const waiting = project([hydrated], [card])
    if (waiting?._tag === 'Mission') expect(waiting.step).not.toHaveProperty('handle')
    expect(project([hydrated], [{ ...card, detail: 'Use the updated plan?' }]))
      .toMatchObject({ step: { _tag: 'Waiting', question: 'Use the updated plan?' } })
    expect(project([hydrated], [{ ...card, state: 'resolved' }]))
      .toMatchObject({ step: { _tag: 'Step', name: work.title } })
    expect(project([{ ...hydrated, run_details: [{ ...run, progress: { done: 3, total: 5 } }] }]))
      .toMatchObject({ progress: { _tag: 'Known', done: 3, total: 5 } })
  })

  it('keeps concise accessible names aligned with real mission evidence', () => {
    const actual = sidebarRow({ agent: seat, stale: false, missions: [hydrated] })
    const prefix = `${seat.name}, Idle, Mission ${mission.id.split('/').at(-1)}, 2 of 5 steps`
    expect(sidebarRowAccessibleName(actual)).toBe(`${prefix}, Step Review changes`)
    const card: Attention = { ...fixtureAttention[0]!, attention_kind: 'agent-request', state: 'open',
      actions: ['work.done'], conversation_id: seat.ref, detail: 'Ship this change?', update: undefined }
    const waiting = sidebarRow({ agent: seat, stale: false, missions: [hydrated], attention: [card],
      missionInfo: { _tag: 'Stale', updatedAt: 1000 } })
    expect(sidebarRowAccessibleName(waiting)).toBe(`${prefix}, Waiting for you: Ship this change?, mission info may be out of date`)
    expect(sidebarRowAccessibleName({ ...actual, missionInfo: { _tag: 'Failed' } }))
      .toBe(`${prefix}, Step Review changes, mission info may be out of date`)
    expect(sidebarRowAccessibleName(row())).toBe('Reported name, Idle')
  })
  it('does not attach another agent question just because it shares the mission run', () => {
    const card: Attention = { ...fixtureAttention[0]!, attention_kind: 'agent-request', state: 'open',
      actions: ['work.done' as const], conversation_id: 'agent/example-another', conversation_ids: [],
      requester_id: 'agent/example-another', source_id: 'agent/example-another', step_run_id: undefined,
      blocked: undefined, mission_run_id: work.runId, detail: 'Someone else must decide', update: undefined }
    expect(project([hydrated], [card])).toMatchObject({ step: { _tag: 'Step', name: work.title } })
  })


  it('deduplicates only observed upcoming missions and excludes the active mission', () => {
    const other = { ...work, missionId: 'mission/example/next', runId: 'mission-run/example/next' }
    for (const repetitions of [0, 1, 2, 5]) {
      expect(sidebarRow({ agent: { ...seat, upcomingWork: [work, ...Array.from({ length: repetitions }, () => other)] },
        stale: false }).missionContext).toMatchObject({
        queued: repetitions === 0 ? [] : [{ id: other.missionId, slug: 'next' }],
      })
    }
  })
})

it('queued mission projection preserves first occurrence order for arbitrary bounded windows', () => {
  fc.assert(fc.property(fc.array(fc.constantFrom('current', 'alpha', 'beta')), names => {
    const currentWork = [{ id: 'work/example/current', missionId: 'mission/example/current',
      runId: 'mission-run/example/current', path: 'work', state: 'claimed' }]
    const upcomingWork = names.map(name => ({ ...currentWork[0]!, missionId: `mission/example/${name}` }))
    const context = sidebarRow({ agent: { ...agent, currentWork, upcomingWork }, stale: false }).missionContext
    expect(context?._tag).toBe('Mission')
    if (context?._tag !== 'Mission') return
    expect(context.queued.map(mission => mission.slug)).toEqual([...new Set(names.filter(name => name !== 'current'))])
  }))
})
