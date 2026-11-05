import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import recording from './subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { gatewayObjectives } from './objectivesSource.ts'

it('reads served rows and bounded history, preserving tones, forecasts, gaps and stale observations', async () => {
  const registry = AtomRegistry.make()
  const requests: URL[] = []
  let fail = false
  const row = { id: 'synthetic-budget', kind: 'budget', about: 'Synthetic allowance', current: '92% used', target: '100%', window: 'weekly', remaining_percent: 8, status: 'met', tone: 'yellow', explain: 'Synthetic stale allowance.', forecast_used_percent: 99, forecast_basis_minutes: 120, runs_out_at_unix_ms: 90_000_000 }
  const options = { baseUrl: 'http://gateway.invalid', fetchImpl: async (input: RequestInfo | URL) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json(recording.capabilities)
    requests.push(url)
    if (fail) throw new TypeError('Disconnected')
    const value = url.pathname.endsWith('/history') ? { id: row.id, since_ms: 3600000, until_ms: 90_000_000, samples: 2, steps: [80, null, 8] }
      : url.pathname.endsWith('/usage') ? { since_ms: 3600000, until_ms: 90_000_000, rows: [], accounts: [] }
      : { as_of_unix_ms: 90_000_000, rows: [row], otel_ui_url: 'https://otel.example.invalid' }
    return Response.json({ ...recording.cases.agent.payload, value })
  } }
  const source = gatewayObjectives({ options, discovery: new FractalClient(options), registry, now: () => 90_000_000 })
  const chart = source.history(row.id)
  const release = [registry.mount(source.list), registry.mount(chart), registry.mount(source.accounts)]
  try {
    expect(source.history(row.id)).toBe(chart)
    await vi.waitFor(() => expect(registry.get(source.list)).toMatchObject({ _tag: 'Observed', freshness: 'live', value: { rows: [{ tone: 'yellow', forecast_used_percent: 99, runs_out_at_unix_ms: 90_000_000 }] } }))
    await vi.waitFor(() => expect(registry.get(chart)).toMatchObject({ _tag: 'Observed', value: { steps: [80, null, 8] } }))
    const query = requests.find(url => url.pathname.endsWith('/history'))!.searchParams
    expect(Object.fromEntries(query)).toEqual({ id: row.id, since_ms: '3600000', until_ms: '90000000', steps: '48' })
    fail = true
    source.refresh()
    await vi.waitFor(() => expect(registry.get(source.list)).toMatchObject({ _tag: 'Observed', freshness: 'stale', error: { reason: 'failed' }, value: { rows: [{ tone: 'yellow' }] } }))
    fail = false
    source.refresh()
    await vi.waitFor(() => expect(registry.get(source.list)).toMatchObject({ freshness: 'live' }))
  } finally {
    release.forEach(dispose => dispose())
    registry.dispose()
  }
})
