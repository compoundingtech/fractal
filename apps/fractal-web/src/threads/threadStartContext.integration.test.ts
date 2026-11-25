import { FractalClient } from '@fractal/client'
import * as Write from '@fractal/client/schema'
import * as Native from '@fractal/client/schema/read'
import * as Schema from 'effect/Schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { gatewayThreads } from './threadSource.ts'

const base = fixture.capabilities
const original = 'agent/workshop/atlas'
const fallback = 'agent/workshop/new-recipient'
const origin = { source_id: 'step-run/review/+exact', episode: 'opaque episode/+?&' }
const originalRecipient = { agent_id: original, basis: { _tag: 'original' } }
const fallbackRecipient = {
  agent_id: fallback,
  basis: { _tag: 'fallback', original_agent_id: original, retirement: { claim_id: 'claim/retired', kind: 'agent.retired', state: 'retired' } },
}
const context = (start_recipient: unknown = originalRecipient) => ({ kind: 'thread-start-context', ...origin, start_recipient })
const capabilities = () => Response.json({ ...base, value: {
  ...base.value, transport: 'local-web', capabilities: [
    ...base.value.capabilities,
    { id: 'threads-v1', version: 3, state: 'granted' },
    { id: 'thread-summary-feed', version: 1, state: 'granted' },
    { id: 'thread.start', version: 0, state: 'granted' },
    { id: 'thread.reply', version: 0, state: 'granted' },
  ],
} })
const refusal = (code: string, status: number) => Response.json({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
  code, message: 'The requested recipient is unavailable.', retryable: false, details: {},
}, { status })
const decodeAction = Schema.decodeUnknownSync(Schema.fromJsonString(Write.ActionRequest))
type ThreadAction = Extract<Write.ActionRequest, { type: 'thread.start' | 'thread.reply' }>
const threadAction = (body: unknown): ThreadAction => {
  const action = decodeAction(body)
  if (action.type !== 'thread.start' && action.type !== 'thread.reply') throw new Error('Expected thread action')
  return action
}
const receipt = (body: ThreadAction) => Response.json({ ...base, value: {
  kind: 'action-result', action_id: body.id, operation_id: 'operation/send', status: 'completed',
  affected_ids: [body.parameters.target_id, 'message/first'], snapshot_id: base.snapshot.id,
} })

it.each([originalRecipient, fallbackRecipient, { agent_id: original, basis: { _tag: 'configured' } }])(
  'reads the authenticated exact opening key and preserves generated recipient proof %#', async recipient => {
    const reads: Array<{ url: URL; authorization: string | null }> = []
    const token = 'paired-secret'
    const registry = AtomRegistry.make()
    const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', credential: async () => token, fetchImpl: async (input, init) => {
      const url = new URL(String(input))
      if (url.pathname.endsWith('/capabilities')) return capabilities()
      reads.push({ url, authorization: new Headers(init?.headers).get('Authorization') })
      return Response.json({ ...base, value: context(recipient) })
    } }), registry)
    const atom = source.startContext!(origin.source_id, origin.episode)
    const stop = registry.mount(atom)
    try {
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', value: context(recipient) }))
      expect(reads).toHaveLength(1)
      expect(reads[0]?.url.pathname).toBe('/v1/client/threads/start-context')
      expect([...reads[0]!.url.searchParams]).toEqual([['source_id', origin.source_id], ['episode', origin.episode]])
      expect(reads[0]?.authorization).toBe(`Bearer ${token}`)
    } finally { stop(); registry.dispose() }
  },
)

it.each(['recipient-retired', 'not-found'])('keeps %s as a refusal with no inferred default recipient', async code => {
  let actions = 0
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async input => {
    if (String(input).endsWith('/capabilities')) return capabilities()
    if (String(input).includes('/actions')) actions++
    return refusal(code, code === 'not-found' ? 404 : 409)
  } }), registry)
  const atom = source.startContext!(origin.source_id, origin.episode)
  const stop = registry.mount(atom)
  try {
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', code, explanation: 'The requested recipient is unavailable.' }))
    expect(registry.get(atom)).not.toHaveProperty('value')
    expect(actions).toBe(0)
  } finally { stop(); registry.dispose() }
})

it('never infers retirement or fallback from an offline opening read', async () => {
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async input => {
    if (String(input).endsWith('/capabilities')) return capabilities()
    throw new TypeError('offline')
  } }), registry)
  const atom = source.startContext!(origin.source_id, origin.episode)
  const stop = registry.mount(atom)
  try {
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: 'failed' }))
    expect(registry.get(atom)).not.toHaveProperty('code', 'recipient-retired')
    expect(registry.get(atom)).not.toHaveProperty('value')
  } finally { stop(); registry.dispose() }
})

