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

/** Mounts one thread surface against a real gateway thread source; `reply` answers every read after capabilities, `capabilities` may replace the granted answer, and `unreachable` fails every request, capabilities included. */
async function mount(element: React.ReactElement, { actions = [], reply, capabilities, unreachable }: {
  readonly actions?: readonly string[]; readonly reply: Reply; readonly capabilities?: () => Response | undefined; readonly unreachable?: () => boolean
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
    expect(view.host.querySelector('summary')?.textContent).toBe('Threads 2')
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

it('names an agent by its short name, never its id, while the roster has no row for it', async () => {
  // The mounted roster holds no agents, so every agent read is unavailable.
  const view = await mount(<LiveInboxThreads alert={ask} />, { actions: ['thread.start'], reply: () => page([]) })
  try {
    await vi.waitFor(() => expect(view.host.textContent).toContain('Ask atlas about this item without answering it.'))
    await act(async () => view.button('Start thread')?.click())
    expect(view.host.textContent).toContain('To atlas')
    expect(view.host.textContent).not.toContain(agent)
    // The exact id stays available as the name's title.
    expect([...view.host.querySelectorAll('[title]')].map(each => [each.textContent, each.getAttribute('title')])).toContainEqual(['atlas', agent])
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
