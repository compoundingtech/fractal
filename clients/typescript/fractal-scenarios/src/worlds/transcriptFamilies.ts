import type { TimelineEntry } from '@fractal/client'

import { scenarioDomainFixture } from '../domain-fixture.ts'
import { child, scenarioId } from '../kit/context.ts'
import { agent } from '../kit/factories/agent.ts'
import * as resources from '../kit/resources.ts'
import type { Slice } from '../kit/slice.ts'
import { liveSync } from '../kit/variants.ts'
import type { WorldDefinition } from '../kit/world.ts'
import { empty } from './empty.ts'

/** One inspection, a failed export, and a deliberately cancelled retry. */
export const transcriptFamilies: WorldDefinition = {
  id: 'transcript-families',
  title: 'Transcript families',
  narrative: 'An Atlas builder inspects a release report using visible reasoning and native structured tools, loses an export run, then cancels its retry before summarizing the retained evidence.',
  seed: 25,
  cast: { project: 'atlas', roles: ['builder'], hosts: 1, people: 1, missions: [] },
  slices: (ctx) => {
    const member = ctx.cast.agents[0]!
    const person = ctx.cast.people[0]!
    const inherited = empty.slices(ctx)
    const source = { _tag: 'synthetic', seed: 25 } as const
    const built = agent(child(ctx, 'builder'), member, {
      state: 'waiting', sinceMs: -120_000, lastActivityMs: 0, harnessState: 'idle',
    })
    let sequence = 0
    const next = (atMs: number) => {
      sequence += 1
      return {
        id: scenarioId(ctx, 'timeline-entry', `builder-${sequence}`),
        sequence, revision: 1, timestamp: ctx.t.at(atMs), final: true,
      }
    }
    const request = (atMs: number, text: string): TimelineEntry[] => [
      { ...next(atMs), role: 'user', type: 'message',
        body: { message_id: scenarioId(ctx, 'message', `request-${sequence}`), from: person.id, to: member.id } },
      { ...next(atMs), role: 'user', type: 'content', body: { media_type: 'text/plain', text } },
    ]
    const items: TimelineEntry[] = [
      ...request(-120_000, 'Inspect the release report before exporting it.'),
      { ...next(-116_000), role: 'assistant', type: 'content', body: {
        media_type: 'text/markdown', text: 'I will compare the release scope and report totals before exporting.',
        blocks: [{ id: 'inspection-reasoning', kind: 'reasoning', source_type: 'reasoning', visibility: 'visible',
          payload: { text: 'Compare the report totals with the release scope before requesting an export.' } }],
      } },
      { ...next(-112_000), role: 'assistant', type: 'status', body: { status: 'completed', detail: 'Inspection plan ready.' } },
      ...request(-100_000, 'Compare the release scope and retrieve the report totals.'),
      { ...next(-96_000), role: 'assistant', type: 'tool_call', body: {
        call_id: 'inspect-report', name: 'catalog.compare_release',
        arguments: { releases: ['atlas-7', 'atlas-8'], include: { totals: true, sections: ['runtime', 'client'] }, limit: 3 },
      } },
      { ...next(-92_000), role: 'tool', type: 'tool_result', body: {
        call_id: 'inspect-report', status: 'success', media_type: 'application/json',
        content: { release: 'atlas-8', totals: { passed: 18, failed: 0 }, sections: [{ name: 'runtime', ready: true }, { name: 'client', ready: true }] },
      } },
      { ...next(-88_000), role: 'system', type: 'status', body: { status: 'completed', detail: 'Both report sections are ready.' } },
      ...request(-76_000, 'Export the checked report.'),
      { ...next(-72_000), role: 'assistant', type: 'tool_call', body: {
        call_id: 'failed-export', name: 'reports.export', arguments: { release: 'atlas-8', formats: ['json', 'csv'], destination: { bucket: 'demo-reports' } },
      } },
      { ...next(-68_000), role: 'system', type: 'error', body: {
        code: 'export_worker_lost', message: 'The export worker disconnected before returning a result.',
        retryable: true, details: { call_id: 'failed-export', retained_report: 'atlas-8' },
      } },
      { ...next(-64_000), role: 'system', type: 'status', body: {
        status: 'failed', detail: 'Export worker lost; report totals retained, but no export result was received.',
      } },
      ...request(-52_000, 'Retry the export, but stop if the destination is still unavailable.'),
      { ...next(-48_000), role: 'assistant', type: 'tool_call', body: {
        call_id: 'cancelled-export', name: 'reports.export', arguments: { release: 'atlas-8', formats: ['json'], destination: { bucket: 'demo-reports' }, dry_run: true },
      } },
      { ...next(-44_000), role: 'system', type: 'status', body: {
        status: 'cancelled', detail: 'Operator cancelled the retry while its destination check was outstanding; no export was produced.',
      } },
      ...request(-8_000, 'Summarize what is safe to hand off without an export.'),
    ]
    const partial: TimelineEntry = { ...next(0), role: 'assistant', type: 'content', final: false,
      body: { media_type: 'text/markdown', text: 'The report totals are retained; checking the handoff summary.' } }
    items.push(partial)
    const final: TimelineEntry = { ...partial, revision: 2, timestamp: ctx.t.at(2_000), final: true,
      body: { media_type: 'text/markdown', text: 'All 18 checks passed. The export failed and its retry was cancelled; hand off the report totals, not an export artifact.' } }
    const conversation: Slice<'conversation'> = {
      kind: 'conversation', variant: 'default', source, decode: 'strict', loading: false,
      state: { threads: [{ agent: member.id, session_id: member.session, items, page_size: 100, has_more: false }] },
      timeline: [{ _tag: 'entries', at_ms: 2_000, store: 0, agent: member.id, items: [final,
        { ...next(2_000), role: 'system', type: 'status', body: { status: 'completed', detail: 'Handoff summary complete; no export claimed.' } },
      ] }],
    }
    return {
      roster: { ...inherited.roster, source, state: {
        sessions: [], devices: [],
        agents: [built.agent], runtimes: built.runtime === null ? [] : [built.runtime],
        machines: [resources.machine(ctx, member.host, [member.runtime], [])], order: [member.id],
      } },
      details: { ...inherited.details, source },
      attention: { ...inherited.attention, source },
      terminal: { ...inherited.terminal, source },
      conversation,
      sync: { ...liveSync(ctx, { conversation }), source },
    }
  },
}

export const scenarioDomainFixtureMarker: typeof scenarioDomainFixture = scenarioDomainFixture
