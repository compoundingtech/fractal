// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { within } from 'storybook/test'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import type { Feed } from '../data/source.ts'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { LiveAgentWorkspace } from '../web/LiveAgentWorkspace.tsx'
import { gatewayThreads } from './threadSource.ts'
import type { UnreadActionPort, UnreadReminderRead } from './unread-action-port.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it('retains the opened marked thread, scroll and draft after dwell clears and removes its list row', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 })
  let focused = false
  vi.spyOn(document, 'hasFocus').mockImplementation(() => focused)
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
  window.localStorage.clear()
  window.history.replaceState(null, '', '/')
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
  const threadId = 'thread/retained-marked'
  const messageId = 'message/retained-anchor'
  const agentId = 'agent/example/atlas'
  const base = fixture.capabilities
  const mark: Native.ThreadUnreadMark = { mark_id: 'operation/captured-reminder', anchor_message_id: messageId }
  const reminderValue: Native.ThreadUnreadReminder = { marks: [mark], anchor_message_id: messageId }
  const summary = {
    id: threadId, agent_id: agentId, title: 'Retained marked conversation', state: 'open', unread: true,
    title_revision: 'revision/title', revision: 'revision/thread', unread_reminder: reminderValue,
  }
  const page = (items: readonly object[]): Feed<Native.ThreadSummaryPage> => ({
    _tag: 'Observed', freshness: 'live', value: Native.decodeUnknownSync(Native.ThreadSummaryPage)({ items, next_cursor: null }),
  })
  const registry = AtomRegistry.make()
  const markedList = Atom.make(page([summary])).pipe(Atom.keepAlive)
  const emptyList = Atom.make(page([])).pipe(Atom.keepAlive)
  const reminder = Atom.make<UnreadReminderRead>({ _tag: 'Known', reminder: reminderValue }).pipe(Atom.keepAlive)
  const clears = vi.fn<UnreadActionPort['clear']>(async () => {
    // The canonical refresh no longer contains the marked row. Its open context must not depend on that membership.
    registry.set(reminder, { _tag: 'Known', reminder: undefined })
    registry.set(markedList, page([]))
    return { _tag: 'Sent' }
  })
  const unreadActions: UnreadActionPort = {
    canMark: true, reminder: () => reminder, clear: clears,
    mark: async () => ({ _tag: 'Sent' }),
    seek: async () => ({ _tag: 'Found', messageId }),
  }
  const writes: unknown[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, transport: 'local-web', capabilities: [
      ...base.value.capabilities,
      { id: 'threads-v1', version: 3, state: 'granted' },
      { id: 'thread-summary-feed', version: 1, state: 'granted' },
      { id: 'thread.read', version: 0, state: 'granted' },
      { id: 'thread.reply', version: 0, state: 'granted' },
    ] } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [{
      kind: 'message', id: messageId, actor_id: agentId, content: 'A reply to return to.', accepted_at_unix_ms: 1, thread_quote_id: null,
    }], next_cursor: null } })
    if (url.pathname.endsWith('/actions')) {
      const body = JSON.parse(String(init?.body)); writes.push(body)
      return Response.json({ ...base, value: { kind: 'action-result', action_id: body.id, operation_id: 'operation/read',
        status: 'completed', affected_ids: [messageId], snapshot_id: base.snapshot.id } })
    }
    return Response.json({ ...base, value: summary })
  } })
  const threads = gatewayThreads(client, registry)
  const source = {
    ...fixtureSource({ world: { now: 1, events: [], agents: [], missions: [], attention: [],
      conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
    threads: { ...threads, list: (query: Parameters<typeof threads.list>[0]) => 'marked_unread' in query && query.marked_unread ? markedList : emptyList, unreadActions },
  }
  const host = document.createElement('div'); document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
    await act(async () => within(host).getByRole('tab', { name: 'Alerts' }).click())
    await vi.waitFor(() => expect(host.querySelector(`[data-marked-unread-thread="${threadId}"]`)).not.toBeNull())
    await act(async () => host.querySelector<HTMLButtonElement>(`[data-marked-unread-thread="${threadId}"] button`)!.click())
    await vi.waitFor(() => expect(host.querySelector('[data-thread-message-content]')).not.toBeNull())
    const body = host.querySelector<HTMLElement>('[data-thread-message-content]')!
    const row = body.closest('li')!
    const panel = body.closest<HTMLElement>('[data-thread-panel]')!
    const scroller = panel.parentElement!
    const draft = panel.querySelector<HTMLTextAreaElement>('textarea')!
    expect(draft).not.toBeNull()
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(draft, 'Keep this unsent reply')
      draft.dispatchEvent(new Event('input', { bubbles: true }))
    })
    scroller.scrollTop = 280
    const rect = new DOMRect(280, 200, 240, 40)
    body.style.lineHeight = '20px'
    vi.spyOn(body, 'getBoundingClientRect').mockReturnValue(rect)
    vi.spyOn(row, 'getBoundingClientRect').mockReturnValue(rect)
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => body })
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    await act(async () => {
      focused = true
      window.dispatchEvent(new Event('focus'))
      for (const observer of observers) observer.changed([...observer.rows].map(target => ({
        target, isIntersecting: target === row, intersectionRatio: target === row ? 1 : 0,
      }) as IntersectionObserverEntry))
    })
    await act(async () => { await vi.advanceTimersByTimeAsync(499) })
    expect(clears).not.toHaveBeenCalled()
    expect(host.querySelector(`[data-marked-unread-thread="${threadId}"]`)).not.toBeNull()
    await act(async () => { await vi.advanceTimersByTimeAsync(1) })
    expect(clears).toHaveBeenCalledExactlyOnceWith({ threadId, observed_marks: [mark.mark_id] })
    expect(writes).toHaveLength(1)
    expect(host.querySelector(`[data-marked-unread-thread="${threadId}"]`)).toBeNull()
    expect(host.textContent).toContain('No marked-unread threads')
    expect(host.querySelector('[data-thread-panel]')).toBe(panel)
    expect(host.querySelector('[data-thread-message-content]')).toBe(body)
    expect(panel.parentElement).toBe(scroller)
    expect(scroller.scrollTop).toBe(280)
    expect(panel.querySelector('textarea')).toBe(draft)
    expect(draft.value).toBe('Keep this unsent reply')
  } finally {
    await act(async () => root.unmount())
    registry.dispose(); host.remove()
    Reflect.deleteProperty(document, 'elementFromPoint')
    vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals()
    window.localStorage.clear(); window.history.replaceState(null, '', '/')
  }
})
