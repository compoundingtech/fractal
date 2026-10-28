import { describe, expect, it } from 'vitest'

import { conversationEntries } from '../../fractal-views/conversationView.ts'

import { decodeSlice } from '../scripts/decode.ts'
import { loadWorld } from '../src/index.ts'
import { foldSlice } from '../src/kit/fold.ts'
import { ANCHOR_MS } from '../src/kit/time.ts'

const world = loadWorld('transcript-families', { now: ANCHOR_MS })
const conversation = world.slices.conversation
const folded = foldSlice(conversation, 2_000).state
const items = folded.threads[0]!.items
const names = new Map([[world.cast.people[0]!.id, 'Ada'], [world.cast.agents[0]!.id, 'Atlas builder']])
const projected = conversationEntries(items, names)

describe('transcript-families', () => {
  it('keeps one agent and one strictly decoded conversation thread', () => {
    expect(world.cast.agents).toHaveLength(1)
    expect(world.slices.roster.state.agents).toHaveLength(1)
    expect(folded.threads).toHaveLength(1)
    expect(folded.threads[0]!.agent).toBe(world.cast.agents[0]!.id)
    expect(items.filter((item) => item.type === 'message')).toHaveLength(5)
    expect(items.map((item) => item.sequence)).toEqual(Array.from({ length: items.length }, (_, index) => index + 1))
    expect(decodeSlice(world.id, conversation)).toEqual([])
  })

  it('projects all five exact prompts as user-authored mail and never misattributes assistant replies', () => {
    const prompts = [
      'Inspect the release report before exporting it.',
      'Compare the release scope and retrieve the report totals.',
      'Export the checked report.',
      'Retry the export, but stop if the destination is still unavailable.',
      'Summarize what is safe to hand off without an export.',
    ]
    const userEntries = projected.filter((entry) => entry.body.kind === 'mail' || entry.body.kind === 'user')
    expect(userEntries.map((entry) => entry.body)).toEqual(prompts.map((text) => ({
      kind: 'mail', from: 'Ada', to: 'Atlas builder', subject: '', text,
    })))
    let promptIndex = 0
    for (const [index, item] of items.entries()) {
      if (item.type !== 'message') continue
      expect(item.role).toBe('user')
      expect(item.body.blocks).toBeUndefined()
      expect(items[index + 1]).toMatchObject({
        role: 'user', type: 'content', timestamp: item.timestamp,
        body: { media_type: 'text/plain', text: prompts[promptIndex++] },
      })
    }
    const assistant = items.filter((item) => item.type === 'content').filter((item) => item.role === 'assistant')
    expect(projected.filter((entry) => entry.body.kind === 'assistant').map((entry) => entry.body))
      .toEqual(assistant.map((item) => ({ kind: 'assistant', text: item.body.text })))
  })

  it('projects reasoning content, generic tools and structured output while preserving terminal wire evidence', () => {
    const reasoning = items.find((item) => item.type === 'content' && item.body.blocks?.some((block) => block.kind === 'reasoning'))!
    expect(projected.find((entry) => entry.id === reasoning.id)).toMatchObject({
      body: { kind: 'assistant', text: 'I will compare the release scope and report totals before exporting.' },
    })
    expect(reasoning.body).toMatchObject({ blocks: [expect.objectContaining({ kind: 'reasoning', visibility: 'visible' })] })
    expect(projected.filter((entry) => entry.body.kind === 'tool').map((entry) => entry.body)).toEqual([
      { kind: 'tool', title: 'catalog.compare_release', state: 'ok', output: [
        JSON.stringify({ release: 'atlas-8', totals: { passed: 18, failed: 0 }, sections: [{ name: 'runtime', ready: true }, { name: 'client', ready: true }] }),
      ] },
      { kind: 'tool', title: 'reports.export', state: 'running', output: [] },
      { kind: 'tool', title: 'reports.export', state: 'running', output: [] },
    ])
    for (const status of ['failed', 'cancelled']) {
      const terminal = items.find((item) => item.type === 'status' && item.body.status === status)!
      expect(terminal).toMatchObject({ final: true, body: { status, detail: expect.any(String) } })
      // The current consumer deliberately omits status rows; terminal evidence stays on the wire.
      expect(projected.some((entry) => entry.id === terminal.id)).toBe(false)
    }
    expect(projected).toContainEqual(expect.objectContaining({
      body: expect.objectContaining({ kind: 'event', text: expect.stringContaining('export worker disconnected') }),
    }))
    const partial = conversation.state.threads[0]!.items.at(-1)!
    expect(conversationEntries(conversation.state.threads[0]!.items, names).find((entry) => entry.id === partial.id))
      .toMatchObject({ body: { kind: 'assistant', text: 'The report totals are retained; checking the handoff summary.' } })
    expect(projected.find((entry) => entry.id === partial.id)).toMatchObject({
      body: { kind: 'assistant', text: 'All 18 checks passed. The export failed and its retry was cancelled; hand off the report totals, not an export artifact.' },
    })
    expect(projected.filter((entry) => entry.id === partial.id)).toHaveLength(1)
  })

  it('exposes visible reasoning on content blocks, not a new entry type', () => {
    const reasoning = items.filter((item) => item.type === 'content').flatMap((item) => item.body.blocks ?? [])
    expect(reasoning).toContainEqual({
      id: 'inspection-reasoning', kind: 'reasoning', source_type: 'reasoning', visibility: 'visible',
      payload: { text: 'Compare the report totals with the release scope before requesting an export.' },
    })
  })

  it('retains generic tool arguments and a native structured result', () => {
    const call = items.find((item) => item.type === 'tool_call' && item.body.call_id === 'inspect-report')
    expect(call).toMatchObject({ type: 'tool_call', body: {
      name: 'catalog.compare_release',
      arguments: { releases: ['atlas-7', 'atlas-8'], include: { totals: true, sections: ['runtime', 'client'] }, limit: 3 },
    } })
    const result = items.find((item) => item.type === 'tool_result' && item.body.call_id === 'inspect-report')
    expect(result).toMatchObject({ type: 'tool_result', body: {
      call_id: 'inspect-report', status: 'success', media_type: 'application/json',
      content: { release: 'atlas-8', totals: { passed: 18, failed: 0 }, sections: [{ name: 'runtime', ready: true }, { name: 'client', ready: true }] },
    } })
  })

  it.each([
    ['failed', 'failed-export', 'Export worker lost; report totals retained, but no export result was received.'],
    ['cancelled', 'cancelled-export', 'Operator cancelled the retry while its destination check was outstanding; no export was produced.'],
  ])('retains a %s terminal turn with its unresolved call and detail', (status, callId, detail) => {
    const terminalIndex = items.findIndex((item) => item.type === 'status' && item.body.status === status)
    expect(terminalIndex).toBeGreaterThan(0)
    expect(items[terminalIndex]).toMatchObject({ final: true, type: 'status', body: { status, detail } })
    const turnStart = items.slice(0, terminalIndex).reduce((last, item, index) => item.type === 'message' ? index : last, -1)
    const turn = items.slice(turnStart, terminalIndex + 1)
    expect(turn.some((item) => item.type === 'tool_call' && item.body.call_id === callId)).toBe(true)
    expect(items.some((item) => item.type === 'tool_result' && item.body.call_id === callId)).toBe(false)
    if (status === 'failed') expect(turn).toContainEqual(expect.objectContaining({ type: 'error', body: {
      code: 'export_worker_lost', message: 'The export worker disconnected before returning a result.',
      retryable: true, details: { call_id: callId, retained_report: 'atlas-8' },
    } }))
  })

  it('revises the same streaming entry from revision one to a final summary', () => {
    const partial = conversation.state.threads[0]!.items.at(-1)!
    expect(partial).toMatchObject({ revision: 1, final: false, type: 'content' })
    expect(foldSlice(conversation, 1_999).state.threads[0]!.items.at(-1)).toEqual(partial)
    const final = items.find((item) => item.id === partial.id)
    expect(final).toMatchObject({ id: partial.id, sequence: partial.sequence, revision: 2, final: true, type: 'content',
      body: { text: 'All 18 checks passed. The export failed and its retry was cancelled; hand off the report totals, not an export artifact.' } })
    expect(items.filter((item) => item.id === partial.id)).toHaveLength(1)
    expect(items.at(-1)).toMatchObject({ type: 'status', body: { status: 'completed' } })
  })
})
