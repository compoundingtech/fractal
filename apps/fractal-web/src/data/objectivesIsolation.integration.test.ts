import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import recording from './subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { gatewayObjectives } from './objectivesSource.ts'

it('keeps separate member reads and classifies unsupported native getters', async () => {
  const registry = AtomRegistry.make()
  const member = (baseUrl: string, supported: boolean) => {
    const options = { baseUrl, fetchImpl: async (input: RequestInfo | URL) => String(input).endsWith('/capabilities')
      ? Response.json(recording.capabilities)
      : supported ? Response.json({ ...recording.cases.agent.payload, value: { as_of_unix_ms: 1000, rows: [], otel_ui_url: baseUrl } })
        : new Response('', { status: 404 }) }
    return gatewayObjectives({ options, discovery: new FractalClient(options), registry, now: () => 1000 })
  }
  const first = member('https://first.example.invalid', true)
  const next = member('https://next.example.invalid', false)
  const release = [registry.mount(first.list), registry.mount(next.list)]
  try {
    await vi.waitFor(() => expect(registry.get(first.list)).toMatchObject({ _tag: 'Observed', value: { otel_ui_url: 'https://first.example.invalid' } }))
    await vi.waitFor(() => expect(registry.get(next.list)).toMatchObject({ _tag: 'Unavailable', reason: 'unsupported' }))
    expect(first.history('same-row')).not.toBe(next.history('same-row'))
  } finally {
    release.forEach(dispose => dispose())
    registry.dispose()
  }
})
