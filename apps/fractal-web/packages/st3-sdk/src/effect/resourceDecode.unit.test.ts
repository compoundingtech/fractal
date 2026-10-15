import type { Resource as ResourceWire } from '@smalltalk/st3-client'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import { Option } from 'effect'
import { describe, expect, it } from 'vitest'

import oldLiteral from './agentSnapshot.oldLiteral.json'
import { decodeResourceRow, decodeResourceRows, makeDecodeDiagnostics, decodedUsageEquivalence, openAgent } from './resourceDecode.ts'
import { makeWindowIngest, type ProcessedWindow } from './windowIngest.ts'
import { frame } from './windowIngest.fixture.ts'

const agent = oldLiteral.items[0]!

describe('row-wise resource decoding', () => {
  it('decodes the synthetic old-literal agent snapshot', () => {
    const result = decodeResourceRows(oldLiteral.items)
    expect(result.items).toHaveLength(oldLiteral.items.length)
    expect(result.undecodable).toEqual([])
    expect(result.items[0]).toMatchObject({ id: agent.id, kind: 'agent', name: agent.name })
    const decoded = result.items[0]!
    if (decoded.kind !== 'agent') throw new Error('Expected example agent')
    expect(Option.isSome(decoded.usage)).toBe(true)
    expect(decoded).toEqual(decodeUnknownSync(Agent)(agent))
    expect(openAgent(decodeUnknownSync(Agent)(agent))).toEqual(decoded)
  })

  it('preserves a future aggregation as a real open leaf without fabricating known usage semantics', () => {
    const result = decodeResourceRow({ ...agent, usage: { ...agent.usage, aggregation: 'future-aggregation-v2' } })
    expect(result.value).toMatchObject({
      id: agent.id,
      decode: { unrecognized: [{ path: 'usage.aggregation', value: { _tag: 'Unrecognized', raw: 'future-aggregation-v2' } }] },
    })
    expect(result.undecodable).toEqual([{
      _tag: 'Undecodable', id: agent.id, path: 'usage.aggregation',
      reason: 'Unrecognized leaf value',
    }])
    if (result.value?.kind !== 'agent') throw new Error('Expected retained agent')
    expect(Option.isSome(result.value.usage)).toBe(true)
    expect(Option.getOrThrow(result.value.usage).aggregation).toEqual({ _tag: 'Unrecognized', raw: 'future-aggregation-v2' })
  })

  it('keeps broken identity typed and isolated from healthy sibling rows', () => {
    const result = decodeResourceRows([{ ...agent, id: 'not-a-resource-id' }, agent])
    expect(result.items).toHaveLength(1)
    expect(result.items[0]?.id).toBe(agent.id)
    expect(result.undecodable).toContainEqual({
      _tag: 'Undecodable', id: 'not-a-resource-id', path: 'id',
      reason: 'Value does not satisfy the resource field schema',
    })
    const missing = decodeResourceRow({ ...agent, id: 42 })
    expect(missing.value).toBeUndefined()
    expect(missing.undecodable).toContainEqual({
      _tag: 'Undecodable', path: 'id', reason: 'Value does not satisfy the resource field schema',
    })
  })

  it('opens generated enums without manufacturing a schema-known case', () => {
    const result = decodeResourceRow({ ...agent, state: 'future-state' })
    expect(result.value?.decode?.unrecognized).toContainEqual({
      path: 'state', value: { _tag: 'Unrecognized', raw: 'future-state' },
    })
    expect(result.value).toMatchObject({ state: { _tag: 'Unrecognized', raw: 'future-state' } })
  })

  it('opens other closed literal sets using generated schema metadata', () => {
    const result = decodeResourceRow({ ...agent, observation: 'future-observation' })
    expect(result.value?.decode?.unrecognized).toContainEqual({
      path: 'observation', value: { _tag: 'Unrecognized', raw: 'future-observation' },
    })
    expect(result.value).toMatchObject({ observation: { _tag: 'Unrecognized', raw: 'future-observation' } })
    expect(result.undecodable).toContainEqual({
      _tag: 'Undecodable', id: agent.id, path: 'observation',
      reason: 'Unrecognized leaf value',
    })
  })

  it('preserves nested structural failure paths without including row contents', () => {
    const result = decodeResourceRow({ ...agent, runtime_ids: ['broken-runtime-id'] })
    expect(result.value).toBeUndefined()
    expect(result.undecodable).toContainEqual({
      _tag: 'Undecodable', id: agent.id, path: 'runtime_ids.0',
      reason: 'Value does not satisfy the resource field schema',
    })
  })

  it('keeps identity discriminators and wrong primitive leaf types closed', () => {
    expect(decodeResourceRow({ ...agent, kind: 'future-agent' })).toMatchObject({
      undecodable: [{ _tag: 'Undecodable', path: 'kind' }],
    })
    const malformed = decodeResourceRow({ ...agent, usage: { ...agent.usage, aggregation: 42 } })
    expect(malformed.value).toBeUndefined()
  })

  it('compares unknown usage literals by their real raw value', () => {
    const first = decodeResourceRow({ ...agent, usage: { ...agent.usage, aggregation: 'future-a' } }).value
    const same = decodeResourceRow({ ...agent, usage: { ...agent.usage, aggregation: 'future-a' } }).value
    const changed = decodeResourceRow({ ...agent, usage: { ...agent.usage, aggregation: 'future-b' } }).value
    if (first?.kind !== 'agent' || same?.kind !== 'agent' || changed?.kind !== 'agent')
      throw new Error('Expected retained agents')
    expect(decodedUsageEquivalence(Option.getOrThrow(first.usage), Option.getOrThrow(same.usage))).toBe(true)
    expect(decodedUsageEquivalence(Option.getOrThrow(first.usage), Option.getOrThrow(changed.usage))).toBe(false)
  })

  it('retains the last good items for an all-bad window and recovers on good or empty snapshots', () => {
    const tasks: (() => void)[] = []
    const windows: ProcessedWindow<unknown>[] = []
    const ingest = makeWindowIngest({
      decode: (raw) => decodeResourceRow(raw).value,
      decodeDetailed: decodeResourceRow,
      publish: (window) => windows.push(window),
      now: () => 0,
      schedule: (work) => { tasks.push(work); return () => {} },
    })
    const send = (rows: readonly unknown[]) => {
      // Intentionally inject malformed JSON at the same untrusted wire boundary as the socket.
      ingest.accept(frame(rows as ResourceWire[]))
      while (tasks.length > 0) tasks.shift()!()
    }
    send([agent])
    send([{ ...agent, id: 'broken-id' }])
    expect(windows[1]?.items).toBe(windows[0]?.items)
    expect(windows[1]?.decodeStale).toBe(true)
    expect(windows[1]?.undecodable[0]).toMatchObject({ _tag: 'Undecodable', path: 'id' })
    send([agent])
    expect(windows[2]?.decodeStale).toBe(false)
    expect(windows[2]?.undecodable).toEqual([])
    send([])
    expect(windows[3]?.items).toEqual([])
    expect(windows[3]?.decodeStale).toBe(false)
  })

  it.each([42, undefined, null])('retains diagnostics for an identity %s that cannot join server order', (id) => {
    const tasks: (() => void)[] = []
    const windows: ProcessedWindow<unknown>[] = []
    const ingest = makeWindowIngest({
      decode: (raw) => decodeResourceRow(raw).value,
      decodeDetailed: decodeResourceRow,
      publish: (window) => windows.push(window),
      now: () => 0,
      schedule: (work) => { tasks.push(work); return () => {} },
    })
    const good = frame([agent] as ResourceWire[])
    if (good.kind !== 'snapshot') throw new TypeError('Expected a window snapshot')
    const send = (items: readonly unknown[]) => {
      ingest.accept({ ...good, items: items as ResourceWire[] })
      while (tasks.length > 0) tasks.shift()!()
    }
    send([agent])
    const broken = id === null ? null : { ...agent, id }
    send([broken])
    expect(windows[1]?.items).toBe(windows[0]?.items)
    expect(windows[1]?.decodeStale).toBe(true)
    expect(windows[1]?.undecodable).not.toHaveLength(0)
    expect(windows[1]?.rawItems).toEqual([broken])
    send([agent, broken])
    expect(windows[2]?.items).toHaveLength(1)
    expect(windows[2]?.decodeStale).toBe(false)
    expect(windows[2]?.undecodable).not.toHaveLength(0)
  })

  it('counts each resource/path and emits the first-path signal once per SDK session', () => {
    const observed: unknown[] = []
    const report = makeDecodeDiagnostics((resource, path, count, first) => observed.push({ resource, path, count, first }))
    const diagnostic = { _tag: 'Undecodable', path: 'usage.aggregation', reason: 'bad leaf' } as const
    report('agents', [diagnostic, diagnostic])
    report('sessions', [diagnostic])
    report('agents', [{ ...diagnostic, path: 'id' }])
    expect(observed).toEqual([
      { resource: 'agents', path: 'usage.aggregation', count: 1, first: true },
      { resource: 'agents', path: 'usage.aggregation', count: 2, first: false },
      { resource: 'sessions', path: 'usage.aggregation', count: 1, first: true },
      { resource: 'agents', path: 'id', count: 1, first: true },
    ])
  })
})
