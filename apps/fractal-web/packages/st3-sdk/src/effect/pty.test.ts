import { it } from '@effect/vitest'
import * as Effect from 'effect/Effect'
import * as Result from 'effect/Result'
import * as SubscriptionRef from 'effect/SubscriptionRef'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { encodePtyFrame, makePtyFrameReader, openPtySession, PtyAttachFailure, ptyBackoffMillis, ptyFallback, ptyStreamUrl, ptyTransition, requestPtyAttachment, type PtyFrame, type PtyOutput, type PtySession, type PtySocket, type PtyState } from './pty.ts'
const bytes = Uint8Array.of(0, 128, 255)
const frames: PtyFrame[] = [
  { _tag: 'Data', bytes }, { _tag: 'Attach', rows: 24, cols: 80 },
  { _tag: 'Attach', rows: 65535, cols: 0, cellWidth: 8, cellHeight: 16 },
  { _tag: 'Detach' }, { _tag: 'Resize', rows: 30, cols: 90, cellWidth: 9, cellHeight: 18 },
  { _tag: 'Exit', code: -123 }, { _tag: 'Screen', bytes }, { _tag: 'Peek', plain: true, full: true },
  { _tag: 'Status', payload: bytes }, { _tag: 'Geometry', rows: 20, cols: 70 },
  { _tag: 'ResetInputModes' }, { _tag: 'Unknown', tag: 8, payload: bytes }, { _tag: 'Unknown', tag: 9, payload: bytes },
]
const concatenate = (chunks: readonly Uint8Array[]) => {
  const result = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.length, 0))
  let offset = 0
  for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.length }
  return result
}
const read = (chunk: Uint8Array) => Result.getOrThrow(makePtyFrameReader().feed(chunk))

describe('PTY framing', () => {
  test.each(frames)('round trips $ _tag', frame => expect(read(encodePtyFrame(frame))).toEqual([frame]))
  test('reads byte-by-byte, concatenated frames, and split headers', () => {
    const all = concatenate(frames.map(encodePtyFrame))
    const reader = makePtyFrameReader()
    const result = [...all].flatMap(byte => Result.getOrThrow(reader.feed(Uint8Array.of(byte))))
    expect(result).toEqual(frames)
    expect(read(all)).toEqual(frames)
    const split = makePtyFrameReader()
    expect(Result.getOrThrow(split.feed(all.subarray(0, 3)))).toEqual([])
    expect(Result.getOrThrow(split.feed(all.subarray(3)))).toEqual(frames)
  })
  test('rejects oversized headers before allocating and malformed known frames', () => {
    const header = Uint8Array.of(0, 1, 0, 0, 1)
    const reader = makePtyFrameReader()
    expect(Result.isFailure(reader.feed(header))).toBe(true)
    expect(Result.isFailure(reader.feed(new Uint8Array(0)))).toBe(true)
    for (const tag of [1, 2, 3, 4, 6, 10, 11]) {
      expect(Result.isFailure(makePtyFrameReader().feed(Uint8Array.of(tag, 0, 0, 0, 2, 0, 0)))).toBe(true)
    }
  })
  test('ignores attach identity and geometry suffixes', () => {
    const attach = encodePtyFrame(frames[2]!)
    const identity = concatenate([attach, new TextEncoder().encode('{}')])
    new DataView(identity.buffer).setUint32(1, identity.length - 5)
    expect(read(identity)).toEqual([frames[2]])
    const geometry = Uint8Array.of(10, 0, 0, 0, 8, 0, 24, 0, 80, 0, 8, 0, 16)
    expect(read(geometry)).toEqual([{ _tag: 'Geometry', rows: 24, cols: 80 }])
  })
})

