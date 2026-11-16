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

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it('sends an exact quoted reply while keeping the thread closed', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const base = fixture.capabilities
  const id = 'thread/closed-review'
  const messageId = 'message/review-result'
  const writes: unknown[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({
      ...base, value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.reply', version: 0, state: 'granted' },
        { id: 'thread.reopen', version: 0, state: 'granted' },
      ] },
    })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: {
      items: [{ kind: 'message', id: messageId, actor_id: 'agent/workshop/atlas',
        content: 'The worker tests passed.', accepted_at_unix_ms: 1, thread_quote_id: null }], next_cursor: null,
    } })
    if (url.pathname.endsWith('/actions')) {
      const body = JSON.parse(String(init?.body))
      writes.push(body)
      return Response.json({ ...base, value: {
        kind: 'action-result', action_id: body.id, operation_id: 'operation/reply',
        status: 'completed', affected_ids: [id, 'message/review-reply'], snapshot_id: base.snapshot.id,
      } })
    }
    return Response.json({ ...base, value: {
      id, agent_id: 'agent/workshop/atlas', title: 'Worker tests', title_revision: 'claim/title',
      revision: 'claim/closed', state: 'closed', unread: false, message_count: 1,
    } })
  } })
  const registry = AtomRegistry.make()
  const threads = gatewayThreads(client, registry)
  registry.set(threads.draft(id), { content: 'Keep this confirmation with the completed discussion.' })
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], agents: [], missions: [], attention: [],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveThreadPane threadId={id} /></DataSourceProvider>))
    await vi.waitFor(() => expect(host.querySelector('textarea'), host.textContent ?? '').not.toBeNull())
    const quote = [...host.querySelectorAll('button')].find(button => button.getAttribute('aria-label')?.startsWith('Reply to '))
    expect(quote).toBeDefined()
    await act(async () => quote?.click())
    expect(host.textContent).toContain('The worker tests passed.')
    const send = [...host.querySelectorAll('button')].find(button => button.getAttribute('aria-label') === 'Send' || button.textContent === 'Send')
    expect(send).toBeDefined()
    await act(async () => send?.click())
    await vi.waitFor(() => expect(writes).toHaveLength(1))
    expect(writes[0]).toMatchObject({ type: 'thread.reply', parameters: {
      target_id: id, content: 'Keep this confirmation with the completed discussion.', thread_quote_id: messageId,
    } })
    expect(host.textContent).toContain('Closed')
    expect(host.textContent).toContain('Reopen thread')
    expect(host.querySelector('textarea')?.value).toBe('')
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    vi.unstubAllGlobals()
  }
})
