import { createServer } from 'node:http'
import { describe, expect, it } from 'vitest'
import { liveSource } from './liveSource.ts'

const refusal = (details: unknown) => JSON.stringify({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
  code: 'forbidden', message: 'private credential diagnostic', retryable: false, details,
})

/** Real HTTP responses pass through the actual live source and SDK, not a mocked transport. */
describe('browser credential rejection', () => {
  for (const test of [
    { name: 'credential marker', status: 403, body: refusal({ reason: 'credential-rejected' }), reason: 'unauthorized' },
    { name: 'scope refusal or old daemon', status: 403, body: refusal({}), reason: 'ungranted' },
    { name: 'plain-text refusal', status: 403, body: 'Forbidden', reason: 'ungranted' },
    { name: 'wrong status', status: 401, body: refusal({ reason: 'credential-rejected' }), reason: 'ungranted' },
    { name: 'invalid marker', status: 403, body: refusal({ reason: 42 }), reason: 'ungranted' },
  ] as const) {
    it(`classifies ${test.name} without matching message text`, async () => {
      const paths: string[] = []
      const server = createServer((request, response) => {
        paths.push(request.url ?? '')
        response.writeHead(test.status, { 'content-type': test.body.startsWith('{') ? 'application/json' : 'text/plain' })
        response.end(test.body)
      })
      const listening = Promise.withResolvers<void>()
      server.listen(0, '127.0.0.1', listening.resolve)
      await listening.promise
      const address = server.address()
      if (address === null || typeof address === 'string') throw new Error('Expected a TCP server')
      const live = liveSource({ options: { baseUrl: `http://127.0.0.1:${address.port}`, maxFollows: 16 } })
      const release = live.registry.mount(live.source.agents)
      try {
        await expect.poll(() => live.registry.get(live.source.agents)).toMatchObject({ _tag: 'Unavailable', reason: test.reason })
        expect(paths).toContain('/v1/client/capabilities')
        expect(live.registry.get(live.source.conversation('agent/example'))).toMatchObject({ _tag: 'Unavailable', reason: test.reason })
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
