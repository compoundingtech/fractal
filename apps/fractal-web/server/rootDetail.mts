import type { IncomingMessage, ServerResponse } from 'node:http'
import * as Native from '@fractal/client/schema/read'
import { Option, Schema, SchemaIssue, type SchemaAST } from 'effect'
import { gatewayCredential, type GatewayOptions } from './gateway.mts'
import { boundedGet, failure, json, LoadsBusy, ReadDeadline, ReadTooLarge, ttlCache, type LoopbackProducer } from './relayHttp.mts'
import { admittedNativeApiVersions, nativeEnvelope as envelope } from './nativeEnvelope.mts'
import { NativeCapabilities, readCapabilities } from './tagAuthorization.mts'

/**
 * Descendant-inclusive spend of one agent's current native root, for the Details pane.
 *
 * Answers only `GET /v1/client/usage/root-detail?agent=<id>&window=1d|7d`, and only when the host
 * configures a loopback producer serving `/api/v1/lens/root_detail`. The browser names an agent,
 * never a root: the relay derives the root itself from the gateway, with the credential the
 * gateway would forward for this request (`gatewayCredential`), so it can only describe a root
 * the caller may already see.
 *
 * Each request, before any producer read or cache lookup:
 * - the credential must have `read.projections` granted, the scope every native read below needs;
 * - the agent must run the `omp` harness, report its host and incarnation, and bind exactly one
 *   full-UUID native root for that incarnation, from a fresh todo snapshot or a completed
 *   suspension;
 * - the gateway's own usage rows for the last 7 days must pair that agent with that root.
 * The same checks run again after the producer read, so a grant revoked or a root replaced
 * meanwhile gets a refusal, never the old answer.
 *
 * Producer reads carry no credentials, finish within 30 s elapsed and 256 KiB, are cached per
 * (verified root, window) for 30 s, at most 64 entries and 64 distinct reads in flight. Failures
 * are never cached. The answer is an allowlist: root total, descendant count, the first 20
 * descendants (of up to 200 reported), attribution share and durations; nothing else crosses.
 */
export const rootDetailRoute = '/v1/client/usage/root-detail'
export const isRootDetailRequest = (url: string): boolean => url === rootDetailRoute || url.startsWith(`${rootDetailRoute}?`)

export type RootDetailWindow = '1d' | '7d'
const windows: readonly RootDetailWindow[] = ['1d', '7d']
/** Native usage that may corroborate a root binding, independent of the window shown. */
const corroborationMs = 7 * 86_400_000
export const maxRenderedChildren = 20

class Refusal extends Error {
  readonly status: number
  readonly code: string
  constructor(status: number, code: string, message: string) {
    super(message)
    this.status = status
    this.code = code
  }
}
const notAttributed = (message: string): never => { throw new Refusal(422, 'root-detail-not-attributed', message) }

const Text = Schema.NonEmptyString.check(Schema.isMaxLength(512))
const Amount = Schema.Finite.check(Schema.isGreaterThanOrEqualTo(0))
const Count = Schema.Natural.check(Schema.isLessThanOrEqualTo(Number.MAX_SAFE_INTEGER))
const optional = <S extends Schema.Top>(schema: S) => Schema.optionalKey(Schema.NullOr(schema))
const RootSessionId = Schema.String.check(Schema.isUUID())

