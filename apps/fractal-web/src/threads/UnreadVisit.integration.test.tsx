// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { LiveAgentThreads, LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads } from './threadSource.ts'
import type { UnreadActionPort, UnreadReminderRead } from './unread-action-port.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it('keeps delayed marks sticky in two already-open placements; a deliberate later visit clears only its entry IDs', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.spyOn(document, 'hasFocus').mockReturnValue(true)
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
  const observers: Observer[] = []
  class Observer {
    readonly rows = new Set<Element>()
    constructor(readonly changed: (entries: IntersectionObserverEntry[]) => void) { observers.push(this) }
    observe(row: Element) { this.rows.add(row) }
    unobserve(row: Element) { this.rows.delete(row) }
    takeRecords() { return [] }
    disconnect() { this.rows.clear() }
  }
  vi.stubGlobal('IntersectionObserver', Observer)
  const base = fixture.capabilities, threadId = 'thread/sticky', agentId = 'agent/example'
  const summary = { id: threadId, agent_id: agentId, title: 'A sticky reminder', title_revision: 'revision/title', revision: 'revision/thread', state: 'closed', unread: true, unread_reminder: null }
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async input => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, transport: 'local-web', capabilities: [...base.value.capabilities,
      { id: 'threads-v1', version: 3, state: 'granted' }, { id: 'thread-summary-feed', version: 1, state: 'granted' },
    ] } })
    if (url.pathname.endsWith('/threads')) return Response.json({ ...base, value: { items: [summary], next_cursor: null } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [{ kind: 'message', id: 'message/anchor', actor_id: agentId, content: 'Read this meaningful body', accepted_at_unix_ms: 1, thread_quote_id: null }], next_cursor: null } })
    return Response.json({ ...base, value: summary })
  } })
  const registry = AtomRegistry.make(), gateway = gatewayThreads(client, registry)
  const read = Atom.make<UnreadReminderRead>({ _tag: 'Unavailable', detail: 'Entry intent is not known yet.' })
  const clear = vi.fn<UnreadActionPort['clear']>(async () => ({ _tag: 'Sent' }))
  const unreadActions: UnreadActionPort = { canMark: true, reminder: () => read, clear,
    mark: async () => ({ _tag: 'Sent' }), seek: async () => ({ _tag: 'Unavailable', detail: 'Not seeking in this case.' }) }
  const source = { ...fixtureSource({ world: { now: 1, events: [], agents: [], missions: [], attention: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), threads: { ...gateway, unreadActions } }
  const host = document.createElement('div'); document.body.append(host)
  const root = createRoot(host)
  const render = (active: boolean) => act(async () => root.render(<DataSourceProvider source={source} registry={registry}>
    <LiveThreadPane threadId={threadId} active={active} /><LiveAgentThreads agentId={agentId} />
  </DataSourceProvider>))
  const intersect = () => {
    for (const observer of observers) observer.changed([...observer.rows].map(row => ({ target: row, isIntersecting: true, intersectionRatio: 1 }) as IntersectionObserverEntry))
  }
  try {
    await render(true)
    await vi.waitFor(() => expect(host.querySelectorAll('[data-thread-panel]').length).toBe(2))
    await act(async () => {
      const group = host.querySelector('details')
      if (group !== null) { group.open = true; group.dispatchEvent(new Event('toggle')) }
    })
    const embedded = host.querySelector<HTMLButtonElement>('button[aria-expanded="false"]')
    expect(embedded).not.toBeNull()
    await act(async () => embedded?.click())
    await vi.waitFor(() => expect(host.querySelectorAll('[data-thread-anchor]').length).toBe(2))
    let top = 120
    for (const row of host.querySelectorAll('[data-thread-anchor]')) {
      const rect = new DOMRect(10, top, 120, 40)
      top += 120
      vi.spyOn(row, 'getBoundingClientRect').mockReturnValue(rect)
      const body = row.querySelector<HTMLElement>('[data-thread-message-content]')!
      body.style.lineHeight = '20px'
      vi.spyOn(body, 'getBoundingClientRect').mockReturnValue(rect)
    }
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: (x: number, y: number) =>
      [...host.querySelectorAll('[data-thread-message-content]')].find(body => {
        const rect = body.getBoundingClientRect()
        return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
      }) ?? document.body })
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    // Replication completes after both placements entered, before their first geometry event.
    await act(async () => registry.set(read, { _tag: 'Known', reminder: { anchor_message_id: 'message/anchor', marks: [{ mark_id: 'operation/late', anchor_message_id: 'message/anchor' }] } }))
    await act(async () => { intersect(); await vi.advanceTimersByTimeAsync(500) })
    expect(clear).not.toHaveBeenCalled()
    // Focus loss/return is not a visit boundary in either placement.
    await act(async () => { window.dispatchEvent(new Event('blur')); window.dispatchEvent(new Event('focus')); intersect(); await vi.advanceTimersByTimeAsync(500) })
    expect(clear).not.toHaveBeenCalled()
    await render(false)
    await render(true)
    await act(async () => { intersect(); await vi.advanceTimersByTimeAsync(500) })
    expect(clear).toHaveBeenCalledExactlyOnceWith({ threadId, observed_marks: ['operation/late'] })
    // An expanded embedded placement also starts a later visit, without a dedicated-pane exception.
    await act(async () => embedded?.click())
    await act(async () => embedded?.click())
    const reopened = host.querySelectorAll<HTMLElement>('[data-thread-message-content]')[1]!
    reopened.style.lineHeight = '20px'
    vi.spyOn(reopened, 'getBoundingClientRect').mockReturnValue(new DOMRect(10, 240, 120, 40))
    await act(async () => { intersect(); await vi.advanceTimersByTimeAsync(500) })
    expect(clear).toHaveBeenCalledTimes(2)
    expect(clear).toHaveBeenLastCalledWith({ threadId, observed_marks: ['operation/late'] })
  } finally {
    await act(async () => root.unmount()); registry.dispose(); host.remove()
    Reflect.deleteProperty(document, 'elementFromPoint')
    vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals()
  }
})
