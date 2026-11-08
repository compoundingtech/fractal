import { describe, expect, it } from 'vitest'

import { fixtureAttention, fixtureNow, secretsRotation, webfractalFoundation } from '../fixtures.ts'
import { criticalPath, lineage, missionDetailModel, ranIn, span, spendOf, stateAt, waitedMs, type MissionDetailModel } from './detailModel.ts'
import { syncAttention, syncMission, syncMissionWithHistory, syncNow, syncRunRef, syncStart, syncStep, syncUsageRows, withServedHistory } from './fixtures.ts'

const MIN = 60_000
const HOUR = 60 * MIN
const at = (h: number, m = 0) => syncStart + h * HOUR + m * MIN
const model = (overrides: Partial<Parameters<typeof missionDetailModel>[0]> = {}): MissionDetailModel =>
  missionDetailModel({ mission: syncMission, agents: [], attention: syncAttention, usage: syncUsageRows, now: syncNow, ...overrides })
const step = (detail: MissionDetailModel, path: string) => {
  const found = detail.steps.find(each => each.path === path)
  if (found === undefined) throw new Error(`no step ${path}`)
  return found
}

describe('mission detail model: coarse history from served fields', () => {
  it('reads the selected attempt as the one known attempt, and says earlier ones are missing', () => {
    const server = step(model(), 'server')
    expect(server.history).toBe('coarse')
    expect(server.attemptCount).toBe(3)
    expect(server.attempts).toEqual([{ n: 3, agent: 'agent/build-host-a/workbench-shell', start: at(6, 20), end: at(8, 40), outcome: 'done', reason: undefined }])
    expect(server.historyComplete).toBe(false)
    expect(step(model(), 'spec').historyComplete).toBe(true)
  })
  it('keeps a running attempt open and a failed one closed', () => {
    const detail = model()
    expect(step(detail, 'web').state).toBe('running')
    expect(step(detail, 'web').attempts).toEqual([expect.objectContaining({ start: at(26), end: undefined, outcome: 'running' })])
    expect(step(detail, 'load').state).toBe('failed')
    expect(step(detail, 'load').attempts).toEqual([expect.objectContaining({ start: at(12), end: at(13, 40), outcome: 'failed' })])
  })
  it('reads the current wait: on you from an open ask on the step, on dependencies while waiting', () => {
    const detail = model()
    const security = step(detail, 'security')
    expect(security.state).toBe('waiting-you')
    expect(security.waits).toEqual([{ kind: 'you', start: at(25), end: undefined, reason: 'Approve widening sync tokens to read.projections?' }])
    // It ran from its start until it began to wait.
    expect(security.attempts).toEqual([expect.objectContaining({ start: at(22), end: at(25) })])
    expect(waitedMs(security, syncNow, 'you')).toBe(5 * HOUR)
    expect(step(detail, 'canary').waits).toEqual([expect.objectContaining({ kind: 'dependency', start: at(0), end: undefined })])
    expect(step(detail, 'canary').attempts).toEqual([])
  })
  it('treats any open ask on a step as waiting on the person, not only human gates', () => {
    const detail = missionDetailModel({ mission: webfractalFoundation, agents: [], attention: fixtureAttention.filter(item => item.state === 'open'), usage: undefined, now: fixtureNow })
    expect(detail.needsYou.length).toBe(1)
    expect(detail.steps.find(each => each.id === detail.needsYou[0])?.state).toBe('waiting-you')
  })
  it('keys dependencies by step id, in plan order, with removed steps last and out of progress', () => {
    const detail = model()
    expect(step(detail, 'load').after).toEqual([syncStep('server'), syncStep('migration')])
    expect(detail.steps[0]!.path).toBe('spec')
    expect(detail.steps.at(-1)!.state).toBe('cancelled')
    expect(detail.progress).toEqual({ done: 6, total: 12 })
  })
  it('names what runs, what needs the person, what failed and what comes next', () => {
    const detail = model()
    expect(detail.running).toEqual([syncStep('web')])
    expect(detail.needsYou).toEqual([syncStep('security')])
    expect(detail.failed).toEqual([syncStep('load')])
    expect(detail.next).toBe(syncStep('canary'))
    expect(detail.run).toEqual(expect.objectContaining({ id: syncRunRef, startedAt: syncStart, settled: false, eta: { lowMs: 6 * HOUR, highMs: 9 * HOUR, asOf: syncNow } }))
    expect(detail.startedAt).toBe(syncStart)
  })
  it('lists problems from what is served: the failure, the retries, the wait on you', () => {
    const kinds = model().problems.map(problem => `${problem.kind}:${problem.step.split('/').pop()}`)
    expect(kinds).toEqual(expect.arrayContaining(['failure:load', 'retry:server', 'wait-you:security']))
  })
  it('shows agents without a roster row by their slug, live state unknown', () => {
    const web = model().agents.find(agent => agent.id === 'agent/build-host-a/terminal-renderer')
    expect(web).toEqual({ id: 'agent/build-host-a/terminal-renderer', label: 'terminal-renderer', state: 'unknown', step: syncStep('web') })
  })
  it('shows the open run from a detail read that also carries a finished run', () => {
    const detail = model({ mission: syncMissionWithHistory })
    expect(detail.run?.id).toBe(syncRunRef)
    expect(detail.steps.every(each => each.id.startsWith('step-run/platform/sync-v2/2/'))).toBe(true)
  })
  it('shows a settled mission’s latest run', () => {
    const detail = missionDetailModel({ mission: secretsRotation, agents: [], attention: [], usage: undefined, now: fixtureNow })
    expect(detail.run?.settled).toBe(true)
    expect(detail.steps.length).toBeGreaterThan(0)
  })
})

