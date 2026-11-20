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
import { LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads } from './threadSource.ts'
import type { UnreadActionPort, UnreadReminderRead } from './unread-action-port.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it.each([
  { name: 'marks another person’s visible message unread without naturally reading it', actor: 'person/avery', foregroundRead: false },
  { name: 'marks the viewer’s own visible message unread without naturally reading it', actor: 'person/operator', foregroundRead: false },
  { name: 'naturally reads an incoming agent reply and clears its captured reminder after 500 ms', actor: 'agent/example/atlas', foregroundRead: true },
])('$name', async ({ actor, foregroundRead }) => {
  let focused = false
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.spyOn(document, 'hasFocus').mockImplementation(() => focused)
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
  const observers: Viewport[] = []
  class Viewport {
    readonly rows = new Set<Element>()
    constructor(readonly changed: (entries: IntersectionObserverEntry[]) => void) { observers.push(this) }
    observe(row: Element) { this.rows.add(row) }
    unobserve(row: Element) { this.rows.delete(row) }
    takeRecords() { return [] }
    disconnect() { this.rows.clear() }
  }
  vi.stubGlobal('IntersectionObserver', Viewport)
  const base = fixture.capabilities
  const threadId = 'thread/production-anchor'
  const agentId = 'agent/example/atlas'
  const visibleId = 'message/visible-real-row'
  const offscreenId = 'message/newer-offscreen'
  const visibleBody = 'A meaningful reply from the actual timeline author.'
  const offscreenBody = 'A newer reply outside the viewport.'
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
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [
      { kind: 'message', id: offscreenId, actor_id: actor, content: offscreenBody, accepted_at_unix_ms: 2, thread_quote_id: null },
      { kind: 'message', id: visibleId, actor_id: actor, content: visibleBody, accepted_at_unix_ms: 1, thread_quote_id: null },
    ], next_cursor: null } })
    if (url.pathname.endsWith('/actions')) {
      const body = JSON.parse(String(init?.body))
      writes.push(body)
      return Response.json({ ...base, value: {
        kind: 'action-result', action_id: body.id, operation_id: 'operation/read',
        status: 'completed', affected_ids: [visibleId], snapshot_id: base.snapshot.id,
      } })
    }
    return Response.json({ ...base, value: {
      id: threadId, agent_id: agentId, title: 'Production timeline anchors',
      title_revision: 'revision/title', revision: 'revision/thread', state: 'closed', unread: true,
    } })
  } })
  const registry = AtomRegistry.make()
  const reminder = Atom.make<UnreadReminderRead>({ _tag: 'Known', reminder: {
    anchor_message_id: visibleId,
    marks: [{ mark_id: 'operation/captured-at-entry', anchor_message_id: visibleId }],
  } })
  const mark = vi.fn<UnreadActionPort['mark']>(async () => ({ _tag: 'Sent' }))
  const clear = vi.fn<UnreadActionPort['clear']>(async () => ({ _tag: 'Sent' }))
  const unreadActions: UnreadActionPort = {
    canMark: true, reminder: () => reminder, mark, clear,
    seek: async () => ({ _tag: 'Unavailable', detail: 'No jump is requested in this test.' }),
  }
  const source = {
    ...fixtureSource({ world: {
      now: 1, events: [], agents: [], missions: [], attention: [],
      conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
    } }),
    threads: { ...gatewayThreads(client, registry), unreadActions },
  }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  // Find real production rows by their message body, independently of the selector under test.
  const bodyWithText = (text: string) => [...host.querySelectorAll<HTMLElement>('[data-thread-message-content]')]
    .find(body => body.textContent === text)
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}>
      <LiveThreadPane threadId={threadId} active />
    </DataSourceProvider>))
    await vi.waitFor(() => expect(bodyWithText(visibleBody)).toBeDefined())
    const visible = bodyWithText(visibleBody)!
    const offscreen = bodyWithText(offscreenBody)!
    const visibleRow = visible.closest('li')!
    const offscreenRow = offscreen.closest('li')!
    for (const [body, row, top] of [[visible, visibleRow, 120], [offscreen, offscreenRow, 2000]] as const) {
      const rect = new DOMRect(10, top, 180, 40)
      body.style.lineHeight = '20px'
      vi.spyOn(body, 'getBoundingClientRect').mockReturnValue(rect)
      vi.spyOn(row, 'getBoundingClientRect').mockReturnValue(rect)
    }
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: (x: number, y: number) =>
      [visible, offscreen].find(body => {
        const rect = body.getBoundingClientRect()
        return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
      }) ?? document.body })
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    await act(async () => {
      focused = true
      window.dispatchEvent(new Event('focus'))
      // Deliver only observations the browser would produce for registered real rows.
      for (const observer of observers) observer.changed([...observer.rows].map(target => ({
        target, isIntersecting: target === visibleRow, intersectionRatio: target === visibleRow ? 1 : 0,
      }) as IntersectionObserverEntry))
    })
    await act(async () => { await vi.advanceTimersByTimeAsync(499) })
    expect(writes).toEqual([])
    expect(clear).not.toHaveBeenCalled()
    await act(async () => { await vi.advanceTimersByTimeAsync(1) })
    if (foregroundRead) {
      expect(writes).toHaveLength(1)
      expect(writes[0]).toMatchObject({
        type: 'thread.read', fence: { snapshot_id: base.snapshot.id, subject_revisions: {} },
        parameters: { target_id: threadId, through_message_id: visibleId, seen_message_ids: [visibleId] },
      })
      expect(clear).toHaveBeenCalledExactlyOnceWith({ threadId, observed_marks: ['operation/captured-at-entry'] })
      expect(mark).not.toHaveBeenCalled()
    } else {
      // Canonical anchoring must not broaden incoming-agent-only natural read eligibility.
      expect(writes).toEqual([])
      expect(clear).not.toHaveBeenCalled()
      const button = [...host.querySelectorAll('button')].find(row => row.textContent === 'Mark unread')
      expect(button).toBeDefined()
      await act(async () => button?.click())
      expect(mark).toHaveBeenCalledExactlyOnceWith({
        threadId, anchor_message_id: visibleId, observed_marks: ['operation/captured-at-entry'],
      })
      expect(writes).toEqual([])
      expect(clear).not.toHaveBeenCalled()
    }
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    Reflect.deleteProperty(document, 'elementFromPoint')
    vi.useRealTimers()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  }
})
