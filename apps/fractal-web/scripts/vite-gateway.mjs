import { Tracer } from 'effect'

import { createFractalWebMiddleware } from '../server/core.mts'
import { devAdmission } from '../server/devAdmission.mts'
import { readAuthorization } from '../server/authorization.mts'
import { loopbackProducer } from '../server/relayHttp.mts'

/**
 * Enable live data only when explicitly configured; Storybook remains credential-free.
 * `WF_USAGE_HISTORY_URL` additionally enables the interim usage-history relay (server/usageHistory.mts).
 */
export function webfractalGateway({ edgeHost, edgeAliases = [] } = {}) {
  return {
    name: 'wf:shared-client-gateway',
    apply: 'serve',
    configResolved(config) {
      if (!process.env.WF_ST_GATEWAY) return
      if (!['127.0.0.1', 'localhost'].includes(config.server.host)) {
        throw new Error('WF_ST_GATEWAY requires a loopback Vite listener')
      }
    },
    configureServer(vite) {
      const socket = process.env.WF_ST_GATEWAY
      if (!socket) return
      if (!vite.httpServer) throw new Error('WF_ST_GATEWAY requires a Vite HTTP listener')
      const authorization = readAuthorization(process.env, 'WF_ST_CREDENTIAL_FILE')
      const server = vite.httpServer
      const boundary = createFractalWebMiddleware({
        server,
        admit: devAdmission({
          edgeHost,
          edgeAliases,
          port: () => {
            const address = server.address()
            return typeof address === 'object' && address !== null ? address.port : undefined
          },
        }),
        gateway: {
          socketPath: socket.replace(/^unix:/, ''),
          host: process.env.WF_ST_GATEWAY_HOST ?? '127.0.0.1',
          authorization,
          timeoutMs: 30_000,
        },
        tracer: Tracer.make({ span: (options) => new Tracer.NativeSpan(options) }),
        usageHistory: loopbackProducer(process.env.WF_USAGE_HISTORY_URL, 'WF_USAGE_HISTORY_URL'),
      })
      vite.middlewares.use(boundary.middleware)
      vite.httpServer.once('close', boundary.close)
    },
  }
}
