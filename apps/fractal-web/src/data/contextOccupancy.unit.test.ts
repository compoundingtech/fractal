import { Agent as NativeAgent, decodeUnknownSync, UsageSummary, type UsageSummaryEncoded } from '@fractal/client/schema'
import { openAgent } from '@fractal/sdk/effect'
import { describe, expect, it } from 'vitest'
import * as Option from 'effect/Option'
import { contextOccupancy } from './contextOccupancy.ts'
import { createProjections } from './projections.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import { observed, unavailable, type Agent } from './source.ts'
import { projectNativeDetails } from './nativeDetails.ts'

const usage = (context?: UsageSummaryEncoded['context']) => ({ _tag: 'Known' as const, value: decodeUnknownSync(UsageSummary)({
  aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1,
  input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 480_035_214,
  ...(context === undefined ? {} : { context }),
}) })
const reading = { used_tokens: 61_206, window_tokens: 1_000_000, used_percent: 6.1206, observed_at_unix_ms: 3000, model: 'observed-model', compactions: 2 }
const native = openAgent(decodeUnknownSync(NativeAgent)({ kind: 'agent', id: 'agent/example-details', name: 'Example', host_id: 'host/example', runtime_ids: [], reachability: 'reachable', state: 'running', revision: '1', updated_at: '1970-01-01T00:00:04.000Z', incarnation_id: 'first', usage: { aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 480_035_214, context: reading } }))
const occupancy = (agent: Agent) => contextOccupancy({ usage: agent.usage, now: 10_000, observedAfter: agent.contextObservedAfter })
const details = (row: typeof native, agent: Agent) => {
  const missing = unavailable({ reason: 'unsupported', detail: 'Not reported' })
  const reads = { refreshUsage: () => undefined, retrySession: () => undefined, retryQueue: () => undefined }
  return projectNativeDetails({ agentRef: native.id, row: observed({ value: row }), agent: observed({ value: agent }), period: missing, session: missing, declarations: missing, missions: missing, now: 10_000, onRetry: () => undefined, reads })
}

