// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import * as Write from '@fractal/client/schema'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import * as Schema from 'effect/Schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import startRefusalWire from '../../../../docs/st3/client-v0/fixtures/thread-start-refusals.json'
import { LiveInboxThreads, LiveResolvedInboxThreads, LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads, type ThreadSource } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const base = fixture.capabilities
const original = 'agent/workshop/atlas'
const configured = 'agent/workshop/triage'
const fallback = 'agent/workshop/harbor'
const episode = 'claim/episode'
/** An item with no listed agent: a person asked it, and nothing on the row names who a thread would reach. */
const item = (source_id: string, attention_kind: string, title: string) => decodeUnknownSync(Attention)({
  id: `attention/${source_id.split('/').at(-1)}`, kind: 'attention', revision: episode, episode,
  person_id: 'person/operator', source_id, requester_id: 'person/operator',
  attention_kind, title, detail: 'Is the checklist complete?', priority: 'high',
  state: 'open', alert: true, actions: ['work.done'], requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z',
})
const ask = item('custom/garden/review/origin', 'custom.garden.review', 'Release checklist')
const notice = item('system/model-drift/atlas', 'model-drift', 'Model drift on atlas')
const seat = (id: string, name: string, offline = false) => Write.decodeUnknownSync(Write.Agent)({ kind: 'agent', id, name, host_id: 'host/workshop', runtime_ids: [],
  reachability: offline ? 'unreachable' : 'reachable', state: offline ? 'stopped' : 'running', harness_state: 'idle', blocked_on: null, fault: null,
  revision: '1', updated_at: '2026-11-01T11:00:00.000Z' })
const originalRecipient = { agent_id: original, basis: { _tag: 'original' } }
const configuredRecipient = { agent_id: configured, basis: { _tag: 'configured' } }
const fallbackRecipient = {
  agent_id: fallback,
  basis: { _tag: 'fallback', original_agent_id: original, retirement: { claim_id: 'claim/retired', kind: 'agent.retired', state: 'retired' } },
}
/** Fractal's start preview for exactly the key the client asked about; the gateway source decodes it with the generated schema. */
const context = (url: URL, start_recipient: object) => Response.json({ ...base, value: {
  kind: 'thread-start-context', source_id: url.searchParams.get('source_id'), episode: url.searchParams.get('episode'), start_recipient,
} })

// Whole canonical envelopes from cc060eea; never infer application certainty from the code.
const wireRefusal = (code: keyof typeof startRefusalWire) => Response.json(startRefusalWire[code], { status: 409 })

it.each(Object.values(startRefusalWire))('decodes a golden server 409 envelope through the generated contract', body => {
  expect(() => Write.decodeUnknownSync(Write.ErrorEnvelope)(body)).not.toThrow()
})
const decodeAction = Schema.decodeUnknownSync(Schema.fromJsonString(Write.ActionRequest))
type Action = Extract<Write.ActionRequest, { type: 'thread.start' | 'thread.reply' }>
const threadAction = (body: unknown): Action => {
  const action = decodeAction(body)
  if (action.type !== 'thread.start' && action.type !== 'thread.reply') throw new Error('Expected a thread action')
  return action
}
const receipt = (action: Action) => Response.json({ ...base, value: { kind: 'action-result', action_id: action.id, operation_id: 'operation/send',
  status: 'completed', affected_ids: [action.parameters.target_id, 'message/first'], snapshot_id: base.snapshot.id } })
const page = (items: readonly object[]) => Response.json({ ...base, value: { items, next_cursor: null } })
const startParameters = (action: Action | undefined) => action?.type === 'thread.start' ? action.parameters : undefined

