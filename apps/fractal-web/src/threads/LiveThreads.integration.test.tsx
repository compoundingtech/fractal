// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import type { Attention as AttentionRow, Mission, TerminalScreen } from '@fractal/client/schema'
import type { DecodedAgent } from '@fractal/sdk/read'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { initialFeedSync, type FeedSyncObservation } from '../data/feedSync.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { credentialExpiredMessage, type ConversationPage } from '../data/source.ts'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { LiveAgentThreads, LiveInboxThreads, LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const base = fixture.capabilities
const agent = 'agent/example/atlas'
const row = (id: string, title: string, state = 'open') => ({
  id, agent_id: agent, title, title_revision: 'claim/title', revision: 'claim/current', state, message_count: 1, unread: false,
})
const page = (items: readonly object[]) => Response.json({ ...base, value: { items, next_cursor: null } })
const refusal = (status: number, code: string, details: Record<string, unknown> = {}) => Response.json({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/thread-read',
  code, message: 'Private server diagnostic', retryable: false, details,
}, { status })
const ask = decodeUnknownSync(Attention)({
  id: 'attention/review', kind: 'attention', revision: 'claim/episode', episode: 'claim/episode',
  person_id: 'person/operator', source_id: 'custom/garden/review/origin', requester_id: 'person/operator', conversation_id: agent,
  attention_kind: 'custom.garden.review', title: 'Release checklist', detail: 'Is the checklist complete?', priority: 'high',
  state: 'open', alert: true, actions: ['work.done'], requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z',
})
const live: FeedSyncObservation = { status: { _tag: 'Live', since: 0 }, observedAt: 0 }

type Reply = (url: URL, init: RequestInit | undefined) => Response | Promise<Response>

/** Mounts one thread surface against a real gateway thread source; `reply` answers every read after capabilities. */
async function mount(element: React.ReactElement, { actions = [], reply }: { readonly actions?: readonly string[]; readonly reply: Reply }) {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const requests: URL[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    requests.push(url)
    if (url.pathname.endsWith('/capabilities')) return Response.json({
      ...base, value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        ...actions.map(id => ({ id, version: 0, state: 'granted' })),
      ] },
    })
    return reply(url, init)
  } })
  const registry = AtomRegistry.make()
  const gateway = Atom.make<FeedSyncObservation>(live).pipe(Atom.keepAlive)
  const threads = gatewayThreads(client, registry)
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], agents: [], missions: [], attention: [ask],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads, sync: {
    gateway,
    agents: Atom.make(initialFeedSync<readonly DecodedAgent[]>(0)),
    missions: Atom.make(initialFeedSync<readonly Mission[]>(0)),
    attention: Atom.make(initialFeedSync<readonly AttentionRow[]>(0)),
    conversation: () => Atom.make(initialFeedSync<ConversationPage>(0)),
    terminal: () => Atom.make(initialFeedSync<TerminalScreen>(0)),
  } }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}>{element}</DataSourceProvider>))
  const button = (name: string) => [...host.querySelectorAll('button')].find(each => each.textContent === name || each.getAttribute('aria-label') === name)
  return {
    host, requests, threads, button,
    setGateway: (next: FeedSyncObservation) => act(async () => registry.set(gateway, next)),
    dispose: async () => {
      await act(async () => root.unmount())
      registry.dispose()
      host.remove()
      vi.unstubAllGlobals()
    },
  }
}

const type = (element: HTMLTextAreaElement | HTMLInputElement | null | undefined, value: string) => act(async () => {
  const prototype = element instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype
  Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(element, value)
  element?.dispatchEvent(new Event('input', { bubbles: true }))
})

