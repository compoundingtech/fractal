import { request } from 'node:http'
import { Schema } from 'effect'
import type { GatewayOptions } from './gateway.mts'

const Capabilities = Schema.fromJsonString(Schema.Struct({
  api_version: Schema.Literal('st3.client.v0'),
  value: Schema.Struct({ kind: Schema.Literal('capabilities'), capabilities: Schema.Array(Schema.Struct({
    id: Schema.String, state: Schema.Literals(['granted', 'ungranted', 'unavailable']),
  })) }),
}))

/** GET /v1/client/capabilities advertises every ALL_SCOPES entry, not only enabled features.
 * Source: crates/fractal-api-core/src/api/client_v0.rs (ALL_SCOPES and capabilities).
 * Scope namespaces are read, write, control and terminal; pairing authority is control.pairing.
 */
export const checkTagCapabilities = (body: string, label = 'Tag'): void => {
  const { value } = Schema.decodeUnknownSync(Capabilities)(body)
  const scopes = value.capabilities.filter(capability => /^(?:read|write|control|terminal|pairing)\./.test(capability.id))
  if (!scopes.some(scope => scope.id.startsWith('read.') && scope.state === 'granted') ||
      scopes.some(scope => scope.state === 'granted' && !scope.id.startsWith('read.')))
    throw new TypeError(`${label} credential must grant read.* scopes only (no write, control, pairing or terminal scopes)`)
}

/** Validate before creating the listener; authentication, wire and transport failures are fatal. */
const checkAuthorization = (gateway: GatewayOptions, authorization: string | undefined, label: string): Promise<void> => new Promise((resolve, reject) => {
  if (!authorization) { reject(new TypeError(`${label} authorization is required`)); return }
  const upstream = request({ socketPath: gateway.socketPath, path: '/v1/client/capabilities', method: 'GET',
    headers: { host: gateway.host, authorization }, agent: false }, response => {
    const chunks: Buffer[] = []
    let size = 0
    response.on('error', reject)
    response.on('aborted', () => reject(new TypeError(`${label} capabilities response aborted`)))
    response.on('data', (chunk: Buffer) => {
      size += chunk.length
      if (size > 262144) { response.destroy(new TypeError(`${label} capabilities response too large`)); return }
      chunks.push(chunk)
    })
    response.on('end', () => {
      try {
        if (response.statusCode !== 200) throw new TypeError(`${label} credential capabilities check failed`)
        checkTagCapabilities(Buffer.concat(chunks).toString('utf8'), label)
        resolve()
      } catch (error) { reject(error) }
    })
  })
  upstream.on('error', reject)
  upstream.setTimeout(gateway.timeoutMs, () => upstream.destroy(new TypeError(`${label} credential capabilities check timed out`)))
  upstream.end()
})

export const checkTagAuthorization = (gateway: GatewayOptions): Promise<void> =>
  checkAuthorization(gateway, gateway.tagAuthorization, 'Tag')

export const checkPublisherAuthorization = (gateway: GatewayOptions): Promise<void> =>
  checkAuthorization(gateway, gateway.authorization, 'Publisher')