it('keeps POST404 without applied metadata unknown despite a changed exact-key preview', async () => {
  const actions: ThreadAction[] = []
  const keys: Array<[string | null, string | null]> = []
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return capabilities()
    if (url.pathname.endsWith('/start-context')) {
      keys.push([url.searchParams.get('source_id'), url.searchParams.get('episode')])
      return Response.json({ ...base, value: context(actions.length === 0 ? originalRecipient : fallbackRecipient) })
    }
    const body = threadAction(init?.body)
    actions.push(body)
    return actions.length === 1 ? refusal('not-found', 404) : receipt(body)
  } }), registry)
  const atom = source.startContext!(origin.source_id, origin.episode)
  const pending = source.pendingStarts!(origin.source_id)
  const stops = [registry.mount(atom), registry.mount(pending)]
  const parameters = { agent_id: original, origin, title: 'Review', content: 'Please explain.' }
  try {
    await vi.waitFor(() => expect(registry.get(atom)._tag).toBe('Observed'))
    expect(await source.start('draft/review', parameters)).toMatchObject({ _tag: 'Refused', detail: 'Fractal has not confirmed this action. Retry to check the same action.' })
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', value: context(fallbackRecipient) }))
    expect(actions).toHaveLength(1)
    expect(keys.length).toBeGreaterThanOrEqual(2)
    expect(keys.every(key => key[0] === origin.source_id && key[1] === origin.episode)).toBe(true)
    expect(registry.get(pending)).toMatchObject([{ draftKey: 'draft/review', state: 'unconfirmed', parameters }])
    expect(actions[0]?.parameters).toMatchObject(parameters)
    expect((await source.start('draft/review', { ...parameters, agent_id: fallback }))._tag).toBe('Refused')
    expect(actions).toHaveLength(1)
    expect((await source.retryStart!('draft/review'))._tag).toBe('Sent')
    expect(actions).toHaveLength(2)
    expect(actions[1]).toEqual(actions[0])
    expect(actions[1]).toMatchObject({ type: 'thread.start', parameters: { agent_id: original } })
  } finally { stops.forEach(stop => stop()); registry.dispose() }
})

it('keeps unconfirmed first intent and IDs immutable across recipient changes and receipt recovery', async () => {
  const actions: ThreadAction[] = []
  let recipient = originalRecipient
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    if (String(input).endsWith('/capabilities')) return capabilities()
    if (String(input).includes('/start-context')) return Response.json({ ...base, value: context(recipient) })
    const body = threadAction(init?.body)
    actions.push(body)
    if (actions.length === 1) throw new TypeError('lost receipt')
    return receipt(body)
  } }), registry)
  const atom = source.startContext!(origin.source_id, origin.episode)
  const pending = source.pendingStarts!(origin.source_id)
  const stops = [registry.mount(atom), registry.mount(pending)]
  const parameters = { agent_id: original, origin, title: 'Review', content: 'Original signed intent.' }
  try {
    await vi.waitFor(() => expect(registry.get(atom)._tag).toBe('Observed'))
    expect((await source.start('draft/review', parameters))._tag).toBe('Refused')
    recipient = fallbackRecipient
    source.reloadStartContext!(origin.source_id, origin.episode)
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', value: context(fallbackRecipient) }))
    expect(registry.get(pending)).toMatchObject([{ state: 'unconfirmed', parameters }])
    expect((await source.start('draft/review', { ...parameters, agent_id: fallback }))._tag).toBe('Refused')
    expect(actions).toHaveLength(1)
    expect((await source.retryStart!('draft/review'))._tag).toBe('Sent')
    expect(actions).toHaveLength(2)
    expect(actions[1]).toEqual(actions[0])
  } finally { stops.forEach(stop => stop()); registry.dispose() }
})

it('leaves existing replies on their stored thread without a recipient preview', async () => {
  const actions: ThreadAction[] = []
  let openingReads = 0
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    if (String(input).endsWith('/capabilities')) return capabilities()
    if (String(input).includes('/start-context')) { openingReads++; return refusal('recipient-retired', 409) }
    const body = threadAction(init?.body)
    actions.push(body)
    return receipt(body)
  } }), registry)
  const stop = registry.mount(source.access)
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions.has('thread.reply')).toBe(true))
    expect(await source.reply({ target_id: 'thread/retired-recipient', content: 'Continue the existing discussion.' })).toEqual({ _tag: 'Sent', threadId: 'thread/retired-recipient' })
    expect(actions[0]?.parameters).toEqual({ target_id: 'thread/retired-recipient', content: 'Continue the existing discussion.' })
    expect(openingReads).toBe(0)
  } finally { stop(); registry.dispose() }
})

it.each([
  context({ agent_id: original, basis: { _tag: 'fallback', original_agent_id: original, retirement: { claim_id: 'claim/r', kind: 'agent.stopped', state: 'retired' } } }),
  { ...context(), source_id: 'not-an-id' },
  { ...context(), episode: '' },
  { ...context(), episode: '😀'.repeat(257) },
])('rejects malformed generated contract payload %#', wire => {
  expect(() => Native.decodeUnknownSync(Native.ThreadStartContext)(wire)).toThrow()
})

it('preserves the generated Revision code-point boundary and opaque episode semantics', () => {
  expect(Native.decodeUnknownSync(Native.ThreadStartContext)({ ...context(), episode: '😀'.repeat(256) }).episode).toBe('😀'.repeat(256))
})

it.each([{ ...origin, episode: 'another episode' }, { ...origin, source_id: 'step-run/another' }])('rejects a successful response for a different exact opening %#', async wrongOrigin => {
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async input => {
    if (String(input).endsWith('/capabilities')) return capabilities()
    return Response.json({ ...base, value: { ...context(), ...wrongOrigin } })
  } }), registry)
  const atom = source.startContext!(origin.source_id, origin.episode)
  const stop = registry.mount(atom)
  try {
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: 'failed' }))
    expect(registry.get(atom)).not.toHaveProperty('value')
  } finally { stop(); registry.dispose() }
})

it.each([401, 403])('clears the previous recipient when exact opening access is revoked (%s)', async status => {
  let allowed = true
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async input => {
    if (String(input).endsWith('/capabilities')) return capabilities()
    return allowed ? Response.json({ ...base, value: context() }) : refusal('forbidden', status)
  } }), registry)
  const atom = source.startContext!(origin.source_id, origin.episode)
  const stop = registry.mount(atom)
  try {
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', value: context() }))
    allowed = false
    source.reloadStartContext!(origin.source_id, origin.episode)
    await vi.waitFor(() => expect(registry.get(atom)._tag).toBe('Unavailable'))
    expect(registry.get(atom)).not.toHaveProperty('value')
  } finally { stop(); registry.dispose() }
})
