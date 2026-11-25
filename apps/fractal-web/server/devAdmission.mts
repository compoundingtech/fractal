import type { IncomingMessage } from 'node:http'

const loopback = ['127.0.0.1', 'localhost']
const loopbackPeers = new Set(['127.0.0.1', '::1', '::ffff:127.0.0.1'])
const singleHeaders = new Set(['host', 'origin', 'sec-fetch-site', 'x-forwarded-proto', 'x-forwarded-host'])
const forwardedAllowed = new Set(['x-forwarded-for', 'x-forwarded-proto', 'x-forwarded-host'])

export interface DevAdmissionOptions {
  /** The actual listener port, undefined until bound. */
  readonly port: () => number | undefined
  readonly edgeHost?: string
  readonly edgeAliases?: readonly string[]
}

/**
 * Admission for a loopback-only development listener, optionally behind an HTTPS edge proxy.
 *
 * Local processes can forge proxy headers; this does not authenticate the proxy. Edge requests
 * must name an explicitly configured authority and the actual IPv4 listener Host. An upgrade
 * needs the exact edge Origin or the listener Origin normalized by the trusted edge proxy.
 * Direct requests retain the loopback Origin policy. Neither path accepts caller credentials.
 */
export const devAdmission = ({ port, edgeHost, edgeAliases = [] }: DevAdmissionOptions) => {
  const edgeHosts = new Set(edgeHost === undefined ? edgeAliases : [edgeHost, ...edgeAliases])
  return (request: IncomingMessage): boolean => {
    const seen = new Set<string>()
    for (let index = 0; index < request.rawHeaders.length; index += 2) {
      const name = request.rawHeaders[index]!.toLowerCase()
      if (!singleHeaders.has(name)) continue
      if (seen.has(name)) return false
      seen.add(name)
    }
    if (request.headers.authorization !== undefined || request.headers['proxy-authorization'] !== undefined) return false
    for (const name of Object.keys(request.headers)) {
      if (name === 'forwarded' || (name.startsWith('x-forwarded-') && !forwardedAllowed.has(name))) return false
    }
    if (request.url === undefined || !request.url.startsWith('/') || request.url.startsWith('//')) return false
    if (!loopbackPeers.has(request.socket.remoteAddress ?? '')) return false
    const site = request.headers['sec-fetch-site']
    if (site !== undefined && site !== 'same-origin' && site !== 'none') return false
    const listening = port()
    if (listening === undefined) return false
    const forwardedHost = request.headers['x-forwarded-host']
    const forwardedProto = request.headers['x-forwarded-proto']
    const origin = request.headers.origin
    if (forwardedHost !== undefined) {
      if (typeof forwardedHost !== 'string' || !edgeHosts.has(forwardedHost) || forwardedProto !== 'https') return false
      if (request.headers.host !== `127.0.0.1:${listening}`) return false
      if (origin === undefined) return request.headers.upgrade === undefined
      return origin === `https://${forwardedHost}` ||
        (request.headers.upgrade !== undefined && origin === `http://127.0.0.1:${listening}`)
    }
    if (forwardedProto !== undefined || !loopback.some(name => request.headers.host === `${name}:${listening}`)) return false
    if (origin === undefined) return request.headers.upgrade === undefined
    return loopback.some(name => origin === `http://${name}:${listening}`)
  }
}