describe('PTY state machine', () => {
  test('connects only on geometry and resets consecutive failures after live', () => {
    let state: PtyState = { _tag: 'Connecting', attempt: 1 }
    expect(ptyTransition({ state, event: { _tag: 'SocketOpened' } })).toEqual(state)
    state = ptyTransition({ state, event: { _tag: 'Geometry', rows: 20, cols: 70 } })
    expect(state).toEqual({ _tag: 'Live', rows: 20, cols: 70 })
    state = ptyTransition({ state, event: { _tag: 'SocketLost', issue: 'drop' } })
    expect(state).toEqual({ _tag: 'Reconnecting', attempt: 1, delayMs: 500, issue: 'drop' })
    state = ptyTransition({ state, event: { _tag: 'RetryDue' } })
    expect(state).toEqual({ _tag: 'Connecting', attempt: 2 })
    state = ptyTransition({ state, event: { _tag: 'SocketLost', issue: 'drop' } })
    expect(state).toEqual({ _tag: 'Reconnecting', attempt: 2, delayMs: 1000, issue: 'drop' })
    state = ptyTransition({ state, event: { _tag: 'RetryDue' } })
    expect(ptyTransition({ state, event: { _tag: 'SocketLost', issue: 'drop' } })).toEqual({ _tag: 'Failed', message: 'drop' })
    expect(ptyBackoffMillis(100)).toBe(30000)
    expect(ptyBackoffMillis(0)).toBe(500)
  })
  test.each([
    [403, 'forbidden', 'Refused'], [401, 'unauthorized', 'Refused'], [409, 'stale-fence', 'Stale'], [404, 'not-found', 'Stale'],
  ] as const)('maps HTTP %s %s to %s', (status, code, tag) => {
    const failure = new PtyAttachFailure({ message: 'no', status, code, retryable: false })
    expect(ptyTransition({ state: { _tag: 'Connecting', attempt: 1 }, event: { _tag: 'AttachFailed', failure } })._tag).toBe(tag)
  })
  test('transient attach failures retry; invalid envelopes do not', () => {
    for (const retryable of [true, false]) {
      const state = ptyTransition({ state: { _tag: 'Connecting', attempt: 1 }, event: { _tag: 'AttachFailed', failure: new PtyAttachFailure({ message: 'bad', retryable }) } })
      expect(state._tag).toBe(retryable ? 'Reconnecting' : 'Failed')
    }
  })
  test('terminal states stay terminal and fallback is limited to refusal/failure', () => {
    const states: PtyState[] = [{ _tag: 'Exited', code: -1 }, { _tag: 'Closed' }, { _tag: 'Stale', message: 'stale' }, { _tag: 'Refused', message: 'no' }, { _tag: 'Failed', message: 'bad' }]
    for (const state of states) expect(ptyTransition({ state, event: { _tag: 'Geometry', rows: 1, cols: 1 } })).toEqual(state)
    expect(ptyTransition({ state: { _tag: 'Live', rows: 1, cols: 1 }, event: { _tag: 'ProcessExited', code: -1 } })).toEqual(states[0])
    expect(ptyTransition({ state: states[0]!, event: { _tag: 'Closed' } })).toEqual(states[1])
    expect(states.map(ptyFallback)).toEqual([undefined, undefined, undefined, { reason: 'refused', message: 'no' }, { reason: 'failed', message: 'bad' }])
  })
})

beforeEach(() => vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] }))
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers() })
const settle = Effect.promise(async () => {
  for (let round = 0; round < 10; round += 1) {
    await new Promise<void>(resolve => setImmediate(resolve))
    await vi.advanceTimersByTimeAsync(0)
  }
})
const target = { terminalId: 'terminal/a/b', incarnation: 'runtime/new incarnation' }
class Gateway {
  readonly requests: { url: string; init: RequestInit | undefined }[] = []
  readonly sockets: { url: string; protocols: readonly string[]; socket: PtySocket; sent: ArrayBuffer[]; closes: (number | undefined)[] }[] = []
  readonly fetch: typeof fetch = async (url, init) => {
    this.requests.push({ url: String(url), init })
    return Response.json({ api_version: 'st3.client.v0', snapshot: {}, value: { terminal_id: target.terminalId, runtime_incarnation: target.incarnation, owner_host_id: 'host/a', mode: 'attach', stream_capability: `cap-${this.requests.length}` } })
  }
  readonly factory = (url: string, protocols: readonly string[]): PtySocket => {
    const sent: ArrayBuffer[] = []
    const closes: (number | undefined)[] = []
    const socket: PtySocket = { binaryType: '', onopen: null, onmessage: null, onclose: null, onerror: null, send: data => sent.push(data), close: code => { closes.push(code) } }
    this.sockets.push({ url, protocols, socket, sent, closes })
    queueMicrotask(() => socket.onopen?.({}))
    return socket
  }
}

