// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import { Agent as AgentRow, decodeUnknownSync as decodeNative, type Attention as AttentionRow, type Mission, type TerminalScreen } from '@fractal/client/schema'
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
import { LiveAgentThreads, LiveInboxThreads, LiveMissionThreads, LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads } from './threadSource.ts'
import { ConversationPane } from '../web/ConversationPane.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const base = fixture.capabilities
const agent = 'agent/example/atlas'
/** A roster row for `id`, as Fractal serves it with its effective `name`. */
const seat = (id: string, name: string) => decodeNative(AgentRow)({ kind: 'agent', id, name, host_id: 'host/build-1', runtime_ids: [], reachability: 'reachable',
  state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-11-01T11:00:00.000Z' })
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

/** Mounts one thread surface against a real gateway thread source; `reply` answers every read after capabilities, `capabilities` may replace the granted answer, `unreachable` fails every request, capabilities included, and `agents` is the roster. */
async function mount(element: React.ReactElement, { actions = [], reply, capabilities, unreachable, conversation, agents = [] }: {
  readonly actions?: readonly string[]; readonly reply: Reply; readonly capabilities?: () => Response | undefined; readonly unreachable?: () => boolean; readonly conversation?: ConversationPage
  readonly agents?: readonly AgentRow[]
}) {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const requests: URL[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    requests.push(url)
    if (unreachable?.()) throw new TypeError('Failed to fetch')
    if (url.pathname.endsWith('/capabilities')) return capabilities?.() ?? Response.json({
      ...base, value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 3, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        ...actions.map(id => ({ id, version: 0, state: 'granted' })),
      ] },
    })
    if (url.pathname.endsWith('/request-history')) return Response.json({ ...base, value: {
      items: [], boundary_items: [], relations: [], source_episodes: [], complete: true, next_cursor: null, source_support: null,
    } })
    if (url.pathname === '/v1/client/threads/start-context') return Response.json({ ...base, value: {
      kind: 'thread-start-context', source_id: url.searchParams.get('source_id'), episode: url.searchParams.get('episode'),
      start_recipient: { agent_id: agent, basis: { _tag: 'original' } },
    } })
    return reply(url, init)
  } })
  const registry = AtomRegistry.make()
  const gateway = Atom.make<FeedSyncObservation>(live).pipe(Atom.keepAlive)
  const threads = gatewayThreads(client, registry)
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], agents, missions: [], attention: [ask],
    conversations: conversation === undefined ? {} : { [agent]: conversation }, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
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