/** Mounts one thread surface on a real gateway thread source. `preview` answers the exact-key start preview, `action` every write. */
async function mount(element: React.ReactElement, { preview, action = receipt, read = () => page([]), offline = false, withoutPreview = false }: {
  readonly preview: (url: URL, actions: readonly Action[]) => Response | Promise<Response>
  readonly action?: (action: Action, actions: readonly Action[]) => Response
  readonly read?: (url: URL) => Response
  readonly offline?: boolean
  /** A thread source with no preview reader at all. */
  readonly withoutPreview?: boolean
}) {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const previews: URL[] = []
  const actions: Action[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, transport: 'local-web', capabilities: [
      ...base.value.capabilities,
      { id: 'threads-v1', version: 3, state: 'granted' },
      { id: 'thread-summary-feed', version: 1, state: 'granted' },
      { id: 'thread.start', version: 0, state: 'granted' },
      { id: 'thread.reply', version: 0, state: 'granted' },
    ] } })
    if (url.pathname === '/v1/client/threads/start-context') {
      previews.push(url)
      return preview(url, actions)
    }
    if (url.pathname.endsWith('/actions')) {
      const body = threadAction(init?.body)
      actions.push(body)
      return action(body, actions)
    }
    return read(url)
  } })
  const registry = AtomRegistry.make()
  const gateway = gatewayThreads(client, registry)
  const { startContext: _startContext, reloadStartContext: _reloadStartContext, ...withoutReader } = gateway
  const threads: ThreadSource = withoutPreview ? withoutReader : gateway
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], missions: [], attention: [ask, notice],
    agents: [seat(original, 'atlas', offline), seat(configured, 'triage'), seat(fallback, 'harbor')],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}>{element}</DataSourceProvider>))
  const button = (name: string) => [...host.querySelectorAll('button')].find(each => each.textContent === name || each.getAttribute('aria-label') === name)
  const type = (value: string) => act(async () => {
    const input = host.querySelector('textarea')
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, value)
    input?.dispatchEvent(new Event('input', { bubbles: true }))
  })
  const settle = () => act(async () => {
    const { promise, resolve } = Promise.withResolvers<void>()
    setTimeout(resolve, 20)
    await promise
  })
  return {
    host, previews, actions, button, type, settle,
    dispose: async () => {
      await act(async () => root.unmount())
      registry.dispose()
      host.remove()
      vi.unstubAllGlobals()
    },
  }
}

it.each([
  { name: 'an ask with no listed agent', element: <LiveInboxThreads alert={ask} />, sourceId: ask.source_id, recipient: originalRecipient, basis: 'Original recipient' },
  { name: 'a typed system notice', element: <LiveInboxThreads alert={notice} />, sourceId: notice.source_id, recipient: configuredRecipient, basis: 'Configured recipient' },
  { name: 'a resolved authored step', element: <LiveResolvedInboxThreads sourceId="step-run/release/authored" episode={episode} title="Write the release notes" />,
    sourceId: 'step-run/release/authored', recipient: configuredRecipient, basis: 'Configured recipient' },
  { name: 'an agent-requested run', element: <LiveResolvedInboxThreads sourceId="step-run/release/agent-requested" episode={episode} title="Check the release" />,
    sourceId: 'step-run/release/agent-requested', recipient: originalRecipient, basis: 'Original recipient' },
])('starts $name with exactly the recipient Fractal’s preview names', async ({ element, sourceId, recipient, basis }) => {
  const view = await mount(element, { preview: url => context(url, recipient) })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    // Only the exact source/episode key is read; nothing on the row stands in for it.
    expect(view.previews.map(url => [...url.searchParams])).toContainEqual([['source_id', sourceId], ['episode', episode]])
    expect(view.previews.every(url => url.searchParams.get('source_id') === sourceId && url.searchParams.get('episode') === episode)).toBe(true)
    expect(view.host.textContent).not.toContain('No responsible agent')
    await act(async () => view.button('Start thread')?.click())
    const row = view.host.querySelector(`[data-start-recipient="${recipient.basis._tag}"]`)
    expect(row?.querySelector('[data-start-recipient-id]')?.textContent).toBe(recipient.agent_id)
    expect(row?.querySelector('[data-start-recipient-basis]')?.textContent).toBe(basis)
    await view.type('Is the checklist complete?')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(1))
    expect(view.actions[0]).toMatchObject({ type: 'thread.start', parameters: { agent_id: recipient.agent_id, origin: { source_id: sourceId, episode } } })
  } finally { await view.dispose() }
})

