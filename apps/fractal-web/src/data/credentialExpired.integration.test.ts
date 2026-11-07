import { createServer } from 'node:http'
import { describe, expect, it } from 'vitest'
import { liveSource } from './liveSource.ts'

/** Exercise the real SDK/bootstrap and HTTP response boundary, including non-JSON gateway refusals. */
describe('expired browser credential', () => {
  for (const status of [401, 403]) {
    it(`leaves loading on a real HTTP ${status} without exposing the response body`, async () => {
      const paths: string[] = []
      const server = createServer((request, response) => {
        paths.push(request.url ?? '')
        response.writeHead(status, { 'content-type': 'text/plain' })
        response.end('private credential diagnostic')
      })
      const listening = Promise.withResolvers<void>()
      server.listen(0, '127.0.0.1', listening.resolve)
      await listening.promise
      const address = server.address()
      if (address === null || typeof address === 'string') throw new Error('Expected a TCP server')
      const live = liveSource({ options: { baseUrl: `http://127.0.0.1:${address.port}`, maxFollows: 16 } })
      const release = live.registry.mount(live.source.agents)
      try {
        await expect.poll(() => live.registry.get(live.source.agents)).toMatchObject({
          _tag: 'Unavailable', reason: 'unauthorized', detail: 'Browser credential expired or revoked',
        })
        expect(paths).toContain('/v1/client/capabilities')
        expect(JSON.stringify(live.registry.get(live.source.agents))).not.toContain('private credential diagnostic')
        // Readers acquired after rejection must not start a new infinite wait either.
        expect(live.registry.get(live.source.conversation('agent/example'))).toMatchObject({
          _tag: 'Unavailable', reason: 'unauthorized',
        })
      } finally {
        release()
        await live.dispose()
        server.closeAllConnections()
        const closed = Promise.withResolvers<void>()
        server.close(error => error ? closed.reject(error) : closed.resolve())
        await closed.promise
      }
    })
  }
})
