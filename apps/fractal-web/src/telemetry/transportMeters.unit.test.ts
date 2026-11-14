import { makeSeries, makeSeriesStore } from '@overeng/meters'
import type { NumberValue } from '@overeng/meters'
import { describe, expect, it, vi } from 'vitest'

import { describeSocket, readTransport, textTileBlock } from './transportMeters.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const bag = (values: Readonly<Record<string, number>>) => (key: string): number => values[key] ?? 0
const number = (value: number) => ({ _tag: 'Value', value: { _tag: 'Number', value } })
const unavailable = (reason: string) => ({ _tag: 'Unavailable', reason })
const live = { state: { _tag: 'Live' }, sinceMs: 0 } as const

describe('transport readings never report an unmeasured value as zero', () => {
  it('HTTP readings are unavailable until the fetch is instrumented', () => {
    const read = bag({})
    expect(readTransport.active(read)).toEqual(unavailable('NotConfigured'))
    expect(readTransport.errors(read)).toEqual(unavailable('NotConfigured'))
    expect(readTransport.p50(read)).toEqual(unavailable('NotConfigured'))
  })
  it('active and errors are a real zero once instrumented, while latency waits for a response', () => {
    const read = bag({ 'Wf.httpInstrumented': 1 })
    expect(readTransport.active(read)).toEqual(number(0))
    expect(readTransport.errors(read)).toEqual(number(0))
    expect(readTransport.p50(read)).toEqual(unavailable('NoSamples'))
    expect(readTransport.p95(read)).toEqual(unavailable('NoSamples'))
  })
  it('reports latency once a response is sampled', () => {
    const read = bag({ 'Wf.httpInstrumented': 1, 'Wf.httpSamples': 2, 'Wf.httpP50Ms': 27, 'Wf.httpP95Ms': 63 })
    expect(readTransport.p50(read)).toEqual(number(27))
    expect(readTransport.p95(read)).toEqual(number(63))
  })
  it('subscriptions need the configured limit', () => {
    expect(readTransport.subscriptions(bag({}))).toEqual(unavailable('NotConfigured'))
    expect(readTransport.subscriptions(bag({ 'Wf.followCap': 16, 'Wf.activeFollows': 4 })))
      .toEqual({ _tag: 'Value', value: { used: 4, cap: 16 } })
  })
  it('the message count is only a measure while the socket is live', () => {
    const read = bag({ 'Wf.frames': 9 })
    expect(readTransport.messages(read, undefined)).toEqual(unavailable('NotConfigured'))
    expect(readTransport.messages(read, { state: { _tag: 'Closed' }, sinceMs: 0 })).toEqual(unavailable('Disconnected'))
    expect(readTransport.messages(read, live)).toEqual(number(9))
  })
})

describe('socket state', () => {
  it('names every state in words and separates unobserved from down', () => {
    expect(describeSocket(undefined).label).toBe('n/a')
    expect(describeSocket(live)).toMatchObject({ label: 'live', tone: 'normal' })
    expect(describeSocket({ state: { _tag: 'Closed' }, sinceMs: 0 })).toMatchObject({ label: 'down', tone: 'danger' })
    expect(describeSocket({ state: { _tag: 'Reconnecting', attempt: 3, issue: 'Socket closed' }, sinceMs: 0 }))
      .toMatchObject({ label: 'reconnecting', tone: 'warning', detail: 'Reconnecting, attempt 3: Socket closed' })
  })
})

describe('textTileBlock', () => {
  it('describes values and availability from the series', () => {
    const series = makeSeries<NumberValue>({ id: 'test.errors', label: 'Errors', unit: 'count', capacity: 4 })
    const store = makeSeriesStore()
    const writer = store.register({ series })
    const reader = textTileBlock({
      id: 'errors', series, shortLabel: 'err', widthPx: 64,
      value: (sample) => sample.value,
      text: (sample) => String(sample.value),
      describe: (sample) => `${sample.value} since load`,
      tone: (_, theme) => theme.normal,
    }).read(store)
    expect(reader.describe({ nowMs: 0 })).toBe('n/a (NoSamples)')
    writer.append({ sample: { _tag: 'Unavailable', atMs: 1, reason: 'NotConfigured' } })
    expect(reader.describe({ nowMs: 1 })).toBe('n/a (NotConfigured)')
    writer.append({ sample: { _tag: 'Value', atMs: 2, value: { _tag: 'Number', value: 3 } } })
    expect(reader.describe({ nowMs: 2 })).toBe('3 since load')
  })
})