it('previews a fallback recipient as new beside the retired original agent and starts with the fallback', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, { preview: url => context(url, fallbackRecipient) })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    // Before any draft opens the item names who a thread would reach.
    const idle = view.host.querySelector('[data-start-recipient="fallback"]')
    expect(idle?.textContent).toContain('Ask harbor about this item without answering it.')
    expect(idle?.querySelector('[data-start-recipient-id]')?.textContent).toBe(fallback)
    await act(async () => view.button('Start thread')?.click())
    const recipient = view.host.querySelector('[data-start-recipient="fallback"]')
    expect(recipient?.textContent).toContain('To harbor')
    expect(recipient?.querySelector('[data-start-recipient-id]')?.textContent).toBe(fallback)
    expect(recipient?.querySelector('[data-start-recipient-basis]')?.textContent).toBe('New recipient')
    // The original agent is named on its own line, never as the recipient.
    const originalRow = recipient?.querySelector('[data-start-recipient-original-row]')
    expect(originalRow?.textContent).toContain('Originally for atlas')
    expect(originalRow?.querySelector('[data-start-recipient-original]')?.textContent).toBe(original)
    expect([...(recipient?.querySelectorAll('[title]') ?? [])].map(each => each.getAttribute('title'))).toEqual([fallback, original])
    await view.type('Is the checklist complete?')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(1))
    expect(startParameters(view.actions[0])?.agent_id).toBe(fallback)
  } finally { await view.dispose() }
})

it('keeps the real recipient-retired preview terminal with no target, Retry or send', async () => {
  const view = await mount(<LiveInboxThreads alert={notice} />, { preview: () => wireRefusal('recipient-retired') })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('[data-start-recipient="retired"]')).not.toBeNull())
    const notice = view.host.querySelector('[data-start-recipient="retired"]')
    expect(notice?.textContent).toContain('The recipient is retired, so no thread can start here.')
    expect(notice?.textContent).toContain(startRefusalWire['recipient-retired'].message)
    expect(view.button('Retry')).toBeUndefined()
    expect(view.button('Start thread')).toBeUndefined()
    expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
    expect(view.host.querySelector('textarea')).toBeNull()
    await view.settle()
    expect(view.actions).toHaveLength(0)
  } finally { await view.dispose() }
})

it('offers no Start and names no recipient on a source without the preview reader', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, { preview: url => context(url, originalRecipient), withoutPreview: true })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('[data-start-recipient="none"]')).not.toBeNull())
    expect(view.button('Start thread')).toBeUndefined()
    expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
    expect(view.previews).toHaveLength(0)
  } finally { await view.dispose() }
})

it.each(['stale-snapshot'] as const)('retains the original signed Start when golden %s has no applied metadata', async code => {
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    preview: (url, actions) => context(url, actions.length === 0 ? originalRecipient : configuredRecipient),
    action: (action, actions) => actions.length === 1 ? wireRefusal(code) : receipt(action),
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    await view.type('Keep the original signed intent.')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.button('Retry thread start')).toBeDefined())
    await view.settle()
    expect(view.actions).toHaveLength(1)
    await act(async () => view.button('Retry thread start')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(2))
    expect(view.actions[1]).toEqual(view.actions[0])
    expect(startParameters(view.actions[1])?.agent_id).toBe(original)
  } finally { await view.dispose() }
})

it.each(['outer', 'details'] as const)('keeps unknown dominant over golden retirement none in %s metadata and a later unassigned none', async location => {
  const golden = startRefusalWire['recipient-retired']
  const uncertain = location === 'outer'
    ? { ...golden, applied: 'unknown' }
    : { ...golden, details: { ...golden.details, applied: 'unknown' } }
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    preview: (url, actions) => context(url, actions.length === 0 ? originalRecipient : configuredRecipient),
    action: (action, actions) => actions.length === 1 ? Response.json(uncertain, { status: 409 })
      : actions.length === 2 ? wireRefusal('recipient-unassigned') : receipt(action),
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    await view.type('Preserve the ambiguous action.')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.button('Retry thread start')).toBeDefined())
    await view.settle()
    expect(view.actions).toHaveLength(1)
    await act(async () => view.button('Retry thread start')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(2))
    await view.settle()
    expect(view.button('Retry thread start')).toBeDefined()
    expect(view.actions[1]).toEqual(view.actions[0])
    await act(async () => view.button('Retry thread start')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(3))
    expect(view.actions[2]).toEqual(view.actions[0])
    expect(startParameters(view.actions[2])?.agent_id).toBe(original)
  } finally { await view.dispose() }
})

