import { describe, expect, it, vi } from 'vitest'
import { sidebarRow } from './sidebarRow.ts'
import { sidebarRowDescription } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarAgentRow.tsx'
import { AgentRowDetails } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/RowOverlays.tsx'
import { sidebarRowAccessibleName } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/rowPresentation.tsx'
import { SidebarStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarStatus.tsx'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { Agent } from '../data/source.ts'
import * as DateTime from 'effect/DateTime'
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

describe('agent status glyph vocabulary', () => {
  it.each([
    ['working', '●'], ['idle', '○'], ['waiting', '○'], ['ended', '×'], ['retired', '×'],
    ['failed', '!'], ['pending', '~'], ['suspended', '○'], ['unobserved', ''], ['stale', ''], ['offline', ''],
  ] as const)('renders %s as exactly %s', (status, glyph) => {
    const html = renderToStaticMarkup(createElement(SidebarStatus, { status, variant: 'Agents', freshness: 'live', iconOnly: true }))
    expect(html.match(/data-agent-glyph="true"[^>]*>([^<]*)<\/span>/)?.[1]).toBe(glyph)
  })
  it.each(['live', 'stale', 'unobserved'] as const)('retains the last working glyph for %s observation', freshness => {
    const html = renderToStaticMarkup(createElement(SidebarStatus, { status: 'working', variant: 'Agents', freshness, iconOnly: true }))
    expect(html).toContain('>●</span>')
    expect(html.includes('data-frozen="true"')).toBe(freshness !== 'live')
    expect(html).not.toContain('<svg')
  })
  it.each(['SG-1', 'SG-2', 'SG-3'] as const)('preserves the existing %s SVG presentation', variant => {
    const html = renderToStaticMarkup(createElement(SidebarStatus, { status: 'working', variant, freshness: 'live', iconOnly: true }))
    expect(html).toContain('<svg')
    expect(html).not.toContain('data-agent-glyph')
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
      expect(row({ status, connected: false })).toMatchObject({ status: 'idle', statusLabel: 'Last verified · Idle', freshness: 'stale' })
    }
  })
  it('does not invent usage, duration, attention, unread or children', () => {
    const result=row()
    expect(result.usage).toEqual({_tag:'Unknown'})
    expect(result.duration).toEqual({_tag:'Unknown'})
    expect(result.lastTurn).toEqual({_tag:'Unknown'})
    expect(result.needsMe).toBeUndefined()
    expect(result.unread).toBeUndefined()
    expect(result.subagentObservation).toEqual({ _tag: 'NotReported', reason: { _tag: 'Harness' } })
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
    expect(row({connected:false})).toMatchObject({ status: 'idle', freshness: 'stale', statusLabel: 'Last verified · Idle' })
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
    expect(row({ activity: 'errored', status: 'errored' })).toMatchObject({ status: 'failed', statusLabel: 'Failed' })
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
  it('distinguishes a structured harness question from other attention', () => {
    expect(row({ ask: { _tag: 'Known', value: 'question' } }).asking).toBe(true)
    for (const value of ['permission', 'review']) expect(row({ ask: { _tag: 'Known', value } }).asking).toBe(false)
    expect(row({ activity: 'waiting', status: 'waiting for you (question)' }).asking).toBeUndefined()
    expect(sidebarRow({ agent, stale: false, needsMe: true }).asking).toBeUndefined()
  })
  it('keeps faults distinct from observation gaps and pending lifecycle', () => {
    expect(row({ state: 'failed' })).toMatchObject({ status: 'failed', statusLabel: 'Failed' })
    expect(row({ activity: 'errored', connected: false })).toMatchObject({ status: 'failed', freshness: 'stale' })
    expect(row({ state: 'starting', activity: 'working' })).toMatchObject({ status: 'pending' })
    expect(row({ activity: 'working', connected: false })).toMatchObject({ status: 'working', freshness: 'stale' })
  })
  it('projects native children without borrowing parent activity, time, usage or attention', () => {
    const result = row({ activity: 'working', subagents: [{
      id: 'reviewer', driver: 'omp', description: 'Review', session_id: 'session/review',
      started_at: '2026-10-18T12:00:00Z', lease_expires_at: '2026-10-18T12:01:00Z',
    }] })
    expect(result.subagentObservation).toEqual({ _tag: 'Reported' })
    expect(result.subagentCount).toBe(1)
    expect(sidebarRowAccessibleName(result)).toBe('Reported name, 1 subagent, Working')
    expect(result.children[0]).toMatchObject({ title: 'Review', parentRef: agent.ref, status: 'unobserved', statusLabel: 'Not observed', usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' } })
    expect(result.children[0]?.lastActivityAt).toBeUndefined()
    expect(result.children[0]?.statusSince).toBeUndefined()
    expect(result.children[0]?.needsMe).toBeUndefined()
    expect(result.children[0]?.unread).toBeUndefined()
    expect(result.children[0]?.terminal).toBeUndefined()
    const child = result.children[0]!
    expect(child.subagentObservation).toEqual({ _tag: 'NotReported', reason: { _tag: 'Harness' } })
    expect(renderToStaticMarkup(createElement(AgentRowDetails, { row: child }))
      .match(/>Subagents<\/dt><dd[^>]*>([^<]*)/)?.[1]).toBe('Not reported by this harness')
    expect(sidebarRowDescription(child)).toBe(`Review. Host: build-1. Status: Not observed; live observation. Harness: omp. Not reported by this harness. ID: ${child.ref}.`)
    expect(row({ subagents: [] }).subagentObservation).toEqual({ _tag: 'Reported' })
    expect(row({ subagents: [] }).subagentCount).toBe(0)
    expect(sidebarRowAccessibleName(row({ subagents: [] }))).toBe('Reported name, Idle')
    expect(row().subagentCount).toBeUndefined()
    expect(sidebarRowAccessibleName(row())).toBe('Reported name, Idle')
  })
  it('distinguishes harness capability from omitted reports on older daemons', () => {
    for (const [driver, cause, text] of [
      [{ _tag: 'NonObserving', name: 'opencode' }, 'Harness', 'Not reported by this harness'],
      [{ _tag: 'NonObserving', name: 'pi' }, 'Harness', 'Not reported by this harness'],
      [{ _tag: 'Observing', name: 'claude' }, 'DaemonVersion', 'Not reported by this daemon version'],
      [{ _tag: 'Observing', name: 'codex' }, 'DaemonVersion', 'Not reported by this daemon version'],
      [{ _tag: 'Observing', name: 'omp' }, 'DaemonVersion', 'Not reported by this daemon version'],
      [{ _tag: 'Unknown', name: 'exec' }, 'Harness', 'Not reported by this harness'],
      [{ _tag: 'Unknown' }, 'Harness', 'Not reported by this harness'],
    ] as const) {
      const harness = driver._tag === 'Observing' ? driver.name : undefined
      const result = row({ driver, harness, state: 'running' })
      expect(result.subagentObservation).toEqual({ _tag: 'NotReported', reason: { _tag: cause } })
      expect(sidebarRowAccessibleName(result)).toBe('Reported name, Idle')
      const details = renderToStaticMarkup(createElement(AgentRowDetails, { row: result }))
      expect(details.match(/>Subagents<\/dt><dd[^>]*>([^<]*)/)?.[1]).toBe(text)
      expect(sidebarRowDescription(result)).toBe(`Reported name. Host: build-1. Status: Idle; live observation. ${harness === undefined ? '' : `Harness: ${harness}. `}${text}. ID: ${agent.ref}.`)
    }
  })
  it('treats explicit empty Claude, Codex and OMP rosters as reported zero', () => {
    for (const harness of ['claude', 'codex', 'omp'] as const) {
      const result = row({ harness, driver: { _tag: 'Observing', name: harness }, subagents: [] })
      expect(result.subagentObservation).toEqual({ _tag: 'Reported' })
      expect(result.subagentCount).toBe(0)
      expect(sidebarRowAccessibleName(result)).toBe('Reported name, Idle')
      expect(sidebarRowDescription(result)).toBe(`Reported name. Host: build-1. Status: Idle; live observation. Harness: ${harness}. Subagents: 0. ID: ${agent.ref}.`)
      const details = renderToStaticMarkup(createElement(AgentRowDetails, { row: result }))
      expect(details.match(/>Subagents<\/dt><dd[^>]*>([^<]*)/)?.[1]).toBe('0 collapsed children')
    }
  })
  it.each(['desired', 'starting'] as const)('describes observing %s reports as not yet reported', state => {
    const result = row({ harness: 'omp', driver: { _tag: 'Observing', name: 'omp' }, state })
    expect(result.subagentObservation).toEqual({ _tag: 'Unavailable' })
    expect(sidebarRowAccessibleName(result)).toBe('Reported name, ? subagents, Pending')
    expect(sidebarRowDescription(result)).toBe(`Reported name. Host: build-1. Status: Pending; live observation. Harness: omp. Not reported yet. ID: ${agent.ref}.`)
    const details = renderToStaticMarkup(createElement(AgentRowDetails, { row: result }))
    expect(details.match(/>Subagents<\/dt><dd[^>]*>([^<]*)/)?.[1]).toBe('Not reported yet')
  })
  it('retains stale OMP children and their count as last-verified facts', () => {
    const stale = row({ harness: 'omp', driver: { _tag: 'Observing', name: 'omp' }, subagents: [{
      id: 'reviewer', driver: 'omp', description: 'Review',
      lease_expires_at: '2026-10-18T12:01:00Z',
    }] }, true)
    expect(stale.subagentObservation).toEqual({ _tag: 'Reported' })
    expect(stale.subagentCount).toBe(1)
    expect(stale.children).toHaveLength(1)
    expect(stale.children[0]?.freshness).toBe('stale')
    expect(sidebarRowAccessibleName(stale)).toBe('Reported name, 1 subagent, Last verified · Idle')
    expect(sidebarRowDescription(stale)).toBe(`Reported name. Host: build-1. Status: Last verified · Idle; stale observation. Harness: omp. Subagents: 1. ID: ${agent.ref}.`)
    expect(renderToStaticMarkup(createElement(AgentRowDetails, { row: stale }))
      .match(/>Subagents<\/dt><dd[^>]*>([^<]*)/)?.[1]).toBe('1 collapsed children')
  })
})

describe('server-owned mission runs', () => {
  const work = { id: 'work/current', missionId: 'mission/example/review', runId: 'mission-run/example/run', path: 'review', state: 'claimed', title: 'Review changes' } as const
  it('never joins current or upcoming work into the row; the related-runs read owns mission facts', () => {
    const seat = sidebarRow({ agent: { ...agent, mission: work.missionId, currentWork: [work], upcomingWork: [{ ...work, missionId: 'mission/example/next' }] }, stale: false })
    expect(seat).not.toHaveProperty('missionContext')
    expect(seat).not.toHaveProperty('missionInfo')
    expect(seat.mission).toBe(work.missionId)
    expect(sidebarRowAccessibleName(seat)).toBe('Reported name, Idle')
  })
})

describe('reported model', () => {
  const usage = (model?: string) => ({ _tag: 'Known', value: {
    aggregation: 'cumulative-per-incarnation-else-response-deltas', cached_tokens: 0, incarnation_count: 1, input_tokens: 1, output_tokens: 1, total_tokens: 2,
    context: { used_tokens: 10, window_tokens: 100, observed_at_unix_ms: DateTime.makeUnsafe(1000), ...(model === undefined ? {} : { model }) },
  } }) as Agent['usage']
  it('shows only a fenced, concrete model and never invents one', () => {
    expect(sidebarRow({ agent: { ...agent, usage: usage('model-example') }, stale: false, now: 2000 }).model).toBe('model-example')
    for (const model of [undefined, '', ' ', '.unknown']) expect(sidebarRow({ agent: { ...agent, usage: usage(model) }, stale: false, now: 2000 }).model).toBeUndefined()
    expect(sidebarRow({ agent: { ...agent, usage: usage('model-example'), contextObservedAfter: 1000 }, stale: false, now: 2000 }).model).toBeUndefined()
    expect(row().model).toBeUndefined()
    expect(row()).not.toHaveProperty('effort')
  })
})
