import { createServer } from 'node:http'
import { FractalClient } from '@fractal/client'
import type * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { describe, expect, it } from 'vitest'
import { gatewayDocuments } from '../documents/document-port.ts'
import { gatewayResources } from '../resources/agent/source.ts'
import { liveSource } from './liveSource.ts'
import { gatewayNativeDetails } from './nativeDetailsSource.ts'
import { gatewayObjectives } from './objectivesSource.ts'
import { gatewaySubjectReads } from './subjectReadPort.ts'
import type { Feed } from './source.ts'
import recording from './subjectReadPort.gateway.fixtures.json' with { type: 'json' }

const refusal = (details: unknown) => JSON.stringify({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
  code: 'forbidden', message: 'private credential diagnostic', retryable: false, details,
})

/** Real HTTP responses pass through each actual reader and SDK, not a mocked transport. */
describe.each(['follow', 'objectives', 'native details', 'subject', 'resources', 'documents'] as const)('%s credential rejection', (reader) => {
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
        // Independent getters must reach their own HTTP route, not fail during capability discovery.
        if (reader !== 'follow' && request.url === '/v1/client/capabilities') {
          response.writeHead(200, { 'content-type': 'application/json' })
          response.end(JSON.stringify(recording.capabilities))
        } else {
          response.writeHead(test.status, { 'content-type': test.body.startsWith('{') ? 'application/json' : 'text/plain' })
          response.end(test.body)
        }
      })
      const listening = Promise.withResolvers<void>()
      server.listen(0, '127.0.0.1', listening.resolve)
      await listening.promise
      const address = server.address()
      if (address === null || typeof address === 'string') throw new Error('Expected a TCP server')
      const options = { baseUrl: `http://127.0.0.1:${address.port}`, maxFollows: 16 }
      const live = reader === 'follow' ? liveSource({ options }) : undefined
      const registry = live?.registry ?? AtomRegistry.make()
      const discovery = new FractalClient(options)
      const args = { options, discovery, registry, now: () => Date.now() }
      const atom: Atom.Atom<Feed<unknown>> = live?.source.agents ?? (reader === 'objectives' ? gatewayObjectives(args).list
        : reader === 'native details' ? gatewayNativeDetails(args).usagePeriod('agent/example')
        : reader === 'subject' ? gatewaySubjectReads(args).session.feed('session/example')
        : gatewayResources({ baseUrl: options.baseUrl, fetchImpl: fetch }).byAgent('agent/example'))
      const release = reader === 'documents' ? () => {} : registry.mount(atom)
      try {
        if (reader === 'documents')
          expect(await gatewayDocuments(discovery).read('doc/example')).toMatchObject({ _tag: 'Unavailable', reason: test.reason })
        else await expect.poll(() => registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: test.reason })
        expect(paths).toContain('/v1/client/capabilities')
        if (reader !== 'follow') expect(paths.some(path => path !== '/v1/client/capabilities')).toBe(true)
        if (live) expect(registry.get(live.source.conversation('agent/example'))).toMatchObject({ _tag: 'Unavailable', reason: test.reason })
      } finally {
        release()
        if (live) await live.dispose()
        else registry.dispose()
        server.closeAllConnections()
        const closed = Promise.withResolvers<void>()
        server.close(error => error ? closed.reject(error) : closed.resolve())
        await closed.promise
      }
    })
  }
})