it('treats golden unassigned applied:none as a definitive blocked setup refusal with no further sends', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    preview: (url, actions) => actions.length === 0 ? context(url, originalRecipient) : wireRefusal('recipient-unassigned'),
    action: () => wireRefusal('recipient-unassigned'),
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    await view.type('Keep the refused setup draft.')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.host.querySelector('[data-start-recipient="unassigned"]')).not.toBeNull())
    expect(view.host.querySelector('[role="alert"]')?.textContent).toContain("Couldn't start this thread.")
    expect(view.button('Retry')).toBeUndefined()
    expect(view.button('Retry thread start')).toBeUndefined()
    expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
    expect(view.host.querySelector('textarea')?.value).toBe('Keep the refused setup draft.')
    const reads = view.previews.length
    await act(async () => view.button('Send')?.click())
    await view.settle()
    expect(view.previews).toHaveLength(reads)
    expect(view.actions).toHaveLength(1)
    expect(startParameters(view.actions[0])?.agent_id).toBe(original)
  } finally { await view.dispose() }
})

it.each(['changed', 'refused', 'retired'] as const)('visibly rereads after golden applied:none retirement and requires explicit confirmation when %s', async next => {
  let release!: () => void
  let pendingRead: Promise<Response> | undefined
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    preview: (url, actions) => {
      if (actions.length === 0) return context(url, originalRecipient)
      if (pendingRead === undefined) {
        const { promise, resolve } = Promise.withResolvers<Response>()
        pendingRead = promise
        release = () => resolve(next === 'changed' ? context(url, configuredRecipient)
          : wireRefusal(next === 'retired' ? 'recipient-retired' : 'recipient-unassigned'))
      }
      return pendingRead.then(response => response.clone())
    },
    action: (action, actions) => actions.length === 1 ? wireRefusal('recipient-retired') : receipt(action),
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    await view.type('Keep this exact draft.')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.host.textContent).toContain('Loading the recipient'))
    expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
    expect(view.actions).toHaveLength(1)
    expect(view.host.querySelector('textarea')?.value).toBe('Keep this exact draft.')
    await act(async () => release())
    await vi.waitFor(() => expect(view.host.querySelector('[role="alert"]')?.textContent).toContain("Couldn't start this thread."))
    await view.settle()
    expect(view.actions).toHaveLength(1)
    expect(view.previews.every(url => url.searchParams.get('source_id') === ask.source_id && url.searchParams.get('episode') === episode)).toBe(true)
    if (next === 'changed') {
      expect(view.host.querySelector('[data-start-recipient-id]')?.textContent).toBe(configured)
      await act(async () => view.button('Send')?.click())
      await vi.waitFor(() => expect(view.actions).toHaveLength(2))
      expect(startParameters(view.actions[1])?.agent_id).toBe(configured)
      expect(view.actions[1]?.id).not.toBe(view.actions[0]?.id)
      expect(view.actions[1]?.idempotency_key).not.toBe(view.actions[0]?.idempotency_key)
      expect(view.actions[1]?.parameters.target_id).not.toBe(view.actions[0]?.parameters.target_id)
    } else {
      expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
      if (next === 'retired') {
        expect(view.host.querySelector('[data-start-recipient="retired"]')).not.toBeNull()
        expect(view.button('Retry')).toBeUndefined()
      } else {
        expect(view.host.querySelector('[data-start-recipient="unassigned"]')).not.toBeNull()
        expect(view.button('Retry')).toBeUndefined()
      }
      await act(async () => view.button('Send')?.click())
      await view.settle()
      expect(view.actions).toHaveLength(1)
    }
  } finally { await view.dispose() }
})

it('fails closed on the real GET stale-fence body without inventing an unconfigured state', async () => {
  const view = await mount(<LiveInboxThreads alert={notice} />, { preview: () => wireRefusal('stale-snapshot') })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain(startRefusalWire['stale-snapshot'].message))
    expect(view.button('Start thread')).toBeUndefined()
    expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
    expect(view.host.querySelector('textarea')).toBeNull()
    expect(view.host.querySelector('[data-start-recipient="retired"]')).toBeNull()
    await view.settle()
    expect(view.actions).toHaveLength(0)
  } finally { await view.dispose() }
})