it.each(['agent', 'mission'] as const)('expands independent %s threads inline, preserves their drafts, and opens a tab only explicitly', async surface => {
  const open = vi.fn()
  const summaries = [row('thread/one', 'Release checklist'), row('thread/two', 'Rollout notes', 'closed')]
  const view = await mount(surface === 'agent'
    ? <LiveAgentThreads agentId={agent} onOpenThread={open} />
    : <LiveMissionThreads missionId="mission/example/release" onOpenThread={open} />, {
    actions: ['thread.reply'],
    reply: url => {
      if (url.pathname.endsWith('/threads')) return page(summaries)
      const summary = summaries[url.pathname.includes('two') ? 1 : 0]!
      return url.pathname.endsWith('/timeline') ? page([{ kind: 'message', id: `message/${summary.id}`, actor_id: agent,
        content: `History for ${summary.title}.`, accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null }])
        : Response.json({ ...base, value: summary })
    },
  })
  const panel = (title: string) => view.host.querySelector<HTMLElement>(`[data-thread-panel][aria-label="${title}"]`)
  const header = (title: string) => panel(title)?.querySelector<HTMLButtonElement>('button[aria-expanded]')
  const draft = (title: string) => panel(title)?.querySelector<HTMLTextAreaElement>('textarea')
  try {
    await vi.waitFor(() => expect(view.host.querySelector('summary')?.getAttribute('aria-label')).toBe('Threads, 2'))
    expect(view.host.querySelector('details')?.open).toBe(false)
    expect(view.requests.some(url => url.pathname.endsWith('/timeline'))).toBe(false)
    await act(async () => view.host.querySelector('summary')?.click())
    expect(header('Release checklist')?.getAttribute('aria-expanded')).toBe('false')
    expect(header('Rollout notes')?.getAttribute('aria-expanded')).toBe('false')
    await act(async () => header('Release checklist')?.click())
    await vi.waitFor(() => expect(panel('Release checklist')?.textContent).toContain('History for Release checklist.'))
    expect(open).not.toHaveBeenCalled()
    expect(header('Release checklist')?.getAttribute('aria-expanded')).toBe('true')
    expect(header('Rollout notes')?.getAttribute('aria-expanded')).toBe('false')
    await type(draft('Release checklist'), 'Keep the checklist draft.')
    await act(async () => header('Rollout notes')?.click())
    await vi.waitFor(() => expect(panel('Rollout notes')?.textContent).toContain('History for Rollout notes.'))
    await type(draft('Rollout notes'), 'Keep the rollout draft.')
    expect(view.host.querySelectorAll('textarea')).toHaveLength(2)
    // Collapsing one thread must not collapse its sibling or navigate away.
    await act(async () => header('Release checklist')?.click())
    expect(draft('Release checklist')).toBeNull()
    expect(draft('Rollout notes')?.value).toBe('Keep the rollout draft.')
    await act(async () => header('Release checklist')?.click())
    await vi.waitFor(() => expect(draft('Release checklist')?.value).toBe('Keep the checklist draft.'))
    expect(open).not.toHaveBeenCalled()
    const openInTab = [...panel('Rollout notes')!.querySelectorAll('button')].find(button => button.textContent === 'Open in tab')
    expect(openInTab).toBeDefined()
    await act(async () => openInTab?.click())
    expect(open).toHaveBeenCalledExactlyOnceWith('thread/two')
    expect(draft('Release checklist')?.value).toBe('Keep the checklist draft.')
    expect(draft('Rollout notes')?.value).toBe('Keep the rollout draft.')
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

it.each([
  ['fails to reach Fractal', () => { throw new TypeError('Failed to fetch') }],
  ['gets a 503 from the gateway', () => refusal(503, 'unavailable')],
] as const)('marks an Inbox item paused with Retry when an open thread’s messages later %s while its list and detail stay live', async (_case, broken) => {
  let timelineDown = false
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    reply: url => url.pathname.endsWith('/threads') ? page(url.searchParams.get('recently_closed') === 'true' ? [] : [row('thread/one', 'Release checklist')])
      : url.pathname.endsWith('/timeline') ? timelineDown ? broken() : page([{ kind: 'message', id: 'message/first', actor_id: agent,
        content: 'The checklist is complete.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null }])
      : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  const heading = () => view.host.querySelector('h3')
  try {
    const header = () => view.host.querySelector<HTMLButtonElement>('[data-thread-panel][aria-label="Release checklist"] button')
    await vi.waitFor(() => expect(header()).not.toBeNull())
    if (header()?.getAttribute('aria-expanded') !== 'true') await act(async () => header()?.click())
    await vi.waitFor(() => expect(view.host.textContent).toContain('The checklist is complete.'))
    expect(heading()?.getAttribute('aria-label')).toBe('Threads, 1')
    expect(view.button('Retry: Updates paused')).toBeUndefined()
    // Every thread read runs again; only the messages fail, and the gateway stays live.
    timelineDown = true
    await act(async () => view.threads.reload?.())
    await vi.waitFor(() => expect(heading()?.getAttribute('aria-label')).toBe('Threads, 1, updates paused'))
    expect(view.host.querySelectorAll('[data-threads-paused]')).toHaveLength(1)
    expect(view.host.textContent).toContain('The checklist is complete.')
    expect(view.button('Retry: Updates paused')).toBeDefined()
    timelineDown = false
    await act(async () => view.button('Retry: Updates paused')?.click())
    await vi.waitFor(() => expect(heading()?.getAttribute('aria-label')).toBe('Threads, 1'))
    expect(view.host.querySelectorAll('[data-threads-paused]')).toHaveLength(0)
    expect(view.button('Retry: Updates paused')).toBeUndefined()
    expect(view.host.textContent).toContain('The checklist is complete.')
  } finally { silence.mockRestore(); await view.dispose() }
})

it('marks a thread in its own tab paused with Retry when its messages later fail while its detail stays live', async () => {
  let timelineDown = false
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    reply: url => url.pathname.endsWith('/timeline') ? timelineDown ? refusal(502, 'bad-gateway') : page([{ kind: 'message', id: 'message/first', actor_id: agent,
      content: 'The checklist is complete.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null }])
      : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain('The checklist is complete.'))
    expect(view.host.querySelector('[data-threads-paused]')).toBeNull()
    timelineDown = true
    await act(async () => view.threads.reload?.())
    await vi.waitFor(() => expect(view.host.querySelector('[data-threads-paused]')?.textContent).toContain('Updates paused'))
    expect(view.host.textContent).toContain('The checklist is complete.')
    timelineDown = false
    await act(async () => view.button('Retry: Updates paused')?.click())
    await vi.waitFor(() => expect(view.host.querySelector('[data-threads-paused]')).toBeNull())
  } finally { silence.mockRestore(); await view.dispose() }
})

it.each(['recovers through Retry', 'closes while stale'] as const)('marks an Inbox item paused when only an earlier message page later fails, and clears it once that page %s', async (path) => {
  let earlierDown = false
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const message = (id: string, content: string, minute: number) => ({ kind: 'message', id, actor_id: agent, content,
    accepted_at_unix_ms: Date.parse(`2026-11-01T11:0${minute}:00Z`), thread_quote_id: null })
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    reply: url => {
      if (url.pathname.endsWith('/threads')) return page(url.searchParams.get('recently_closed') === 'true' ? [] : [row('thread/one', 'Release checklist')])
      if (!url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: row('thread/one', 'Release checklist') })
      if (url.searchParams.get('cursor') !== 'older') return Response.json({ ...base, value: { items: [message('message/newest', 'The checklist is complete.', 3)], next_cursor: 'older' } })
      return earlierDown ? refusal(504, 'gateway-timeout') : page([message('message/earliest', 'Starting the checklist now.', 1)])
    },
  })
  const heading = () => view.host.querySelector('h3')
  const earlierReads = () => view.requests.filter(url => url.pathname.endsWith('/timeline') && url.searchParams.get('cursor') === 'older').length
  try {
    const header = () => view.host.querySelector<HTMLButtonElement>('[data-thread-panel][aria-label="Release checklist"] button')
    await vi.waitFor(() => expect(header()).not.toBeNull())
    if (header()?.getAttribute('aria-expanded') !== 'true') await act(async () => header()?.click())
    await vi.waitFor(() => expect(view.button('Load earlier messages')).toBeDefined())
    await act(async () => view.button('Load earlier messages')?.click())
    await vi.waitFor(() => expect(view.host.textContent).toContain('Starting the checklist now.'))
    expect(earlierReads()).toBe(1)
    // Every thread read runs again; only the earlier page fails, while the newest page, the list and the gateway stay live.
    earlierDown = true
    await act(async () => view.threads.reload?.())
    await vi.waitFor(() => expect(heading()?.getAttribute('aria-label')).toBe('Threads, 1, updates paused'))
    expect(earlierReads()).toBe(2)
    expect(view.host.textContent).toContain('Starting the checklist now.')
    expect(view.button('Retry: Updates paused')).toBeDefined()
    if (path === 'recovers through Retry') {
      earlierDown = false
      await act(async () => view.button('Retry: Updates paused')?.click())
      await vi.waitFor(() => expect(heading()?.getAttribute('aria-label')).toBe('Threads, 1'))
      // Retry read the failed earlier page again.
      expect(earlierReads()).toBe(3)
      expect(view.host.textContent).toContain('Starting the checklist now.')
    } else {
      // Closing the row unmounts the stale page, which withdraws its own report.
      await act(async () => header()?.click())
      await vi.waitFor(() => expect(heading()?.getAttribute('aria-label')).toBe('Threads, 1'))
      expect(view.host.textContent).not.toContain('Starting the checklist now.')
    }
    expect(view.host.querySelectorAll('[data-threads-paused]')).toHaveLength(0)
    expect(view.button('Retry: Updates paused')).toBeUndefined()
  } finally { silence.mockRestore(); await view.dispose() }
})

it('names an unlisted canonical recipient as an agent and displays its exact first-send id', async () => {
  // The canonical preview grants this recipient even though the mounted roster has no row.
  const view = await mount(<LiveInboxThreads alert={ask} />, { actions: ['thread.start'], reply: () => page([]) })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain('Ask an agent about this item without answering it.'))
    await act(async () => view.button('Start thread')?.click())
    expect(view.host.textContent).toContain('To an agent')
    expect(view.host.querySelector('[data-start-recipient-id]')?.textContent).toBe(agent)
    expect(view.host.querySelector('[data-start-recipient-basis]')?.textContent).toBe('Original recipient')
    expect([...view.host.querySelectorAll('[title]')].map(each => [each.textContent, each.getAttribute('title')])).toContainEqual(['an agent', agent])
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

it('shows a broken capabilities answer as a failed read with Retry, not as loading, and Retry reads access again', async () => {
  let broken = true
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    actions: ['thread.reply', 'thread.close'],
    capabilities: () => broken ? refusal(500, 'internal') : undefined,
    reply: url => url.pathname.endsWith('/timeline') ? page([]) : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  try {
    await vi.waitFor(() => expect(view.button('Retry: Fractal could not load threads.')).toBeDefined())
    expect(view.host.textContent).not.toContain('Loading thread')
    expect(view.host.textContent).not.toContain('Private server diagnostic')
    broken = false
    await act(async () => view.button('Retry: Fractal could not load threads.')?.click())
    await vi.waitFor(() => expect(view.button('Close thread')).toBeDefined())
    expect(view.host.textContent).not.toContain('Fractal could not load threads.')
  } finally { silence.mockRestore(); await view.dispose() }
})

it.each([
  ['fails', () => refusal(500, 'internal')],
  ['answers for another thread', () => Response.json({ ...base, value: row('thread/other', 'Rollout notes') })],
] as const)('replaces an open Inbox row with a Retry notice when its detail read %s, and Retry restores its messages and controls', async (_case, broken) => {
  let detailBroken = true
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    actions: ['thread.reply', 'thread.close'],
    reply: url => url.pathname.endsWith('/threads') ? page(url.searchParams.get('recently_closed') === 'true' ? [] : [row('thread/one', 'Release checklist')])
      : url.pathname.endsWith('/timeline') ? page([{ kind: 'message', id: 'message/first', actor_id: agent, content: 'The checklist is complete.',
        accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null }])
      : detailBroken ? broken() : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  try {
    const header = () => view.host.querySelector<HTMLButtonElement>('[data-thread-panel][aria-label="Release checklist"] button')
    await vi.waitFor(() => expect(header()).not.toBeNull())
    if (header()?.getAttribute('aria-expanded') !== 'true') await act(async () => header()?.click())
    await vi.waitFor(() => expect(view.button('Retry: Fractal could not load threads.')).toBeDefined())
    // The list and timeline answered, but the row cannot vouch for this thread, so it shows neither messages nor controls.
    expect(view.host.textContent).not.toContain('The checklist is complete.')
    expect(view.button('Close thread')).toBeUndefined()
    expect(view.host.querySelector('textarea')).toBeNull()
    detailBroken = false
    await act(async () => view.button('Retry: Fractal could not load threads.')?.click())
    await vi.waitFor(() => expect(view.button('Close thread')).toBeDefined())
    expect(view.host.textContent).toContain('The checklist is complete.')
    expect(view.host.textContent).not.toContain('Fractal could not load threads.')
  } finally { silence.mockRestore(); await view.dispose() }
})

it.each(['is still pending', 'fails until Retry'] as const)('lets an unread Inbox row opened while its detail read %s mark its visible reply read once the detail arrives', async (path) => {
  const observers: Viewport[] = []
  class Viewport {
    readonly rows = new Set<Element>()
    constructor(readonly changed: (entries: IntersectionObserverEntry[]) => void) { observers.push(this) }
    observe(row: Element) { this.rows.add(row) }
    unobserve(row: Element) { this.rows.delete(row) }
    disconnect() { this.rows.clear() }
    takeRecords() { return [] }
  }
  vi.stubGlobal('IntersectionObserver', Viewport)
  let release = () => {}
  const held = new Promise<void>(resolve => { release = resolve })
  let detailBroken = path === 'fails until Retry'
  let unread = true
  const writes: unknown[] = []
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const summary = () => ({ ...row('thread/one', 'Release checklist'), unread })
  const view = await mount(<LiveInboxThreads alert={ask} />, {
    actions: ['thread.read'],
    reply: async (url, init) => {
      if (url.pathname.endsWith('/threads')) return page(url.searchParams.get('recently_closed') === 'true' ? [] : [summary()])
      if (url.pathname.endsWith('/timeline')) return page([{ kind: 'message', id: 'message/first', actor_id: agent, content: 'The checklist is complete.',
        accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null }])
      if (url.pathname.endsWith('/actions')) {
        const body = JSON.parse(String(init?.body))
        writes.push(body)
        unread = false
        return Response.json({ ...base, value: { kind: 'action-result', action_id: body.id, operation_id: 'operation/read',
          status: 'completed', affected_ids: ['message/first'], snapshot_id: base.snapshot.id } })
      }
      if (path === 'is still pending') await held
      return detailBroken ? refusal(500, 'internal') : Response.json({ ...base, value: summary() })
    },
  })
  const mark = () => view.button('Mark visible replies read')
  try {
    const header = () => view.host.querySelector<HTMLButtonElement>('[data-thread-panel][aria-label="Release checklist"] button')
    await vi.waitFor(() => expect(header()).not.toBeNull())
    if (header()?.getAttribute('aria-expanded') !== 'true') await act(async () => header()?.click())
    if (path === 'is still pending') {
      await vi.waitFor(() => expect(view.host.textContent).toContain('Loading thread'))
      await act(async () => release())
    } else {
      await vi.waitFor(() => expect(view.button('Retry: Fractal could not load threads.')).toBeDefined())
      detailBroken = false
      await act(async () => view.button('Retry: Fractal could not load threads.')?.click())
    }
    // The timeline mounts only now; its rows are observed without the row being reopened.
    await vi.waitFor(() => expect(observers.at(-1)?.rows.size).toBe(1))
    const viewport = observers.at(-1)!
    const reply = [...viewport.rows][0]!
    vi.spyOn(reply, 'getBoundingClientRect').mockReturnValue(new DOMRect(8, 120, 100, 40))
    expect(mark()?.disabled).toBe(true)
    await act(async () => viewport.changed([{ target: reply, isIntersecting: true, intersectionRatio: 1 } as unknown as IntersectionObserverEntry]))
    expect(mark()?.disabled).toBe(false)
    await act(async () => mark()?.click())
    expect(writes).toHaveLength(1)
    expect(writes[0]).toMatchObject({ type: 'thread.read', parameters: { target_id: 'thread/one', through_message_id: 'message/first', seen_message_ids: ['message/first'] } })
    await vi.waitFor(() => expect(mark()).toBeUndefined())
  } finally { release(); silence.mockRestore(); await view.dispose() }
})

it('says a thread link names no thread instead of offering Retry', async () => {
  const silence = vi.spyOn(console, 'log').mockImplementation(() => {})
  const view = await mount(<LiveThreadPane threadId="thread/missing" />, { reply: () => refusal(404, 'not-found') })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain('Fractal has no thread at this link.'))
    expect(view.host.querySelector('button')).toBeNull()
    expect(view.host.textContent).not.toContain('Private server diagnostic')
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
    agents: [seat(agent, 'atlas')],
  })
  try {
    await vi.waitFor(() => expect(view.button('Start thread')).toBeDefined())
    await act(async () => view.button('Start thread')?.click())
    const title = () => view.host.querySelector<HTMLInputElement>('input')
    expect(title()?.value).toBe('Release checklist')
    expect(view.host.textContent).toContain('To atlas')
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

it('names thread senders without their ids: an unobserved agent as An agent, Fractal machinery as Fractal', async () => {
  const reconciler = 'agent/st3/reconciler'
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    reply: url => url.pathname.endsWith('/timeline') ? page([
      { kind: 'thread.closed', id: 'event/closed', actor_id: reconciler, accepted_at_unix_ms: Date.parse('2026-11-01T11:04:30Z') },
      { kind: 'message', id: 'message/daemon', actor_id: 'daemon/runtime', content: 'Delivery retried.', accepted_at_unix_ms: Date.parse('2026-11-01T11:04:00Z'), thread_quote_id: null },
      { kind: 'message', id: 'message/reconciler', actor_id: reconciler, content: 'Reconciled the mission run.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:30Z'), thread_quote_id: null },
      { kind: 'message', id: 'message/first', actor_id: 'agent/build-host-a/worker-tests', content: 'The checklist is complete.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null },
    ]) : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain('Delivery retried.'))
    const senders = [...view.host.querySelectorAll('ol > li > div > strong')].map(each => [each.textContent, each.getAttribute('title')])
    expect(senders).toEqual([
      ['An agent', 'agent/build-host-a/worker-tests'],
      ['Fractal', reconciler],
      ['Fractal', 'daemon/runtime'],
    ])
    expect(view.host.querySelector('[data-thread-event="thread.closed"]')?.textContent).toMatch(/^Fractal closed the thread/)
    expect(view.host.textContent).not.toMatch(/build-host-a|reconciler|daemon\//)
  } finally { await view.dispose() }
})

it('names another person in a thread by their handle, never as an agent', async () => {
  const robin = 'person/robin'
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    actions: ['thread.reply'],
    reply: url => url.pathname.endsWith('/timeline') ? page([
      { kind: 'thread.reopened', id: 'event/reopened', actor_id: robin, accepted_at_unix_ms: Date.parse('2026-11-01T11:04:30Z') },
      { kind: 'message', id: 'message/robin', actor_id: robin, content: 'The rollback step is covered.', accepted_at_unix_ms: Date.parse('2026-11-01T11:04:00Z'), thread_quote_id: null },
    ]) : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain('The rollback step is covered.'))
    const sender = view.host.querySelector('ol > li > div > strong')
    expect(sender?.textContent).toBe('robin')
    expect(sender?.getAttribute('title')).toBe(robin)
    expect(view.host.querySelector('[data-thread-event="thread.reopened"]')?.textContent).toMatch(/^robin reopened the thread/)
    expect(view.button('Reply')?.getAttribute('aria-label')).toBe('Reply to robin')
    expect(view.host.textContent).not.toMatch(/an agent/i)
  } finally { await view.dispose() }
})

it.each([
  // A served name is shown as declared, a slug included.
  { id: agent, name: 'atlas', shown: 'atlas', hidden: 'example/atlas' },
  // The roster names the reconciler like any seat; it still reads as Fractal, as in the terminal app.
  { id: 'agent/st3/reconciler', name: 'Reconciler', shown: 'Fractal', hidden: 'Reconciler' },
  // The daemon labels a seat that declares no name by its id without agent/; that is no name, opaque ids included.
  { id: 'agent/build-host-a/worker-tests', name: 'build-host-a/worker-tests', shown: 'An agent', hidden: 'worker-tests' },
  { id: 'agent/example/d415892ea7a1dab11e6ba7ed22a8eb13', name: 'example/d415892ea7a1dab11e6ba7ed22a8eb13', shown: 'An agent', hidden: 'd415892e' },
])('names an observed thread agent $name as $shown in the thread origin and as its sender, with the id only as the title', async ({ id, name, shown, hidden }) => {
  const inSentence = shown === 'An agent' ? 'an agent' : shown
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    agents: [seat(id, name)],
    reply: url => url.pathname.endsWith('/timeline') ? page([
      { kind: 'thread.renamed', id: 'event/renamed-again', actor_id: id, previous_title: 'Release checklist', title: 'Final checklist', accepted_at_unix_ms: Date.parse('2026-11-01T11:05:00Z') },
      { kind: 'thread.renamed', id: 'event/renamed', actor_id: id, previous_title: 'Checklist', title: 'Release checklist', accepted_at_unix_ms: Date.parse('2026-11-01T11:04:00Z') },
      { kind: 'message', id: 'message/first', actor_id: id, content: 'The checklist is complete.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null },
    ]) : Response.json({ ...base, value: { ...row('thread/one', 'Final checklist'), agent_id: id } }),
  })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('[data-thread-origin]')?.textContent).toBe(`With ${inSentence}`))
    expect(view.host.querySelector('[data-thread-origin] [title]')?.getAttribute('title')).toBe(id)
    await vi.waitFor(() => expect(view.host.textContent).toContain('The checklist is complete.'))
    const sender = view.host.querySelector('ol > li > div > strong')
    expect(sender?.textContent).toBe(shown)
    expect(sender?.getAttribute('title')).toBe(id)
    const renames = [...view.host.querySelectorAll('[data-thread-event="thread.renamed"]')]
    expect(renames.map(event => event.querySelector('span')?.textContent)).toEqual([
      `${shown} renamed “Checklist” to “Release checklist”`,
      `${shown} renamed “Release checklist” to “Final checklist”`,
    ])
    expect(renames.map(event => event.querySelector('time')?.dateTime)).toEqual([
      '2026-11-01T11:04:00.000Z',
      '2026-11-01T11:05:00.000Z',
    ])
    expect(view.host.textContent).not.toContain(hidden)
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
  // While the gateway is down every request fails before reaching it, the capabilities read for the fence included.
  let down = false
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    actions: ['thread.reply'],
    reply: url => url.pathname.endsWith('/timeline') ? page([]) : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
    unreachable: () => down,
  })
  const advice = 'Fractal could not be reached, so nothing changed. Try again once it reconnects.'
  try {
    await vi.waitFor(() => expect(view.host.querySelector('textarea')).not.toBeNull())
    down = true
    await view.setGateway({ status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 1, nextAt: 1_000, issue: 'socket closed' } }, observedAt: 1 })
    await type(view.host.querySelector('textarea'), 'Drafted while offline')
    await act(async () => view.button('Send')?.click())
    await vi.waitFor(() => expect(view.host.querySelector('[role="alert"]')?.textContent).toBe(advice))
    await view.setGateway({ status: { _tag: 'Live', since: 2 }, observedAt: 2 })
    expect(view.host.textContent).not.toContain(advice)
    expect(view.host.querySelector('textarea')?.value).toBe('Drafted while offline')
  } finally { await view.dispose() }
})