/** The producer's root_detail lens; unlisted fields (worktrees, seats, prompts, hourly series) are dropped. */
const ProducerRootDetail = Schema.Struct({
  key: Schema.Literal('root_detail'),
  root_session_id: Text,
  window: Schema.Literals(['1d', '7d']),
  message: optional(Text),
  host: optional(Text),
  usd: optional(Amount),
  child_count: optional(Count),
  wall_hours: optional(Amount),
  active_hours: optional(Amount),
  coverage: optional(Schema.Struct({ account_share: optional(Schema.Finite.check(Schema.isBetween({ minimum: 0, maximum: 1 }))) })),
  children: Schema.optionalKey(Schema.Array(Schema.Struct({ session_id: Text, role: optional(Text), model: optional(Text), usd: Amount, requests: Count })).check(Schema.isMaxLength(200))),
})
/** Only schema-owned names enter the diagnostic; record keys and array indices are redacted. */
const failingField = (issue: SchemaIssue.Issue, ast: SchemaAST.AST, path = '$'): string => {
  switch (issue._tag) {
    case 'Composite': return failingField(issue.issues[0], issue.ast, path)
    case 'Pointer': {
      for (const key of issue.path) {
        const field = ast._tag === 'Objects' ? ast.propertySignatures.find(field => field.name === key) : undefined
        path += field === undefined ? '[*]' : `.${String(field.name)}`
        if (field !== undefined) ast = field.type
        else if (ast._tag === 'Arrays') ast = ast.rest[0] ?? ast.elements[0] ?? ast
      }
      return failingField(issue.issue, ast, path)
    }
    case 'Encoding': return failingField(issue.issue, issue.ast, path)
    case 'Filter': return failingField(issue.issue, ast, path)
    case 'AnyOf':
      if (issue.issues[0] !== undefined) return failingField(issue.issues[0], ast, path)
      return `${path}: ${SchemaIssue.defaultLeafHook(new SchemaIssue.InvalidType(ast)).replace(/^Expected /, 'expected ')}`
    default: return `${path}: ${SchemaIssue.defaultLeafHook(new SchemaIssue.InvalidType('ast' in issue ? issue.ast : ast)).replace(/^Expected /, 'expected ')}`
  }
}
const decode = <T,>(schema: Schema.Decoder<T>, body: string, code: string): T => {
  const native = code === 'root-detail-gateway-unavailable'
  let input: unknown
  try { input = JSON.parse(body) }
  catch {
    const context = native
      ? `native envelope; api_version=unavailable; admitted=${admittedNativeApiVersions.join(',')}`
      : 'producer lens; lens_version=unavailable'
    throw new Refusal(502, code, `${context}; $: expected JSON object`)
  }
  const version = input !== null && typeof input === 'object'
    ? Reflect.get(input, native ? 'api_version' : 'lens_version') : undefined
  const identifier = native
    ? typeof version === 'string' && version.length <= 128 && /^[a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)*\.v[0-9]+$/.test(version) ? version : 'unavailable'
    : typeof version === 'number' && Number.isSafeInteger(version) ? String(version) : 'unavailable'
  const context = native ? `native envelope; api_version=${identifier}; admitted=${admittedNativeApiVersions.join(',')}` : `producer lens; lens_version=${identifier}`
  try { return Schema.decodeUnknownSync(schema, { reportInput: false })(input) }
  catch (error) {
    if (!Schema.isSchemaError(error)) throw error
    throw new Refusal(502, code, `${context}; ${failingField(error.issue, schema.ast)}`)
  }
}

interface RootIdentity { readonly agent: string; readonly incarnationId: string; readonly host: string; readonly nativeSessionId: string }
const sameIdentity = (left: RootIdentity, right: RootIdentity): boolean =>
  left.agent === right.agent && left.incarnationId === right.incarnationId && left.host === right.host && left.nativeSessionId === right.nativeSessionId