it.live('speaks binary PTY, reconnects with a fresh grant, and detaches on scope close', () => Effect.gen(function* () {
  const gateway = new Gateway()
  const output: PtyOutput[] = []
  let session: PtySession | undefined
  yield* Effect.gen(function* () {
    session = yield* openPtySession({ baseUrl: 'https://alpha.example/', target, size: { rows: 24, cols: 80 }, onOutput: frame => output.push(frame), fetch: gateway.fetch, socket: gateway.factory })
    yield* settle
    expect(gateway.requests[0]!.url).toBe('https://alpha.example/v1/client/terminals/a%2Fb/raw-attachments')
    expect(gateway.requests[0]!.init?.method).toBe('POST')
    expect(JSON.parse(String(gateway.requests[0]!.init?.body))).toEqual({ runtime_incarnation: target.incarnation, mode: 'attach' })
    const first = gateway.sockets[0]!
    expect(first.url).toBe('wss://alpha.example/v1/client/terminals/a%2Fb/raw-stream?incarnation=runtime%2Fnew%20incarnation&mode=attach')
    expect(first.protocols).toEqual(['st3.client.pty.v0', 'st3.cap.cap-1'])
    expect(first.socket.binaryType).toBe('arraybuffer')
    expect(first.sent[0]).toBeInstanceOf(ArrayBuffer)
    expect(read(new Uint8Array(first.sent[0]!))).toEqual([{ _tag: 'Attach', rows: 24, cols: 80 }])
    expect(session.send(bytes)).toBe(false)
    const expected: PtyOutput[] = [{ _tag: 'Geometry', rows: 20, cols: 70 }, { _tag: 'Screen', bytes: Uint8Array.of(27, 99) }, { _tag: 'Data', bytes }]
    const wire = concatenate(expected.map(encodePtyFrame))
    for (let offset = 0; offset < wire.length; offset += 3) first.socket.onmessage?.({ data: wire.slice(offset, offset + 3).buffer })
    first.socket.onmessage?.({ data: 'ignored text' })
    yield* settle
    expect(output).toEqual(expected)
    expect(yield* SubscriptionRef.get(session.state)).toEqual(expected[0] && { _tag: 'Live', rows: 20, cols: 70 })
    expect(session.send(bytes)).toBe(true)
    session.resize({ rows: 30, cols: 90 })
    expect(session.resetInputModes()).toBe(true)
    expect(first.sent.slice(1).flatMap(buffer => read(new Uint8Array(buffer)))).toEqual([{ _tag: 'Data', bytes }, { _tag: 'Resize', rows: 30, cols: 90 }, { _tag: 'ResetInputModes' }])
    first.socket.onclose?.({ code: 1006, reason: 'drop' })
    expect(session.send(bytes)).toBe(false)
    yield* settle
    expect((yield* SubscriptionRef.get(session.state))._tag).toBe('Reconnecting')
    expect(session.send(bytes)).toBe(false)
    session.resize({ rows: 40, cols: 100 })
    yield* Effect.promise(() => vi.advanceTimersByTimeAsync(500))
    yield* settle
    expect(gateway.requests).toHaveLength(2)
    const second = gateway.sockets[1]!
    expect(second.protocols).toEqual(['st3.client.pty.v0', 'st3.cap.cap-2'])
    expect(read(new Uint8Array(second.sent[0]!))).toEqual([{ _tag: 'Attach', rows: 40, cols: 100 }])
    second.socket.onmessage?.({ data: encodePtyFrame({ _tag: 'Geometry', rows: 40, cols: 100 }).buffer })
    yield* settle
  }).pipe(Effect.scoped)
  const second = gateway.sockets[1]!
  expect(read(new Uint8Array(second.sent.at(-1)!))).toEqual([{ _tag: 'Detach' }])
  expect(second.closes).toContain(1000)
  expect((yield* SubscriptionRef.get(session!.state))._tag).toBe('Closed')
  expect(session!.send(bytes)).toBe(false)
  yield* Effect.promise(() => vi.advanceTimersByTimeAsync(60000))
  expect(gateway.requests).toHaveLength(2)
}))

it.effect('decodes attach failures and maps malformed envelopes and network errors', () => Effect.gen(function* () {
  for (const [fetchImpl, retryable, status] of [
    [async () => Response.json({ api_version: 'st3.client.v0', error_version: 'st3.error.v0', code: 'forbidden', message: 'Denied', retryable: false }, { status: 403 }), false, 403],
    [async () => Response.json({ value: {} }), false, 200],
    [async () => { throw new TypeError('offline') }, true, undefined],
  ] as const) {
    const result = yield* Effect.result(requestPtyAttachment({ baseUrl: 'http://alpha.example', target, fetch: fetchImpl }))
    expect(Result.isFailure(result)).toBe(true)
    if (Result.isFailure(result)) { expect(result.failure.retryable).toBe(retryable); expect(result.failure.status).toBe(status) }
  }
  expect(ptyStreamUrl({ baseUrl: 'http://alpha.example', target }).startsWith('ws://')).toBe(true)
}))
