// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads } from './threadSource.ts'
import { threadAnchorAttribute, threadAnchorSelector } from './threadAnchor.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it.each(['manual', 'foreground', 'inactive-pane'] as const)('marks only current visible message rows through their newest message (%s)', async mode => {
  let focused = false
  vi.spyOn(document, 'hasFocus').mockImplementation(() => focused)
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const observations: Viewport[] = []
  class Viewport {
    readonly rows = new Set<Element>()
    pending: IntersectionObserverEntry[] = []
    constructor(readonly changed: (entries: IntersectionObserverEntry[]) => void) {
      observations.push(this)
    }
    observe(row: Element) { this.rows.add(row) }
    unobserve(row: Element) { this.rows.delete(row) }
    disconnect() { this.rows.clear() }
    takeRecords() { return this.pending.splice(0) }
  }
  vi.stubGlobal('IntersectionObserver', Viewport)
  const entry = (target: Element, isIntersecting: boolean) => ({
    target, isIntersecting, intersectionRatio: isIntersecting ? 1 : 0,
  }) as IntersectionObserverEntry
  const base = fixture.capabilities
  const id = 'thread/thread-one'
  const agent = 'agent/workshop/atlas'
  const message = (id: string, accepted_at_unix_ms: number) => ({
    id, kind: 'message', actor_id: agent, content: id, accepted_at_unix_ms, thread_quote_id: null,
  })
  const writes: unknown[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({
      ...base, value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 3, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.read', version: 0, state: 'granted' },
      ] },
    })
    if (url.pathname.endsWith('/timeline')) {
      const older = url.searchParams.get('cursor') === 'older'
      return Response.json({ ...base, value: { items: older ? [
        message('message/visible', 3),
        { kind: 'thread.closed', id: 'event/closed', actor_id: 'person/operator', accepted_at_unix_ms: 2 },
        message('message/scrolled-away', 1),
      ] : [{ ...message('message/own-post', 5), actor_id: 'person/operator' }, message('message/newest-offscreen', 4)], next_cursor: older ? null : 'older' } })
    }
    if (url.pathname.endsWith('/actions')) {
      const body = JSON.parse(String(init?.body))
      writes.push(body)
      return Response.json({ ...base, value: {
        kind: 'action-result', action_id: body.id, operation_id: 'operation/read',
        status: 'completed', affected_ids: ['message/visible'], snapshot_id: base.snapshot.id,
      } })
    }
    return Response.json({ ...base, value: {
      id, agent_id: agent, title: 'PR #42', title_revision: 'claim/title', revision: 'claim/latest',
      state: 'closed', unread: true, message_count: 3,
    } })
  } })
  const registry = AtomRegistry.make()
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], agents: [], missions: [], attention: [],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads: gatewayThreads(client, registry) }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const mark = () => [...host.querySelectorAll('button')].find(row => row.getAttribute('aria-label') === 'Mark visible replies read')
  const messageViewport = (size: number) => observations.find(observer => observer.rows.size === size && [...observer.rows].every(row => row.matches(threadAnchorSelector)))
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveThreadPane threadId={id} active={mode !== 'inactive-pane'} /></DataSourceProvider>))
    await vi.waitFor(() => expect(messageViewport(2)).toBeDefined())
    const older = [...host.querySelectorAll('button')].find(row => row.textContent === 'Load earlier messages')
    expect(older).toBeDefined()
    await act(async () => older?.click())
    await vi.waitFor(() => expect(messageViewport(4)).toBeDefined())
    const viewport = messageViewport(4)!
    const row = (id: string) => [...viewport.rows].find(element => element.getAttribute(threadAnchorAttribute) === id)!
    for (const element of viewport.rows) {
      const rect = new DOMRect(8, element.getAttribute(threadAnchorAttribute) === 'message/newest-offscreen' ? 2000 : 120, 100, 40)
      vi.spyOn(element, 'getBoundingClientRect').mockReturnValue(rect)
      const content = element.querySelector<HTMLElement>('[data-thread-message-content]')
      if (content !== null) {
        content.style.lineHeight = '20px'
        vi.spyOn(content, 'getBoundingClientRect').mockReturnValue(rect)
      }
    }
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: (x: number, y: number) =>
      [...host.querySelectorAll('[data-thread-message-content]')].find(body => {
        const rect = body.getBoundingClientRect()
        return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
      }) ?? document.body })
    if (mode !== 'manual') vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    expect(mark()?.disabled).toBe(true)
    expect(writes).toEqual([])
    await act(async () => { for (const observer of observations) observer.changed([
      entry(row('message/scrolled-away'), true), entry(row('message/visible'), true),
      entry(row('message/newest-offscreen'), false),
    ]) })
    expect(mark()?.disabled).toBe(false)
    // The intersection changed since the last render. Take only what is visible at the action.
    if (mode === 'manual') {
      for (const observer of observations) observer.pending.push(entry(row('message/scrolled-away'), false))
      await act(async () => mark()?.click())
    } else {
      expect(writes).toEqual([])
      await act(async () => {
        const scrolledBody = row('message/scrolled-away').querySelector('[data-thread-message-content]')!
        vi.spyOn(scrolledBody, 'getBoundingClientRect').mockReturnValue(new DOMRect(8, -100, 100, 40))
        for (const observer of observations) observer.changed([entry(row('message/scrolled-away'), false)])
        focused = true
        window.dispatchEvent(new Event('focus'))
      })
      if (mode === 'inactive-pane') {
        expect(writes).toEqual([])
        await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveThreadPane threadId={id} active /></DataSourceProvider>))
        await act(async () => observations.at(-1)?.changed([entry(row('message/visible'), true)]))
      }
      await act(async () => { await vi.advanceTimersByTimeAsync(499) })
      expect(writes).toEqual([])
      await act(async () => { await vi.advanceTimersByTimeAsync(1) })
    }
    expect(writes).toHaveLength(1)
    expect(writes[0]).toMatchObject({
      type: 'thread.read', fence: { snapshot_id: base.snapshot.id, subject_revisions: {} },
      parameters: { target_id: id, through_message_id: 'message/visible', seen_message_ids: ['message/visible'] },
    })
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    Reflect.deleteProperty(document, 'elementFromPoint')
    vi.useRealTimers()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  }
})