it('pins agent threads as one collapsed summary whose rows open the thread in its own tab', async () => {
  const open = vi.fn()
  const view = await mount(<LiveAgentThreads agentId={agent} onOpenThread={open} />, {
    actions: ['thread.reply'],
    reply: url => url.pathname.endsWith('/threads') ? page([row('thread/one', 'Release checklist'), row('thread/two', 'Rollout notes', 'closed')]) : page([]),
  })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('summary')?.getAttribute('aria-label')).toBe('Threads, 2'))
    expect(view.host.querySelector('summary')?.textContent).toBe('Threads2')
    const details = view.host.querySelector('details')
    expect(details?.open).toBe(false)
    expect(details?.getAttribute('aria-label')).toBe('Threads')
    // Both rows share one list; neither carries its own disclosure.
    const header = view.host.querySelector<HTMLButtonElement>('[data-thread-panel][aria-label="Release checklist"] button')
    expect(view.host.querySelectorAll('[data-thread-panel]')).toHaveLength(2)
    expect(header?.hasAttribute('aria-expanded')).toBe(false)
    await act(async () => header?.click())
    expect(open).toHaveBeenCalledExactlyOnceWith('thread/one')
    expect(view.host.querySelector('textarea')).toBeNull()
    expect(view.requests.some(url => url.pathname.endsWith('/timeline'))).toBe(false)
  } finally { await view.dispose() }
})

it('marks the section paused once and holds it until the gateway is live again, even after an HTTP read succeeds', async () => {
  const view = await mount(<LiveAgentThreads agentId={agent} />, { reply: () => page([row('thread/one', 'Release checklist')]) })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('summary')?.getAttribute('aria-label')).toBe('Threads, 1'))
    await view.setGateway({ status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 1, nextAt: 1_000, issue: 'socket closed' } }, observedAt: 1 })
    expect(view.host.querySelector('summary')?.getAttribute('aria-label')).toBe('Threads, 1, updates paused')
    expect(view.host.querySelector('summary [data-threads-paused]')?.textContent).toBe('· Updates paused')
    const reads = view.requests.length
    await act(async () => view.threads.reload?.())
    await vi.waitFor(() => expect(view.requests.length).toBeGreaterThan(reads))
    await view.setGateway({ status: { _tag: 'Requested', since: 2 }, observedAt: 2 })
    expect(view.host.querySelectorAll('[data-threads-paused]')).toHaveLength(1)
    await view.setGateway({ status: { _tag: 'Live', since: 3 }, observedAt: 3 })
    expect(view.host.querySelectorAll('[data-threads-paused]')).toHaveLength(0)
  } finally { await view.dispose() }
})

it('offers Retry after a failed read and shows the threads it then observes', async () => {
  let broken = true
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    actions: ['thread.start'],
    reply: url => broken ? refusal(500, 'internal') : url.pathname.endsWith('/threads') ? page([row('thread/one', 'Release checklist')]) : page([]),
  })
  try {
    await vi.waitFor(() => expect(view.button('Retry: Fractal could not load threads.')).toBeDefined())
    expect(view.host.textContent).not.toContain('Private server diagnostic')
    broken = false
    await act(async () => view.button('Retry: Fractal could not load threads.')?.click())
    await vi.waitFor(() => expect(view.host.querySelector('[data-thread-panel][aria-label="Release checklist"]')).not.toBeNull())
    expect(view.host.textContent).not.toContain('Fractal could not load threads.')
  } finally { silence.mockRestore(); await view.dispose() }
})

it('shows the pairing notice for a rejected credential without Start or Retry', async () => {
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    actions: ['thread.start'],
    reply: () => refusal(403, 'forbidden', { reason: 'credential-rejected' }),
  })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain(credentialExpiredMessage))
    expect(view.button('Start thread')).toBeUndefined()
    expect([...view.host.querySelectorAll('button')].some(each => (each.getAttribute('aria-label') ?? each.textContent ?? '').startsWith('Retry'))).toBe(false)
  } finally { await view.dispose() }
})

