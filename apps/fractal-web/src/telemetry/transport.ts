import type { ConnectionState, FractalOptions } from '@fractal/sdk/read'
import { wrapTraceFetch } from '../data/sdk-browser/trace.ts'

import { getDebug, incrDebug, setDebug } from './meters.tsx'

/** One finished request, by route only: query strings can carry identifiers. */
export interface RequestRecord {
  readonly method: string
  readonly path: string
  readonly outcome: number | 'network'
  readonly durationMs: number
}
const requestLogSize = 8
/** What the error count and latency tails are made of, newest errors first and slowest responses first. */
export interface RequestLog {
  readonly errors: readonly RequestRecord[]
  readonly slowest: readonly RequestRecord[]
}
const emptyRequestLog: RequestLog = { errors: [], slowest: [] }
let requestLog = emptyRequestLog
const requestLogListeners = new Set<() => void>()
const publishRequestLog = (next: RequestLog): void => {
  requestLog = next
  for (const notify of requestLogListeners) notify()
}

/** An immutable snapshot, replaced only when a request changes it, so readers can compare by identity. */
export const readRequestLog = (): RequestLog => requestLog
export const subscribeRequestLog = (notify: () => void): (() => void) => {
  requestLogListeners.add(notify)
  return () => requestLogListeners.delete(notify)
}

const requestRecord = (
  input: Parameters<typeof globalThis.fetch>[0],
  init: Parameters<typeof globalThis.fetch>[1],
  outcome: RequestRecord['outcome'],
  durationMs: number,
): RequestRecord => {
  const request = input instanceof Request ? input : undefined
  const url = request?.url ?? String(input)
  return {
    method: init?.method ?? request?.method ?? 'GET',
    path: URL.canParse(url) ? new URL(url).pathname : url.split('?')[0]!,
    outcome,
    durationMs,
  }
}
const logError = (record: RequestRecord): void => {
  publishRequestLog({ ...requestLog, errors: [record, ...requestLog.errors].slice(0, requestLogSize) })
}
const logSlowest = (record: RequestRecord): void => {
  const { slowest } = requestLog
  if (slowest.length === requestLogSize && record.durationMs <= slowest[requestLogSize - 1]!.durationMs) return
  publishRequestLog({
    ...requestLog,
    slowest: [...slowest, record].toSorted((a, b) => b.durationMs - a.durationMs).slice(0, requestLogSize),
  })
}

/** Observe fetches and propagate UX context only to the admitted same-origin data boundary. */
export const instrumentFetch = ({ fetchImpl, origin, traceContext }: {
  readonly fetchImpl: typeof globalThis.fetch
  readonly origin: string
  readonly traceContext?: FractalOptions['traceContext']
}): typeof globalThis.fetch => {
  const fetch = traceContext === undefined ? fetchImpl : wrapTraceFetch({ fetchImpl, traceContext, baseUrl: origin })
  const samples: number[] = []
  let cursor = 0
  let inFlight = 0
  // Absent keys read as 0, so meters need these to tell "measured zero" from "not measured".
  setDebug('Wf.httpInstrumented', 1)
  setDebug('Wf.httpSamples', 0)
  setDebug('Wf.httpInFlight', 0)
  setDebug('Wf.httpErrors', 0)
  setDebug('Wf.httpP50Ms', 0)
  setDebug('Wf.httpP95Ms', 0)
  publishRequestLog(emptyRequestLog)
  return async (...[input, init]) => {
    const started = performance.now()
    setDebug('Wf.httpInFlight', ++inFlight)
    try {
      const response = await fetch(input, init)
      if (!response.ok) {
        incrDebug('Wf.httpErrors')
        logError(requestRecord(input, init, response.status, performance.now() - started))
      }
      logSlowest(requestRecord(input, init, response.status, performance.now() - started))
      // Only a response is latency: cancellation and network failure time something else.
      samples[cursor] = performance.now() - started
      cursor = (cursor + 1) % 256
      const sorted = samples.toSorted((a, b) => a - b)
      setDebug('Wf.httpSamples', sorted.length)
      setDebug('Wf.httpP50Ms', sorted[Math.ceil(sorted.length * 0.5) - 1]!)
      setDebug('Wf.httpP95Ms', sorted[Math.ceil(sorted.length * 0.95) - 1]!)
      return response
    } catch (error) {
      // User cancellation is not a failed transport request.
      if (!(error instanceof Error && error.name === 'AbortError')) {
        incrDebug('Wf.httpErrors')
        logError(requestRecord(input, init, 'network', performance.now() - started))
      }
      throw error
    } finally {
      setDebug('Wf.httpInFlight', --inFlight)
    }
  }
}

/** The latest SDK connection state; numeric debug counters cannot carry its attempt or issue. */
export interface SocketObservation {
  readonly state: ConnectionState
  readonly sinceMs: number
}
let socketObservation: SocketObservation | undefined
const socketListeners = new Set<() => void>()

/** Undefined means no live source has reported yet, which is not the same as being down. */
export const readSocketObservation = (): SocketObservation | undefined => socketObservation
export const subscribeSocketObservation = (notify: () => void): (() => void) => {
  socketListeners.add(notify)
  return () => socketListeners.delete(notify)
}
export const recordSocketState = (state: ConnectionState): void => {
  socketObservation = { state, sinceMs: Date.now() }
  for (const notify of socketListeners) notify()
}
