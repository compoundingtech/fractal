import { request } from 'node:http'
import type { RequestOptions, ServerResponse } from 'node:http'

/** HTTP plumbing shared by the optional same-origin relays that read a configured loopback producer. */

/** A configured loopback producer a relay may read. */
export interface LoopbackProducer {
  readonly hostname: '127.0.0.1' | '::1'
  readonly port: number
}

/** Parse an optional producer base URL from setting `name`; unset or empty means the relay is off. */
export const loopbackProducer = (value: string | undefined, name: string): LoopbackProducer | undefined => {
  const text = value?.trim()
  if (!text) return undefined
  let url: URL
  try { url = new URL(text) } catch { throw new TypeError(`${name} must be a URL`) }
  const hostname = url.hostname === '[::1]' ? '::1' : url.hostname
  // URL drops an explicit default port (`:80`), so the port must be read from the raw authority.
  const explicitPort = /^[a-z][a-z0-9+.-]*:\/\/[^/?#]*:(\d+)(?:[/?#]|$)/i.exec(text)?.[1] !== undefined
  const port = url.port === '' ? 80 : Number(url.port)
  if (url.protocol !== 'http:' || (hostname !== '127.0.0.1' && hostname !== '::1') || !explicitPort || port < 1 || port > 65535 ||
      url.username !== '' || url.password !== '' || url.pathname !== '/' || url.search !== '' || url.hash !== '')
    throw new TypeError(`${name} must be a loopback base URL such as http://127.0.0.1:PORT, with a nonzero port and no credentials, path or query`)
  return { hostname, port }
}

/** The read did not finish within its elapsed deadline. */
export class ReadDeadline extends Error {}
/** The answer exceeded its byte limit. */
export class ReadTooLarge extends Error {}

/**
 * One GET that must finish within `deadlineMs` of elapsed time and `maxBytes` of body; any status.
 * Elapsed, not idle: a peer that keeps trickling bytes cannot hold the caller's slot.
 */
export const boundedGet = (options: RequestOptions, deadlineMs: number, maxBytes: number): Promise<{ readonly status: number; readonly body: string }> => {
  const { promise, resolve, reject } = Promise.withResolvers<{ readonly status: number; readonly body: string }>()
  const upstream = request({ ...options, method: 'GET', agent: false }, (response) => {
    const chunks: Buffer[] = []
    let size = 0
    response.on('error', reject)
    response.on('aborted', () => reject(new Error('response aborted')))
    response.on('data', (chunk: Buffer) => {
      size += chunk.length
      if (size > maxBytes) { reject(new ReadTooLarge('response too large')); response.destroy(); return }
      chunks.push(chunk)
    })
    response.on('end', () => resolve({ status: response.statusCode ?? 0, body: Buffer.concat(chunks).toString('utf8') }))
  })
  upstream.on('error', reject)
  const deadline = setTimeout(() => {
    reject(new ReadDeadline('deadline'))
    upstream.destroy()
  }, deadlineMs)
  upstream.end()
  return promise.finally(() => clearTimeout(deadline))
}

/** A producer GET without credentials that requires 200 and a JSON body, and returns that body verbatim. */
export const producerJson = async (producer: LoopbackProducer, path: string, deadlineMs: number, maxBytes = 2 * 1024 * 1024): Promise<string> => {
  const { status, body } = await boundedGet({ hostname: producer.hostname, port: producer.port, path, headers: { accept: 'application/json' } }, deadlineMs, maxBytes)
  if (status !== 200) throw new Error(`HTTP ${status}`)
  try { JSON.parse(body) } catch { throw new Error('response is not JSON') }
  return body
}

/** Too many distinct loads are already in flight; nothing was requested. */
export class LoadsBusy extends Error {}

/**
 * Bounded insertion-ordered TTL cache: at most `entries` stored values and `entries` distinct
 * in-flight loads. Concurrent loads of one key share work; failures are never stored.
 */
export const ttlCache = <T,>(ttlMs: number, entries: number) => {
  const cache = new Map<string, { value: T; expiresAt: number }>()
  const pending = new Map<string, Promise<T>>()
  return (key: string, load: () => Promise<T>): Promise<T> => {
    const cached = cache.get(key)
    if (cached !== undefined && performance.now() < cached.expiresAt) return Promise.resolve(cached.value)
    const current = pending.get(key)
    if (current !== undefined) return current
    if (pending.size >= entries) return Promise.reject(new LoadsBusy('too many producer reads in flight'))
    const loading = load().then((value) => {
      cache.delete(key)
      cache.set(key, { value, expiresAt: performance.now() + ttlMs })
      for (const stale of cache.keys()) {
        if (cache.size <= entries) break
        cache.delete(stale)
      }
      return value
    }).finally(() => pending.delete(key))
    pending.set(key, loading)
    return loading
  }
}

export const json = (res: ServerResponse, status: number, body: string): void => {
  if (res.destroyed) return
  res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' })
  res.end(body)
}
export const failure = (res: ServerResponse, status: number, code: string, message: string, reason?: string): void =>
  json(res, status, JSON.stringify(reason === undefined ? { code, message } : { code, message, reason }))
export const reasonOf = (error: unknown): string => error instanceof Error ? error.message : 'failed'