it('titles a first send from its first line when a sibling thread already uses the item title', async () => {
  const writes: Array<{ readonly type: string; readonly parameters: Record<string, unknown> }> = []
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    actions: ['thread.start'],
    reply: (url, init) => {
      if (url.pathname.endsWith('/actions')) {
        const action = JSON.parse(String(init?.body))
        writes.push(action)
        return Response.json({ ...base, value: { kind: 'action-result', action_id: action.id, operation_id: 'operation/start',
          status: 'completed', affected_ids: [action.parameters.target_id, 'message/first'], snapshot_id: base.snapshot.id } })
      }
      return url.pathname.endsWith('/threads') && url.searchParams.get('recently_closed') !== 'true' ? page([row('thread/one', 'Release checklist')]) : page([])
    },
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    const title = () => view.host.querySelector<HTMLInputElement>('input')
    expect(title()?.value).toBe('Release checklist')
    expect(view.host.textContent).toContain(`To ${agent}`)
    await type(view.host.querySelector('textarea'), 'Check the rollback step\nIt was skipped last time.')
    expect(title()?.value).toBe('Check the rollback step')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(writes).toHaveLength(1))
    expect(writes[0]).toMatchObject({ type: 'thread.start', parameters: { agent_id: agent, title: 'Check the rollback step' } })
  } finally { await view.dispose() }
})

it('replaces the composer with a read-only note and keeps lifecycle changes to one line', async () => {
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    reply: url => url.pathname.endsWith('/timeline') ? page([
      { kind: 'thread.closed', id: 'event/closed', actor_id: base.value.session_actor, accepted_at_unix_ms: Date.parse('2026-11-01T11:04:00Z') },
      { kind: 'message', id: 'message/first', actor_id: agent, content: 'The checklist is complete.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null },
    ]) : Response.json({ ...base, value: row('thread/one', 'Release checklist', 'closed') }),
  })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain('The checklist is complete.'))
    const closed = view.host.querySelector('[data-thread-event="thread.closed"]')
    expect(closed?.textContent).toMatch(/^You closed the thread·\S.* ago$/)
    expect(closed?.querySelector('strong')).toBeNull()
    expect(view.host.textContent).toContain('Read-only on this device. Reply from a device with control access.')
    expect(view.host.querySelector('textarea')).toBeNull()
  } finally { await view.dispose() }
})

it.each([['lost', true], ['held elsewhere', false]] as const)('focuses a thread tab composer only when focus was %s', async (_, lost) => {
  const outside = document.createElement('button')
  document.body.append(outside)
  if (lost) outside.blur(); else outside.focus()
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    actions: ['thread.reply'],
    reply: url => url.pathname.endsWith('/timeline') ? page([
      { kind: 'message', id: 'message/first', actor_id: agent, content: 'The checklist is complete.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null },
    ]) : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('textarea')).not.toBeNull())
    expect(document.activeElement).toBe(lost ? view.host.querySelector('textarea') : outside)
  } finally {
    await view.dispose()
    outside.remove()
  }
})

it('retires the unreachable-send advice once the gateway is live again and keeps the draft', async () => {
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    actions: ['thread.reply'],
    reply: (url, init) => {
      if (init?.method === 'POST') throw new TypeError('Failed to fetch')
      return url.pathname.endsWith('/timeline') ? page([]) : Response.json({ ...base, value: row('thread/one', 'Release checklist') })
    },
  })
  const advice = 'Fractal could not be reached, so nothing changed. Try again once it reconnects.'
  try {
    await vi.waitFor(() => expect(view.host.querySelector('textarea')).not.toBeNull())
    await view.setGateway({ status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 1, nextAt: 1_000, issue: 'socket closed' } }, observedAt: 1 })
    await type(view.host.querySelector('textarea'), 'Drafted while offline')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.host.querySelector('[role="alert"]')?.textContent).toBe(advice))
    await view.setGateway({ status: { _tag: 'Live', since: 2 }, observedAt: 2 })
    expect(view.host.textContent).not.toContain(advice)
    expect(view.host.querySelector('textarea')?.value).toBe('Drafted while offline')
  } finally { await view.dispose() }
})
