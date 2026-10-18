import { describe, expect, it } from 'vitest'
import { sidebarRow } from './sidebarRow.ts'
import type { Agent } from '../data/source.ts'
const agent: Agent = { ref:'agent/fixture/readonly', terminal:'', name:'Reported name', lifecycle: {_tag:'Unknown'}, host:'build-1', connected:true, activity:'idle', status:'idle', usage: {_tag:'Unknown'}, checkout: {_tag:'Unknown'}, workspace: {_tag:'Unknown'}, startedAt: {_tag:'Unknown'}, endedAt: {_tag:'Unknown'}, blockedOn: {_tag:'Unknown'}, ask: {_tag:'Unknown'}, lastActivityAt: {_tag:'Unknown'} }
const row = (change: Partial<Agent> = {}, stale = false) => sidebarRow({agent:{...agent,...change},stale})
describe('controlled native sidebar observations', () => {
  it('never exposes an unknown raw status in hover copy', () => {
    for (const status of ['indeterminate', 'unexpected-runtime-value']) {
      expect(row({ status }).statusLabel).toBe('Not observed')
      expect(row({ status }, true).statusLabel).toBe('Last verified · Not observed')
      expect(row({ status, connected: false }).statusLabel).toBe('Offline · Not observed')
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
