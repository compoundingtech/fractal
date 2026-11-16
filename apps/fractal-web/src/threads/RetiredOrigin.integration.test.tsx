// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import { Agent as AgentRow, decodeUnknownSync as decodeNative } from '@fractal/client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { LiveInboxThreads, LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const base = fixture.capabilities
const original = 'agent/workshop/atlas'
const fallback = 'agent/workshop/harbor'
const sourceId = 'custom/garden/review/origin'
const episode = 'claim/episode'
const ask = decodeUnknownSync(Attention)({
  id: 'attention/review', kind: 'attention', revision: episode, episode,
  person_id: 'person/operator', source_id: sourceId, requester_id: 'person/operator', conversation_id: original,
  attention_kind: 'custom.garden.review', title: 'Release checklist', detail: 'Is the checklist complete?', priority: 'high',
  state: 'open', alert: true, actions: ['work.done'], requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z',
})
const seat = (id: string, name: string, offline = false) => decodeNative(AgentRow)({ kind: 'agent', id, name, host_id: 'host/workshop', runtime_ids: [],
  reachability: offline ? 'unreachable' : 'reachable', state: offline ? 'stopped' : 'running', harness_state: 'idle', blocked_on: null, fault: null,
  revision: '1', updated_at: '2026-11-01T11:00:00.000Z' })
const originalRecipient = { agent_id: original, basis: { _tag: 'original' } }
const fallbackRecipient = {
  agent_id: fallback,
  basis: { _tag: 'fallback', original_agent_id: original, retirement: { claim_id: 'claim/retired', kind: 'agent.retired', state: 'retired' } },
}
const context = (url: URL, start_recipient: object) => Response.json({ ...base, value: {
  kind: 'thread-start-context', source_id: url.searchParams.get('source_id'), episode: url.searchParams.get('episode'), start_recipient,
} })
const refusal = (code: string, status: number) => Response.json({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
  code, message: 'The requested recipient is unavailable.', retryable: false, details: {},
}, { status })
type Action = { id: string; type: string; parameters: { target_id: string; agent_id?: string; content: string } }
const receipt = (action: Action) => Response.json({ ...base, value: { kind: 'action-result', action_id: action.id, operation_id: 'operation/send',
  status: 'completed', affected_ids: [action.parameters.target_id, 'message/first'], snapshot_id: base.snapshot.id } })
const page = (items: readonly object[]) => Response.json({ ...base, value: { items, next_cursor: null } })

/** Mounts one thread surface on a real gateway thread source. `preview` answers the exact-key start preview, `action` every write. */
async function mount(element: React.ReactElement, { preview, action = receipt, read = () => page([]), offline = false }: {
  readonly preview: (url: URL, actions: readonly Action[]) => Response
  readonly action?: (action: Action, actions: readonly Action[]) => Response
  readonly read?: (url: URL) => Response
  readonly offline?: boolean
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
      const body = JSON.parse(String(init?.body)) as Action
      actions.push(body)
      return action(body, actions)
    }
    return read(url)
  } })
  const registry = AtomRegistry.make()
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], missions: [], attention: [ask],
    agents: [seat(original, 'atlas', offline), seat(fallback, 'harbor')],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads: gatewayThreads(client, registry) }
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
  return {
    host, previews, actions, button, type,
    dispose: async () => {
      await act(async () => root.unmount())
      registry.dispose()
      host.remove()
      vi.unstubAllGlobals()
    },
  }
}

it('previews a fallback recipient as new beside the retired original agent and starts with the fallback', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, { preview: url => context(url, fallbackRecipient) })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    // Before any draft opens the item names who a thread would reach.
    expect(view.host.querySelector('[data-start-recipient="fallback"]')?.textContent).toBe('Ask harbor about this item without answering it.New recipientOriginally for atlas, who was retired.')
    expect(view.previews.map(url => [url.searchParams.get('source_id'), url.searchParams.get('episode')])).toContainEqual([sourceId, episode])
    await act(async () => view.button('Start thread')?.click())
    const recipient = view.host.querySelector('[data-start-recipient="fallback"]')
    expect(recipient?.textContent).toContain('To harbor')
    expect(recipient?.querySelector('[data-start-recipient-new]')?.textContent).toBe('New recipient')
    expect(recipient?.textContent).toContain('Originally for atlas, who was retired.')
    expect([...(recipient?.querySelectorAll('[title]') ?? [])].map(each => each.getAttribute('title'))).toEqual([fallback, original])
    await view.type('Is the checklist complete?')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(1))
    expect(view.actions[0]).toMatchObject({ type: 'thread.start', parameters: { agent_id: fallback, origin: { source_id: sourceId, episode } } })
  } finally { await view.dispose() }
})

it('refuses a recipient-retired item with Fractal’s reason and no default recipient', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, { preview: () => refusal('recipient-retired', 409) })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('[data-start-recipient="retired"]')).not.toBeNull())
    const notice = view.host.querySelector('[data-start-recipient="retired"]')
    expect(notice?.textContent).toContain('This item’s agent was retired and Fractal has no new recipient configured, so no thread can start here.')
    expect(notice?.textContent).toContain('The requested recipient is unavailable.')
    expect(view.button('Start thread')).toBeUndefined()
    expect(view.host.textContent).not.toContain('Ask ')
    expect(view.host.textContent).not.toContain('To ')
    expect(view.host.querySelector('textarea')).toBeNull()
    expect(view.actions).toHaveLength(0)
  } finally { await view.dispose() }
})

it('reads the recipient again after a mismatch, keeps the draft and refusal, and sends to the new recipient only when pressed again', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    preview: (url, actions) => context(url, actions.length === 0 ? originalRecipient : fallbackRecipient),
    action: (action, actions) => actions.length === 1 ? refusal('not-found', 404) : receipt(action),
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    expect(view.host.querySelector('[data-start-recipient="original"]')?.textContent).toBe('To atlas')
    await view.type('Is the checklist complete?')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.host.querySelector('[data-start-recipient-changed]')).not.toBeNull())
    expect(view.host.querySelector('[data-start-recipient-changed]')?.textContent).toBe('The recipient changed to harbor. Nothing has been sent; review the recipient and send again.')
    expect(view.host.querySelector('[data-start-recipient="fallback"]')?.textContent).toContain('To harbor')
    expect(view.host.querySelector('[role="alert"]')?.textContent).toContain("Couldn't start this thread.")
    expect(view.host.querySelector('textarea')?.value).toBe('Is the checklist complete?')
    // Reading the new recipient never sends on its own.
    await act(async () => {
      const { promise, resolve } = Promise.withResolvers<void>()
      setTimeout(resolve, 20)
      await promise
    })
    expect(view.actions).toHaveLength(1)
    expect(view.actions[0]?.parameters.agent_id).toBe(original)
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.actions).toHaveLength(2))
    expect(view.actions[1]?.parameters.agent_id).toBe(fallback)
    expect(view.actions[1]?.id).not.toBe(view.actions[0]?.id)
    expect(view.actions[1]?.parameters.target_id).not.toBe(view.actions[0]?.parameters.target_id)
  } finally { await view.dispose() }
})

it('names an offline original agent as the recipient without inferring retirement', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, { preview: url => context(url, originalRecipient), offline: true })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    expect(view.host.querySelector('[data-start-recipient="original"]')?.textContent).toBe('To atlas')
    expect(view.host.querySelector('[data-start-recipient-new]')).toBeNull()
    expect(view.host.textContent).not.toContain('retired')
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