/** The exact query: one canonical agent id and one supported window, nothing else. */
const requested = (url: string): { readonly agent: string; readonly window: RootDetailWindow } | undefined => {
  if (!url.startsWith(`${rootDetailRoute}?`)) return undefined
  const params = new URLSearchParams(url.slice(rootDetailRoute.length + 1))
  const keys = [...params.keys()]
  const agent = params.get('agent')
  const window = windows.find(value => value === params.get('window'))
  if (keys.length !== 2 || !keys.includes('agent') || window === undefined || agent === null || agent.length > 512 || !Schema.is(Native.AgentId)(agent) ||
      agent.split('/').some(part => part === '' || part === '.' || part === '..' || /[%\\?#\p{Cc}\p{Cf}]/u.test(part))) return undefined
  return { agent, window }
}

/** The one current-incarnation root binding of an `omp` agent, or a refusal naming what is missing. */
const rootBinding = (agentRef: string, agent: Native.Agent): RootIdentity => {
  if (agent.id !== agentRef) notAttributed('The gateway answered for another agent')
  if (Option.getOrUndefined(agent.driver) !== 'omp') throw new Refusal(422, 'root-detail-not-covered', 'Descendant spend is reported only for the omp harness')
  const incarnationId = Option.getOrUndefined(agent.incarnation_id)
  const host = Option.getOrUndefined(agent.host_id)
  if (host === undefined) return notAttributed('The agent does not report its host')
  if (incarnationId === undefined || incarnationId.length === 0) return notAttributed('The agent does not report its current incarnation')
  const bindings = new Set<string>()
  const todo = Option.getOrUndefined(agent.todo)
  if (todo !== undefined && !todo.stale && todo.snapshot.incarnation_id === incarnationId && todo.snapshot.harness === 'omp' && todo.snapshot.session_id.trim().length > 0)
    bindings.add(todo.snapshot.session_id)
  const suspension = Option.getOrUndefined(agent.suspension)
  const suspended = suspension === undefined ? undefined : Option.getOrUndefined(suspension.native_session_id)
  if (suspension !== undefined && Option.getOrUndefined(suspension.incarnation_id) === incarnationId && Option.getOrUndefined(suspension.harness) === 'omp' &&
      (suspension.phase === 'suspended' || suspension.phase === 'resumed') && suspended !== undefined && suspended.trim().length > 0) bindings.add(suspended)
  const [nativeSessionId] = bindings
  if (bindings.size !== 1 || nativeSessionId === undefined) return notAttributed('The agent binds no single native root for its current incarnation')
  if (!Schema.is(RootSessionId)(nativeSessionId)) return notAttributed('The native root is not a full session id')
  return { agent: agentRef, incarnationId, host, nativeSessionId }
}

export interface RootDetailRelay {
  /** Answer an admitted request for `rootDetailRoute`; `preview` marks tag admission. */
  handle(req: IncomingMessage, res: ServerResponse, preview: boolean): Promise<void>
}

export const createRootDetailRelay = ({ producer, gateway, producerDeadlineMs = 30_000, now = Date.now }: {
  readonly producer: LoopbackProducer
  readonly gateway: GatewayOptions
  /** Elapsed deadline for each producer read; a cold producer can take several seconds. */
  readonly producerDeadlineMs?: number
  readonly now?: () => number
}): RootDetailRelay => {
  const details = ttlCache<string>(30_000, 64)
  const read = async (credential: string, path: string): Promise<string> => {
    let answer: { readonly status: number; readonly body: string }
    try {
      answer = await boundedGet({ socketPath: gateway.socketPath, path, headers: { host: gateway.host, authorization: credential, accept: 'application/json' } }, gateway.timeoutMs, 2 * 1024 * 1024)
    } catch {
      throw new Refusal(502, 'root-detail-gateway-unavailable', 'The gateway could not verify the agent')
    }
    if (answer.status === 401 || answer.status === 403) throw new Refusal(403, 'forbidden', 'Forbidden')
    if (answer.status !== 200) throw new Refusal(502, 'root-detail-gateway-unavailable', 'The gateway could not verify the agent')
    return answer.body
  }
  /** Grant, binding and corroboration, read fresh with the caller's credential. */
  const verify = async (credential: string, agentRef: string): Promise<RootIdentity> => {
    let grant: { readonly status: number; readonly body: string }
    try { grant = await readCapabilities(gateway, credential, 'Root detail') }
    catch { throw new Refusal(502, 'root-detail-gateway-unavailable', 'The gateway could not verify the agent') }
    if (grant.status === 401 || grant.status === 403) throw new Refusal(403, 'forbidden', 'Forbidden')
    if (grant.status !== 200) throw new Refusal(502, 'root-detail-gateway-unavailable', 'The gateway could not verify the agent')
    const granted = decode(NativeCapabilities, grant.body, 'root-detail-gateway-unavailable').value.capabilities
      .some(capability => capability.id === 'read.projections' && capability.state === 'granted')
    if (!granted) throw new Refusal(403, 'forbidden', 'Forbidden')
    const agent = decode(envelope(Native.Agent), await read(credential, `/v1/client/agents/${encodeURIComponent(agentRef)}`), 'root-detail-gateway-unavailable').value
    const identity = rootBinding(agentRef, agent)
    const until = now()
    const period = decode(envelope(Native.UsagePeriod), await read(credential, `/v1/client/usage?${new URLSearchParams({ since_ms: String(Math.max(0, until - corroborationMs)), until_ms: String(until) })}`), 'root-detail-gateway-unavailable').value
    if (!period.rows.some(row => row.agent === agentRef && row.native_session_id === identity.nativeSessionId))
      notAttributed('Native usage does not pair this agent with its root')
    return identity
  }
  const load = async (identity: RootIdentity, window: RootDetailWindow): Promise<string> => {
    let answer: { readonly status: number; readonly body: string }
    try {
      answer = await boundedGet({ hostname: producer.hostname, port: producer.port, path: `/api/v1/lens/root_detail?${new URLSearchParams({ root: identity.nativeSessionId, window })}`, headers: { accept: 'application/json' } }, producerDeadlineMs, 256 * 1024)
    } catch (error) {
      if (error instanceof ReadDeadline) throw new Refusal(502, 'root-detail-deadline', 'The root-detail producer did not answer in time')
      if (error instanceof ReadTooLarge) throw new Refusal(502, 'root-detail-oversized', 'The root-detail producer answer is too large')
      throw new Refusal(502, 'root-detail-producer-unavailable', 'The root-detail producer is unavailable')
    }
    if (answer.status !== 200) throw new Refusal(502, 'root-detail-producer-unavailable', 'The root-detail producer is unavailable')
    const detail = decode(ProducerRootDetail, answer.body, 'root-detail-incompatible')
    if (detail.root_session_id !== identity.nativeSessionId) throw new Refusal(502, 'root-detail-wrong-root', 'The producer answered for another root')
    if (detail.window !== window) throw new Refusal(502, 'root-detail-incompatible', 'The producer answered for another window')
    // A canonical host reference must name the native owner; any other host text is the ledger's own label.
    if (detail.host?.startsWith('host/') === true && detail.host !== identity.host) throw new Refusal(502, 'root-detail-wrong-host', 'The producer reported a different native owner')
    // A no-request answer carries guidance, not money: never a reported zero.
    if (detail.usd == null && detail.message != null) throw new Refusal(404, 'root-detail-not-found', 'The root has no requests in this window, or the producer has not caught up')
    if (detail.usd == null) throw new Refusal(502, 'root-detail-incompatible', 'The producer omitted the root total')
    const children = detail.children
    if (children !== undefined && (detail.child_count == null || children.length > detail.child_count ||
        new Set(children.map(child => child.session_id)).size !== children.length || children.some(child => child.session_id === identity.nativeSessionId)))
      throw new Refusal(502, 'root-detail-incompatible', 'The producer descendant list is inconsistent')
    return JSON.stringify({
      ...identity, window, inclusiveUsd: detail.usd, childCount: detail.child_count,
      children: children?.slice(0, maxRenderedChildren).map(child => ({ sessionId: child.session_id, role: child.role, model: child.model, usd: child.usd, requests: child.requests })),
      attributedFraction: detail.coverage?.account_share, wallHours: detail.wall_hours, activeHours: detail.active_hours, producerHost: detail.host,
    })
  }
  return {
    handle: async (req, res, preview) => {
      if (req.method !== 'GET') return failure(res, 405, 'method-not-allowed', 'Root detail requires GET')
      const query = requested(req.url ?? '')
      if (query === undefined) return failure(res, 400, 'invalid-query', 'Root detail requires exactly one agent and window=1d or window=7d')
      const credential = gatewayCredential(req.headers, gateway, preview)
      if (credential === undefined) return failure(res, preview ? 403 : 401, preview ? 'forbidden' : 'pairing-required', preview ? 'Forbidden' : 'Browser pairing required')
      try {
        const identity = await verify(credential, query.agent)
        const body = await details(`${JSON.stringify(identity)}\n${query.window}`, () => load(identity, query.window))
        if (!sameIdentity(identity, await verify(credential, query.agent))) notAttributed('The native root changed during the read')
        json(res, 200, body)
      } catch (error) {
        if (error instanceof Refusal) failure(res, error.status, error.code, error.message)
        else if (error instanceof LoadsBusy) failure(res, 503, 'root-detail-busy', 'Root detail is busy')
        else failure(res, 502, 'root-detail-unavailable', 'Root detail is unavailable')
      }
    },
  }
}
