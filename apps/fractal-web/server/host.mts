import type { IncomingMessage } from 'node:http'
import { readFileSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { Tracer } from 'effect'
import { createFractalWebServer, type FractalWebServer, type AdmissionIdentity } from './core.mts'
import { checkPublisherAuthorization, checkTagAuthorization } from './tagAuthorization.mts'

/**
 * Loopback trust boundary: only the identity-stripping published-surface filter is a legitimate
 * caller. Local processes can forge forwarded identity; never expose this listener directly.
 */
export interface TailnetAdmissionOptions {
  /** The exact published HTTPS origin. */
  readonly origin: string
  /** Tailnet login names allowed to act as the paired person. */
  readonly logins: readonly string[]
  readonly tags?: readonly string[]
  /** The loopback listener's own `host:port`, once bound. */
  readonly listener: () => string | undefined
}

const safeMethods = new Set(['GET', 'HEAD'])

export const tailnetAdmissionDecision = ({ origin, logins, tags = [], listener }: TailnetAdmissionOptions) => {
  const published = new URL(origin)
  if (published.origin !== origin || published.protocol !== 'https:') throw new TypeError('The published origin must be an exact https origin')
  if (logins.length === 0) throw new TypeError('At least one tailnet login is required')
  if (tags.some(tag => !/^tag:[^,\s]+$/.test(tag))) throw new TypeError('Tailnet tags must use the tag: prefix')
  return (request: IncomingMessage): AdmissionIdentity | false => {
    const login = request.headers['tailscale-user-login']
    const nodeTags = request.headers['tailscale-node-tags']
    // The filter emits a login OR tags, never both. Ambiguous identities fail closed.
    if (login !== undefined && nodeTags !== undefined) return false
    let identity: AdmissionIdentity
    if (typeof login === 'string' && !login.startsWith('tag:') && logins.includes(login)) {
      identity = { kind: 'login', value: login }
    } else if (login === undefined && typeof nodeTags === 'string') {
      const forwarded = nodeTags.split(',').map(tag => tag.trim())
      if (forwarded.some(tag => !/^tag:[^,\s]+$/.test(tag))) return false
      const tag = forwarded.find(tag => tags.includes(tag))
      if (tag === undefined) return false
      identity = { kind: 'tag', value: tag }
    } else return false
    const host = request.headers.host
    if (host !== published.host && (host === undefined || host !== listener())) return false
    const requestOrigin = request.headers.origin
    // Browsers send Origin on every upgrade and every unsafe method; only plain reads may omit it,
    // and a browser that says a read came from another site is refused even without an Origin.
    if (requestOrigin === undefined) {
      const site = request.headers['sec-fetch-site']
      return request.headers.upgrade === undefined && safeMethods.has(request.method ?? 'GET') &&
        (site === undefined || site === 'same-origin' || site === 'none') ? identity : false
    }
    return requestOrigin === origin ? identity : false
  }
}

/** Person-only predicate for pairing and existing edge integrations; never erases tag identity. */
export const tailnetAdmission = (options: Omit<TailnetAdmissionOptions, 'tags'>) => {
  const decide = tailnetAdmissionDecision({ ...options, tags: [] })
  return (request: IncomingMessage): boolean => decide(request) !== false
}

export interface EdgeOrTailnetAdmissionOptions {
  readonly origin: string | undefined
  readonly logins: readonly string[]
  /** The deployment's unchanged Devnet edge and direct-loopback admission. */
  readonly edge: (request: IncomingMessage) => boolean
}

/**
 * Adds a canonical tailnet origin without weakening an existing Devnet edge policy.
 *
 * Serve's HTTP proxy preserves Host and overwrites X-Forwarded-Host with that same Host, including
 * for tagged nodes without identity headers. Devnet instead sends the listener Host and a distinct
 * external X-Forwarded-Host. Do not let Serve traffic fall through to the login-free edge policy.
 * The listener must remain loopback-only: local processes can forge either proxy's headers.
 */
export const edgeOrTailnetAdmission = ({ origin, logins, edge }: EdgeOrTailnetAdmissionOptions) => {
  if (origin === undefined) return edge
  // Validate eagerly, including the nonempty allowlist, before opening the listener.
  const tailnet = tailnetAdmission({ origin, logins, listener: () => undefined })
  const host = new URL(origin).host
  return (request: IncomingMessage): boolean => {
    if (request.headers.host === host) return tailnet(request)
    if (request.headers['x-forwarded-host'] !== undefined &&
      request.headers['x-forwarded-host'] === request.headers.host) return false
    if (Object.keys(request.headers).some((name) => name.startsWith('tailscale-'))) return false
    return edge(request)
  }
}

export interface HostConfig {
  readonly dist: string
  readonly host: string
  readonly port: number
  readonly origin: string
  readonly logins: readonly string[]
  readonly tags: readonly string[]
  readonly socketPath: string
  readonly authorization: string
  readonly tagAuthorization: string | undefined
  readonly deploymentId: string
}

const loopbackHosts = new Set(['127.0.0.1', '::1', 'localhost'])

export interface SecretFiles {
  readonly readText: (path: string) => string
  /** The file's permission bits and owner. */
  readonly owner: (path: string) => { readonly mode: number; readonly uid: number }
}
const localFiles: SecretFiles = { readText: (path) => readFileSync(path, 'utf8'), owner: (path) => statSync(path) }

/**
 * Reads the host's environment. The bearer comes from a file so it never sits in the process
 * environment, and that file must belong to this user and be unreadable by anyone else.
 */
export const readHostConfig = (env: NodeJS.ProcessEnv, files: SecretFiles = localFiles, uid: number | undefined = process.getuid?.()): HostConfig => {
  const required = (name: string): string => {
    const value = env[name]?.trim()
    if (!value) throw new TypeError(`${name} is required`)
    return value
  }
  const host = env.WF_HOST?.trim() || '127.0.0.1'
  if (!loopbackHosts.has(host)) throw new TypeError('WF_HOST must be a loopback address; publish it with tailscale serve')
  const port = Number(required('WF_PORT'))
  if (!Number.isSafeInteger(port) || port <= 0 || port > 65535) throw new TypeError('WF_PORT must be a TCP port')
  const origin = required('WF_PUBLIC_ORIGIN')
  const logins = required('WF_TAILNET_LOGINS').split(',').map((login) => login.trim()).filter((login) => login !== '')
  if (logins.length === 0) throw new TypeError('WF_TAILNET_LOGINS must name at least one tailnet login')
  const tags = (env.WF_TAILNET_TAGS ?? '').split(',').map(tag => tag.trim()).filter(tag => tag !== '')
  if (tags.some(tag => !/^tag:[^,\s]+$/.test(tag))) throw new TypeError('WF_TAILNET_TAGS must name tag: prefixed tags')
  const readAuthorization = (name: string): string => {
    const path = required(name)
    const { mode, uid: fileOwner } = files.owner(path)
    if ((mode & 0o077) !== 0 || (uid !== undefined && fileOwner !== uid))
      throw new TypeError(`${name} must be owned by this user and readable by no one else (chmod 600)`)
    const token = files.readText(path).trim()
    if (token === '' || /\s/.test(token.replace(/^Bearer /, ''))) throw new TypeError(`${name} must hold one bearer token`)
    return token.startsWith('Bearer ') ? token : `Bearer ${token}`
  }
  const authorization = readAuthorization('WF_ST_AUTHORIZATION_FILE')
  const tagAuthorization = tags.length === 0 ? undefined : readAuthorization('WF_TAG_AUTHORIZATION_FILE')
  if (tagAuthorization !== undefined && tagAuthorization === authorization)
    throw new TypeError('WF_TAG_AUTHORIZATION_FILE must hold a dedicated credential, not the person credential')
  return {
    dist: resolve(env.WF_DIST?.trim() || 'dist/web'),
    host,
    port,
    origin,
    logins,
    tags,
    socketPath: required('WF_ST_GATEWAY').replace(/^unix:/, ''),
    authorization,
    tagAuthorization,
    deploymentId: env.WF_DEPLOYMENT_ID?.trim() || new URL(origin).host,
  }
}

export const startHost = async (config: HostConfig, readText: (path: string) => string = (path) => readFileSync(path, 'utf8')): Promise<FractalWebServer> => {
  const gateway = { socketPath: config.socketPath, host: '127.0.0.1', authorization: config.authorization, tagAuthorization: config.tagAuthorization, timeoutMs: 30_000 }
  if (config.tags.length > 0) await checkTagAuthorization(gateway)
  await checkPublisherAuthorization(gateway)
  const buildIdentity: unknown = JSON.parse(readText(join(config.dist, 'build-identity.json')))
  let listening: string | undefined
  const served = createFractalWebServer({
    dist: config.dist,
    host: config.host,
    port: config.port,
    admit: tailnetAdmissionDecision({ origin: config.origin, logins: config.logins, tags: config.tags, listener: () => listening }),
    pairing: { admit: tailnetAdmission({ origin: config.origin, logins: config.logins, listener: () => listening }) },
    tracer: Tracer.make({ span: (options) => new Tracer.NativeSpan(options) }),
    gateway,
    identity: { deploymentId: config.deploymentId, buildIdentity: buildIdentity as never },
  })
  served.server.once('listening', () => {
    const address = served.server.address()
    if (address !== null && typeof address === 'object') listening = `${address.family === 'IPv6' ? `[${address.address}]` : address.address}:${address.port}`
  })
  return served
}