it('shows a blocked setup refusal for generated recipient-unassigned without Retry, sends or a refresh loop', async () => {
  const view = await mount(<LiveInboxThreads alert={notice} />, { preview: () => wireRefusal('recipient-unassigned') })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('[data-start-recipient="unassigned"]')).not.toBeNull())
    expect(view.host.textContent).toContain('No recipient is configured. Set up a responsible agent before starting a thread.')
    expect(view.host.textContent).not.toContain('The recipient is retired')
    expect(view.button('Retry')).toBeUndefined()
    expect(view.button('Start thread')).toBeUndefined()
    expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
    expect(view.host.querySelector('textarea')).toBeNull()
    const reads = view.previews.length
    await view.settle()
    expect(view.previews).toHaveLength(reads)
    expect(view.actions).toHaveLength(0)
  } finally { await view.dispose() }
})

it('keeps an unknown Start outcome on the exact original identity and retry key without automatic resend', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    preview: url => context(url, originalRecipient),
    action: (action, actions) => {
      if (actions.length === 1) throw new TypeError('Receipt lost')
      return receipt(action)
    },
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    await view.type('Recover the original intent.')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.button('Retry thread start')).toBeDefined())
    await view.settle()
    expect(view.actions).toHaveLength(1)
    await act(async () => view.button('Retry thread start')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(2))
    expect(view.actions[1]).toEqual(view.actions[0])
    expect(startParameters(view.actions[1])?.agent_id).toBe(original)
  } finally { await view.dispose() }
})

it('names an offline original agent as the recipient without inferring retirement', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, { preview: url => context(url, originalRecipient), offline: true })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    const row = view.host.querySelector('[data-start-recipient="original"]')
    expect(row?.querySelector('[data-start-recipient-id]')?.textContent).toBe(original)
    expect(row?.querySelector('[data-start-recipient-basis]')?.textContent).toBe('Original recipient')
    expect(view.host.querySelector('[data-start-recipient="fallback"]')).toBeNull()
    expect(view.host.textContent).not.toContain('Retired')
  } finally { await view.dispose() }
})

it('starts a distinct discussion on every Start of the same item', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, { preview: url => context(url, originalRecipient) })
  try {
    for (const [index, content] of ['First question.', 'Second question.'].entries()) {
      await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
      await act(async () => view.button('Start thread')?.click())
      await view.type(content)
      await act(async () => view.button('Send')?.click())
      await vi.waitFor(() => expect(view.actions).toHaveLength(index + 1))
    }
    expect(view.actions.map(startParameters).map(parameters => [parameters?.agent_id, parameters?.content])).toEqual([[original, 'First question.'], [original, 'Second question.']])
    expect(view.actions[1]?.id).not.toBe(view.actions[0]?.id)
    expect(view.actions[1]?.parameters.target_id).not.toBe(view.actions[0]?.parameters.target_id)
  } finally { await view.dispose() }
})

it('keeps replies on the existing thread’s stored agent without a recipient preview', async () => {
  const summary = { id: 'thread/one', agent_id: original, title: 'Release checklist', title_revision: 'claim/title', revision: 'claim/current', state: 'open', message_count: 1, unread: false }
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    preview: url => context(url, fallbackRecipient),
    read: url => url.pathname.endsWith('/timeline') ? page([]) : Response.json({ ...base, value: summary }),
  })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('textarea')).not.toBeNull())
    await view.type('Following up.')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(1))
    expect(view.actions[0]).toMatchObject({ type: 'thread.reply', parameters: { target_id: 'thread/one', content: 'Following up.' } })
    expect(view.actions[0]?.parameters).not.toHaveProperty('agent_id')
    expect(view.previews).toHaveLength(0)
    expect(view.host.querySelector('[data-start-recipient]')).toBeNull()
  } finally { await view.dispose() }
})

it('keeps a real recipient-retired POST terminal after rereading, without another send', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    preview: (url, actions) => actions.length === 0 ? context(url, originalRecipient) : wireRefusal('recipient-retired'),
    action: () => wireRefusal('recipient-retired'),
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    await view.type('Do not redirect this intent.')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.host.querySelector('[data-start-recipient="retired"]')).not.toBeNull())
    expect(view.host.querySelector('[data-start-recipient-id]')).toBeNull()
    expect(view.button('Retry')).toBeUndefined()
    expect(view.actions).toHaveLength(1)
    await act(async () => view.button('Send')?.click())
    await view.settle()
    expect(view.actions).toHaveLength(1)
  } finally { await view.dispose() }
})