it('keeps a thread tab loading while Fractal has not answered which thread operations it allows', async () => {
  let failed = 0
  const view = await mount(<LiveThreadPane threadId="thread/one" />, {
    reply: () => Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
    unreachable: () => { failed += 1; return true },
  })
  try {
    // Once the capabilities read has failed the answer stays unknown, not a connection without threads.
    await vi.waitFor(() => expect(failed).toBeGreaterThan(0))
    await act(async () => {})
    expect(view.host.textContent).toContain('Loading thread')
    expect(view.host.textContent).not.toContain('does not support threads')
  } finally { await view.dispose() }
})

it.each(['empty', 'unavailable'] as const)('keeps agent threads in the main reading scroll with an %s conversation and the ordinary composer docked outside', async conversation => {
  const view = await mount(<ConversationPane agentRef={agent} agentName="Atlas" onOpenTool={() => {}} />, {
    actions: ['thread.reply'],
    ...(conversation === 'empty' ? { conversation: { items: [], hasOlder: false, observation: { empty: true } } } : {}),
    reply: url => url.pathname.endsWith('/threads') ? page([row('thread/one', 'Release checklist')])
      : url.pathname.endsWith('/timeline') ? page([{ kind: 'message', id: 'message/first', actor_id: agent,
        content: 'The checklist is complete.', accepted_at_unix_ms: Date.parse('2026-11-01T11:03:00Z'), thread_quote_id: null }])
      : Response.json({ ...base, value: row('thread/one', 'Release checklist') }),
  })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('summary')?.getAttribute('aria-label')).toBe('Threads, 1'))
    const dock = view.host.querySelector('[data-testid="conversation-composer-dock"]')
    const ordinaryInput = dock?.querySelector('textarea')
    expect(ordinaryInput).toBeDefined()
    expect(ordinaryInput).not.toBeNull()
    const scroll = view.host.querySelector<HTMLElement>(conversation === 'empty' ? '[data-testid="transcript-scroll"]' : '[data-testid="conversation-history-host"]')
    expect(scroll).not.toBeNull()
    if (conversation === 'unavailable') {
      expect(view.host.querySelector('[data-testid="transcript-unavailable"]')).not.toBeNull()
      expect(scroll?.style.overflow).toBe('auto')
    }
    const panel = view.host.querySelector('[data-thread-panel][aria-label="Release checklist"]')
    expect(scroll?.contains(panel)).toBe(true)
    expect(scroll?.contains(dock)).toBe(false)
    await act(async () => view.host.querySelector('summary')?.click())
    await act(async () => panel?.querySelector<HTMLButtonElement>('button[aria-expanded]')?.click())
    await vi.waitFor(() => expect(panel?.textContent).toContain('The checklist is complete.'))
    expect(dock?.querySelector('textarea')).toBe(ordinaryInput)
    expect(panel?.querySelector('textarea')).not.toBe(ordinaryInput)
  } finally { await view.dispose() }
})
