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
import { LiveMarkedUnreadThreads } from './MarkedUnreadThreads.tsx'
import { gatewayThreads, upgradeThreadEndpoint } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it.each([2, 3])('requires the complete v3 thread experience on a read-only v%i connection', async version => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  const base = fixture.capabilities
  const threadId = 'thread/protocol'
  const anchor = 'message/protocol-anchor'
  const summary = { id: threadId, agent_id: 'agent/example', title: 'Protocol reminder',
    title_revision: 'revision/title', revision: 'revision/thread', state: 'closed', unread: false,
    unread_reminder: { anchor_message_id: anchor, marks: [{ mark_id: 'operation/protocol-mark', anchor_message_id: anchor }] } }
  const requests: URL[] = []
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async input => {
    const url = new URL(String(input)); requests.push(url)
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value,
      capabilities: [...base.value.capabilities.filter(capability => !capability.id.startsWith('thread.')),
        { id: 'threads-v1', version, state: 'granted' }, { id: 'thread-summary-feed', version: 1, state: 'granted' }],
    } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: {
      items: [{ kind: 'message', id: anchor, actor_id: 'agent/example', accepted_at_unix_ms: 1, content: 'A retained return anchor.', thread_quote_id: null }],
      next_cursor: null, ...(url.searchParams.has('around') ? { anchor: { message_id: anchor, state: 'current' } } : {}),
    } })
    return Response.json({ ...base, value: url.pathname.endsWith('/threads') ? { items: [summary], next_cursor: null } : summary })
  } })
  const registry = AtomRegistry.make()
  const threads = gatewayThreads(client, registry)
  const source = { ...fixtureSource({ world: { now: 1, events: [], agents: [], missions: [], attention: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), threads }
  const host = document.createElement('div'); document.body.append(host)
  const root = createRoot(host)
  const open = vi.fn()
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}>
      <LiveThreadPane threadId={threadId} active={false} /><LiveMarkedUnreadThreads onOpenThread={open} />
    </DataSourceProvider>))
    await vi.waitFor(() => expect(registry.get(threads.access)).toMatchObject({ read: version === 3, upgradeRequired: version === 2, actions: new Set() }))
    if (version === 2) {
      expect(host.textContent).toContain(upgradeThreadEndpoint)
      expect(host.textContent).not.toContain('Loading marked-unread threads')
      expect(await threads.unreadActions!.seek({ threadId, anchor_message_id: anchor })).toMatchObject({ _tag: 'Unavailable' })
      expect(requests).toHaveLength(1)
    } else {
      await vi.waitFor(() => expect(host.querySelector('[data-marked-unread-thread]')).not.toBeNull())
      expect(host.textContent).not.toContain(upgradeThreadEndpoint)
      expect(host.querySelector('[aria-label="Mark unread from the first visible reply"]')).toBeNull()
      await act(async () => host.querySelector<HTMLButtonElement>('[data-marked-unread-thread] button')!.click())
      await vi.waitFor(() => expect(open).toHaveBeenCalledWith(threadId))
      expect(requests.some(url => url.searchParams.get('marked_unread') === 'true')).toBe(true)
      expect(requests.some(url => url.searchParams.get('around') === anchor)).toBe(true)
    }
    expect(requests.some(url => url.pathname.endsWith('/actions'))).toBe(false)
    if (version === 2) expect(requests.some(url => url.searchParams.has('marked_unread') || url.searchParams.has('around'))).toBe(false)
  } finally {
    await act(async () => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals()
  }
})