describe('reported context occupancy', () => {
  it('uses occupancy numerator/window, never cumulative tokens; preserves zero and percentage-only reports', () => {
    const projected = contextOccupancy({ usage: usage({ ...reading, used_percent: 6 }), now: 10_000 })
    expect(projected).toMatchObject({ _tag: 'Known', usedTokens: 61_206, windowTokens: 1_000_000, observedAt: 3000 })
    expect(projected._tag === 'Known' ? projected.usedPercent : undefined).toBeCloseTo(6.1206, 6)
    expect(contextOccupancy({ usage: usage({ ...reading, used_tokens: 0, used_percent: 0 }), now: 10_000 })).toMatchObject({ _tag: 'Known', usedPercent: 0 })
    expect(contextOccupancy({ usage: usage({ used_percent: 42, observed_at_unix_ms: 3000 }), now: 10_000 })).toEqual({ _tag: 'Known', usedPercent: 42, observedAt: 3000 })
  })
  it('does not interpret occupancy under an unrecognized usage aggregation', () => {
    expect(contextOccupancy({
      usage: { _tag: 'Known', value: { ...usage(reading).value,
        aggregation: { _tag: 'Unrecognized', raw: 'future-aggregation' } } },
      now: 10_000,
    })).toEqual({ _tag: 'Unknown' })
  })
  it('rejects missing, future and fenced readings without inventing capacity', () => {
    for (const value of [{ _tag: 'Unknown' } as const, usage(), usage({ observed_at_unix_ms: 3000 }), usage({ used_tokens: 50, window_tokens: 0, observed_at_unix_ms: 3000 })])
      expect(contextOccupancy({ usage: value, now: 10_000 })).toEqual({ _tag: 'Unknown' })
    expect(contextOccupancy({ usage: usage(reading), now: 2000 })).toEqual({ _tag: 'Unknown' })
    expect(contextOccupancy({ usage: usage(reading), now: 10_000, observedAfter: 3000 })).toEqual({ _tag: 'Unknown' })
    expect(contextOccupancy({ usage: usage(reading), now: 10_000, observedAfter: 2999 })).toMatchObject({ _tag: 'Known' })
  })
  it('clears on an incarnation change, stays cleared on equal readings, and restores only on newer evidence', () => {
    const project = createProjections()
    const first = project.fleetFromAgents([native]).agents[0]!
    expect(sidebarRow({ agent: first, now: 10_000, stale: false }).context?._tag).toBe('Known')
    const restarted = { ...native, incarnation_id: Option.some('second') }
    const second = project.fleetFromAgents([restarted]).agents[0]!
    expect(sidebarRow({ agent: second, now: 10_000, stale: false }).context).toEqual({ _tag: 'Unknown' })
    // Reusing the same native DTO cannot bypass the transition fence in the WeakMap cache.
    const repeated = project.fleetFromAgents([restarted]).agents[0]!
    expect(repeated).toBe(second)
    expect(sidebarRow({ agent: repeated, now: 10_000, stale: false }).context).toEqual({ _tag: 'Unknown' })
    const newer = project.fleetFromAgents([{ ...restarted, usage: Option.some(usage({ ...reading, observed_at_unix_ms: 5000 }).value) }]).agents[0]!
    expect(sidebarRow({ agent: newer, now: 10_000, stale: false }).context).toMatchObject({ _tag: 'Known', observedAt: 5000 })
    expect(sidebarRow({ agent: newer, now: 10_000, stale: true }).context).toMatchObject({ _tag: 'Known', observedAt: 5000 })
    expect(() => expect(sidebarRow({ agent: second, now: 10_000, stale: false }).context?._tag).toBe('Known')).toThrow()
  })
  it('clears context, model and compactions on restart, including cached DTOs, until strictly newer evidence', () => {
    const project = createProjections()
    const first = project.fleetFromAgents([native]).agents[0]!
    expect(occupancy(first)._tag).toBe('Known')
    const restarted = { ...native, incarnation_id: Option.some('second') }
    const second = project.fleetFromAgents([restarted]).agents[0]!
    expect(second).not.toBe(first)
    expect(occupancy(second)).toEqual({ _tag: 'Unknown' })
    expect(details(restarted, second).overview.model).toBeUndefined()
    expect(details(restarted, second).trace.meters.context.compactions._tag).toBe('Unknown')
    expect(project.fleetFromAgents([restarted]).agents[0]).toBe(second)
    const equal = project.fleetFromAgents([{ ...restarted }]).agents[0]!
    expect(equal).toBe(second)
    const fresh = { ...restarted, usage: Option.some(usage({ ...reading, observed_at_unix_ms: 5000 }).value) }
    const newer = project.fleetFromAgents([fresh]).agents[0]!
    expect(occupancy(newer)).toMatchObject({ _tag: 'Known', observedAt: 5000 })
    expect(details(fresh, newer).overview.model).toBe('observed-model')
    expect(details(fresh, newer).trace.meters.context.compactions).toMatchObject({ _tag: 'Known', value: 2 })
  })
  it('retains the fence when occupancy is omitted at the transition', () => {
    const project = createProjections()
    const previous = { ...native, usage: Option.some(usage({ ...reading, observed_at_unix_ms: 6000 }).value) }
    project.fleetFromAgents([previous])
    project.fleetFromAgents([{ ...native, incarnation_id: Option.some('second'), usage: Option.none() }])
    const retained = project.fleetFromAgents([{ ...previous, incarnation_id: Option.some('second') }]).agents[0]!
    expect(sidebarRow({ agent: retained, now: 10_000, stale: false }).context).toEqual({ _tag: 'Unknown' })
  })
  it('keeps the incarnation fence when the ref leaves a bounded roster window and returns', () => {
    for (const returning of ['same', 'cloned'] as const) {
      const project = createProjections()
      project.fleetFromAgents([native])
      const restarted = { ...native, incarnation_id: Option.some('second') }
      expect(sidebarRow({ agent: project.fleetFromAgents([restarted]).agents[0]!, now: 10_000, stale: false }).context).toEqual({ _tag: 'Unknown' })
      // The ref is evicted from the window (e.g. liveSource's 100-row limit), then reappears.
      project.fleetFromAgents([])
      const back = project.fleetFromAgents([returning === 'same' ? restarted : { ...restarted }]).agents[0]!
      expect(back.contextObservedAfter).toBe(4000)
      expect(sidebarRow({ agent: back, now: 10_000, stale: false }).context).toEqual({ _tag: 'Unknown' })
      project.fleetFromAgents([])
      const fresh = project.fleetFromAgents([{ ...restarted, usage: Option.some(usage({ ...reading, observed_at_unix_ms: 5000 }).value) }]).agents[0]!
      expect(sidebarRow({ agent: fresh, now: 10_000, stale: false }).context).toMatchObject({ _tag: 'Known', observedAt: 5000 })
    }
  })
})
