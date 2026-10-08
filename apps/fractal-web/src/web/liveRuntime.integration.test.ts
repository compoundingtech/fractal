import { Effect } from 'effect'
import * as FetchHttpClient from 'effect/http/FetchHttpClient'
import { afterEach, expect, it, vi } from 'vitest'

// Isolate the page's transport ownership; use the real telemetry and OTLP exporter.
vi.mock('../data/liveSource.ts', () => ({ liveSource: () => ({}) }))
vi.mock('virtual:build-identity', () => ({
  buildIdentity: { machineVersion: 'test' }, deploymentId: undefined,
}))

afterEach(() => {
  vi.unstubAllGlobals()
  vi.unstubAllEnvs()
  vi.restoreAllMocks()
  vi.resetModules()
})

const exportPageSpan = async (endpoint: string | undefined) => {
  vi.stubEnv('VITE_OTLP_TRACES_URL', endpoint)
  vi.stubGlobal('window', { location: { origin: 'https://example.test' } })
  const fetch = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => new Response('', { status: 200 }))
  const sendBeacon = vi.fn(() => true)
  vi.stubGlobal('fetch', fetch)
  vi.stubGlobal('navigator', { sendBeacon })
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {})
  // Import after configuring the browser: this test exercises the page module loading boundary.
  const { createPageRuntime } = await import('./liveRuntime.ts')
  const { telemetry } = createPageRuntime()
  // Closing the scope flushes the exporter deterministically, without timer advancement.
  await Effect.runPromise(Effect.void.pipe(
    Effect.withSpan('wf.ux.reload'),
    Effect.provide(telemetry.layer),
    Effect.provideService(FetchHttpClient.Fetch, fetch),
    Effect.scoped,
  ))
  return { fetch, sendBeacon, errors }
}

it('does not install a browser exporter without an explicitly configured endpoint', async () => {
  const { fetch, sendBeacon, errors } = await exportPageSpan(undefined)
  expect(fetch).not.toHaveBeenCalled()
  expect(sendBeacon).not.toHaveBeenCalled()
  expect(errors).not.toHaveBeenCalled()
})

it('exports to the explicitly configured traces endpoint', async () => {
  const endpoint = 'https://collector.example.test/v1/traces'
  const { fetch, sendBeacon, errors } = await exportPageSpan(endpoint)
  expect(fetch).toHaveBeenCalled()
  expect(fetch.mock.calls.every(([request]) => String(request) === endpoint)).toBe(true)
  expect(sendBeacon).not.toHaveBeenCalled()
  expect(errors).not.toHaveBeenCalled()
})
