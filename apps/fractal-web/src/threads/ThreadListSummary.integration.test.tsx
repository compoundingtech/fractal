// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { LiveAgentThreads } from './LiveThreads.tsx'
import { gatewayThreads } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const agent = 'agent/workshop/atlas'
const id = 'thread/older-page'
const old = { id, agent_id: agent, title: 'Older title', title_revision: 'claim/title-old',
  revision: 'revision/old', state: 'open', message_count: 1 }
const latest = { ...old, title: 'Renamed remotely', title_revision: 'claim/title-new',
  revision: 'revision/new', state: 'closed', message_count: 3 }

const mountOlderPage = async ({ get, follow = false }: {
  readonly get: () => Promise<Response> | Response
  readonly follow?: boolean
}) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const base = fixture.capabilities
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async input => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: {
      ...base.value, transport: 'local-web', capabilities: [...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.reply', version: 0, state: 'granted' },
      ],
    } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [], next_cursor: null } })
    if (url.pathname.endsWith('/threads')) return Response.json({ ...base, value: {
      items: url.searchParams.has('cursor') ? [old] : Array.from({ length: 40 }, (_, index) => ({
        ...old, id: `thread/first-${index}`, title: `First page ${index}`,
      })),
      next_cursor: url.searchParams.has('cursor') ? null : 'older',
    } })
    return get()
  } })
  const registry = AtomRegistry.make()
  type Watches = NonNullable<Parameters<typeof gatewayThreads>[2]>
  let receive: Parameters<ReturnType<Watches['detail']>>[0] | undefined
  const unsubscribe = vi.fn()
  const threads = gatewayThreads(client, registry, follow ? {
    summaries: () => undefined,
    timeline: () => () => () => {},
    detail: () => callback => { receive = callback; return unsubscribe },
  } : undefined)
  registry.set(threads.draft(id), { content: 'Keep this unsent reply.' })
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], agents: [], missions: [], attention: [],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentThreads agentId={agent} /></DataSourceProvider>))
  await vi.waitFor(() => expect([...host.querySelectorAll('button')].some(button => button.textContent === 'Load more threads')).toBe(true))
  await act(async () => [...host.querySelectorAll('button')].find(button => button.textContent === 'Load more threads')?.click())
  await vi.waitFor(() => expect(host.querySelector('[data-thread-panel][aria-label="Older title"]')).not.toBeNull())
  const panel = host.querySelector('[data-thread-panel][aria-label="Older title"]')!
  return {
    host, panel, registry, threads, unsubscribe,
    toggle: () => act(async () => panel.querySelector<HTMLButtonElement>('button[aria-expanded]')!.click()),
    observe: async () => act(async () => receive?.({ _tag: 'Observed', value: {
      items: [Native.decodeUnknownSync(Native.ThreadSummary)(latest)], hasMore: false, snapshotId: 'snapshot/new',
    } })),
    lateObservation: () => receive?.({ _tag: 'Observed', value: {
      items: [Native.decodeUnknownSync(Native.ThreadSummary)({ ...latest, title: 'Cancelled observation' })],
      hasMore: false, snapshotId: 'snapshot/cancelled',
    } }),
    dispose: async () => {
      await act(async () => root.unmount())
      registry.dispose(); host.remove(); vi.unstubAllGlobals()
    },
  }
}

it.each(['rest', 'feed'] as const)('retains the 41st row’s accepted %s summary through collapse without replacing its composer', async mode => {
  const base = fixture.capabilities
  const view = await mountOlderPage({ follow: mode === 'feed', get: () => Response.json({ ...base, value: mode === 'rest' ? latest : old }) })
  try {
    await view.toggle()
    await vi.waitFor(() => expect(view.panel.querySelector('textarea')).not.toBeNull())
    const composer = view.panel.querySelector('textarea')!
    if (mode === 'feed') await view.observe()
    expect(view.panel.getAttribute('aria-label')).toBe(latest.title)
    expect(view.panel.textContent).toContain('3 messages')
    expect(view.panel.textContent).toContain('Closed')
    expect(view.host.querySelector(`[data-thread-panel][aria-label="${latest.title}"]`)).toBe(view.panel)
    expect(view.panel.querySelector('textarea')).toBe(composer)
    expect(composer.value).toBe('Keep this unsent reply.')
    await view.toggle()
    expect(view.panel.querySelector('textarea')).toBeNull()
    expect(view.panel.getAttribute('aria-label')).toBe(latest.title)
    expect(view.panel.textContent).toContain('3 messages')
    expect(view.panel.textContent).toContain('Closed')
    if (mode === 'feed') {
      await vi.waitFor(() => expect(view.unsubscribe).toHaveBeenCalledOnce())
      await act(async () => view.lateObservation())
      expect(view.panel.getAttribute('aria-label')).toBe(latest.title)
      expect(view.registry.get(view.threads.titles).get(id)).toBe(latest.title)
    }
    expect(view.registry.get(view.threads.draft(id)).content).toBe('Keep this unsent reply.')
  } finally { await view.dispose() }
})

it('ignores an exact REST response after its older-page row is collapsed', async () => {
  const pending = Promise.withResolvers<Response>()
  const view = await mountOlderPage({ get: () => pending.promise })
  try {
    await view.toggle()
    await view.toggle()
    await act(async () => pending.resolve(Response.json({ ...fixture.capabilities, value: latest })))
    expect(view.panel.getAttribute('aria-label')).toBe(old.title)
    expect(view.registry.get(view.threads.titles).get(id)).toBe(old.title)
  } finally { await view.dispose() }
})

it.each([401, 403])('does not revive a denied older-page row after exact detail returns %s', async status => {
  const view = await mountOlderPage({ follow: true, get: () => Response.json({
    api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
    code: 'forbidden', message: 'Thread read is no longer granted', retryable: false, details: {},
  }, { status }) })
  try {
    await view.toggle()
    await vi.waitFor(() => expect(view.host.contains(view.panel)).toBe(false))
    await act(async () => view.lateObservation())
    expect(view.host.contains(view.panel)).toBe(false)
    expect(view.registry.get(view.threads.titles).has(id)).toBe(false)
  } finally { await view.dispose() }
})