describe('mission detail model: served per-step history preferred', () => {
  const detail = model({ mission: withServedHistory(syncMission) })
  it('reads every attempt with its outcome and reason', () => {
    const server = step(detail, 'server')
    expect(server.history).toBe('served')
    expect(server.historyComplete).toBe(true)
    expect(server.attempts.map(attempt => [attempt.n, attempt.outcome, attempt.start, attempt.end])).toEqual([[1, 'failed', at(2, 30), at(4, 50)], [2, 'failed', at(5, 10), at(6, 0)], [3, 'done', at(6, 20), at(8, 40)]])
    expect(server.attempts[0]!.reason).toBe('migration test timed out after 20m')
    expect(server.waits.map(wait => wait.kind)).toEqual(['dependency', 'retry', 'retry'])
  })
  it('reads served waits instead of deriving the current one', () => {
    const security = step(detail, 'security')
    expect(security.waits.map(wait => [wait.kind, wait.start, wait.end])).toEqual([['queue', at(8, 40), at(22)], ['you', at(25), undefined]])
  })
  it('reads the per-step expected duration', () => {
    expect(step(detail, 'canary').expected).toEqual({ lowMs: 90 * MIN, highMs: 150 * MIN })
    expect(step(model(), 'canary').expected).toBeUndefined()
  })
  it('lists each failed attempt as its own problem with its reason', () => {
    const retries = detail.problems.filter(problem => problem.step === syncStep('server'))
    expect(retries.map(problem => problem.text)).toEqual([expect.stringContaining('migration test timed out'), expect.stringContaining('flaky lock test')])
    expect(retries.every(problem => problem.resolvedAt === at(8, 40))).toBe(true)
  })
})

describe('mission detail model: spend', () => {
  it('sums usage rows by step', () => {
    expect(step(model(), 'server').spend).toEqual({ costUsd: 11.4, tokens: 910_000 })
    expect(spendOf(syncUsageRows, { step: syncStep('spec') })).toEqual({ costUsd: 0.84, tokens: 61_000 })
  })
  it('sums the run’s rows for the total, and leaves other missions out', () => {
    const total = model().spend!
    expect(total.costUsd).toBeCloseTo(29.26, 6)
    expect(total.tokens).toBe(2_194_000)
  })
  it('says unknown, never zero, without rows or without a read', () => {
    expect(step(model(), 'canary').spend).toBeUndefined()
    expect(model({ usage: undefined }).spend).toBeUndefined()
    expect(step(model({ usage: undefined }), 'server').spend).toBeUndefined()
    expect(model({ usage: [] }).spend).toBeUndefined()
  })
})

describe('mission detail model: replay and linking', () => {
  const coarse = model()
  const served = model({ mission: withServedHistory(syncMission) })
  it('replays a step’s state at a cursor', () => {
    expect(stateAt(step(coarse, 'server'), at(7), syncNow)).toBe('running')
    expect(stateAt(step(coarse, 'server'), at(9), syncNow)).toBe('done')
    expect(stateAt(step(coarse, 'server'), at(1), syncNow)).toBe('pending')
    expect(stateAt(step(served, 'server'), at(3), syncNow)).toBe('running')
    expect(stateAt(step(served, 'server'), at(5), syncNow)).toBe('waiting')
    expect(stateAt(step(coarse, 'security'), at(23), syncNow)).toBe('running')
    expect(stateAt(step(coarse, 'security'), at(26), syncNow)).toBe('waiting-you')
    expect(stateAt(step(coarse, 'load'), at(14), syncNow)).toBe('failed')
    expect(stateAt(step(coarse, 'manual-qa'), at(14), syncNow)).toBe('cancelled')
  })
  it('lights a step’s lineage, up and down', () => {
    const lit = lineage(coarse.steps, syncStep('security'))
    expect([...lit].map(id => id.split('/').pop()).sort()).toEqual(['announce', 'canary', 'rollout', 'schema', 'security', 'server', 'spec'])
  })
  it('knows which steps ran in a time range', () => {
    expect(ranIn(step(coarse, 'web'), at(26, 30), at(27), syncNow)).toBe(true)
    expect(ranIn(step(coarse, 'spec'), at(26, 30), at(27), syncNow)).toBe(false)
  })
  it('finds the critical path through the longest chain', () => {
    const path = criticalPath(coarse.steps, syncNow)
    expect(path.has(syncStep('web'))).toBe(true)
    expect(path.has(syncStep('announce'))).toBe(true)
    expect(path.has(syncStep('docs'))).toBe(false)
  })
})

describe('span', () => {
  it('names a duration compactly', () => {
    expect(span(20_000)).toBe('<1m')
    expect(span(4 * MIN)).toBe('4m')
    expect(span(72 * MIN)).toBe('1h 12m')
    expect(span(5 * HOUR)).toBe('5h')
    expect(span(51 * HOUR)).toBe('2d 3h')
  })
})
