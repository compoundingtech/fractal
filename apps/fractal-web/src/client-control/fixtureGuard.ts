/** Local runner configuration, NOT daemon attestation or enrollment authority. */
export interface FixtureGuardInput {
  readonly origin: string
  /** Origin reserved by the approved bootstrap for this isolated browser identity. */
  readonly approvedOrigin: string
  readonly normalOrigins: readonly string[]
  readonly gateway: string | undefined
  /** All paths must be realpath-resolved by the runner before opening any connection. */
  readonly resolvedGatewayPath: string
  readonly approvedClientSocketPath: string
  readonly approvedInstanceRoot: string
  readonly normalGatewayPaths: readonly string[]
}

export interface FixtureBoundary {
  readonly origin: string
  readonly socketPath: string
  readonly instance: 'client-control-fixture'
}

const absoluteCanonicalPath = (path: string): boolean => path.startsWith('/') &&
  !path.includes('\\') && !/[\s\0?#%]/.test(path) &&
  path.split('/').slice(1).every(part => part !== '' && part !== '.' && part !== '..')

/**
 * Refuses ambiguous origins, privileged sockets and non-fixture paths before browser/server startup.
 * The approved paths must come from independently verified setup output, never browser storage or
 * URL parameters. Realpath resolution is a caller obligation: lexical paths cannot detect symlinks.
 * This cannot prove daemon local-only mode, fixture catalog, or pairing enrollment.
 */
export const guardFixtureBoundary = (input: FixtureGuardInput): FixtureBoundary => {
  const origin = new URL(input.origin)
  if (origin.origin !== input.origin || origin.protocol !== 'http:' ||
    origin.hostname !== '127.0.0.1' || origin.port === '' ||
    input.origin !== input.approvedOrigin || input.normalOrigins.includes(input.origin))
    throw new TypeError('A dedicated exact IPv4 loopback HTTP origin with an explicit port is required')

  const root = input.approvedInstanceRoot
  if (!absoluteCanonicalPath(root) || root.split('/').at(-1) !== 'client-control-fixture')
    throw new TypeError('An independently approved fixture instance root is required')
  const socket = input.gateway?.replace(/^unix:/, '')
  if (socket === undefined || !absoluteCanonicalPath(socket) ||
    socket !== input.resolvedGatewayPath || socket !== input.approvedClientSocketPath ||
    !socket.startsWith(`${root}/`) || input.normalGatewayPaths.includes(socket) ||
    !['st3-client.sock', 'fractal-client.sock'].includes(socket.split('/').at(-1) ?? ''))
    throw new TypeError('WF_ST_GATEWAY must be the resolved approved fixture instance client socket')

  return { origin: input.origin, socketPath: socket, instance: 'client-control-fixture' }
}
