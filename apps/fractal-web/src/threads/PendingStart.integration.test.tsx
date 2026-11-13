// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { observed, type Feed } from '../data/source.ts'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { AlertDetailsPane } from '../web/AlertDetailsPane.tsx'
import { AlertsTab } from '../web/AlertsTab.tsx'
import { gatewayThreads } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

/** The Inbox as the workspace composes it: the sidebar list, and the selected alert's view with its threads. */
const Inbox = ({ onOpen }: { readonly onOpen: (ref: string) => void }) => {
  const [selected, setSelected] = React.useState<string>()
  return <>
    <AlertsTab onOpen={onOpen} {...(selected === undefined ? {} : { selected })} onSelect={setSelected} />
    {selected === undefined ? null : <AlertDetailsPane id={selected} onOpenAlert={setSelected} onOpenRef={onOpen} onClose={() => setSelected(undefined)} onBack={() => setSelected(undefined)} />}
  </>
}

type PostedStart = {
  id: string
  idempotency_key: string
  parameters: { target_id: string; agent_id: string; origin: { source_id: string; episode: string }; title: string; content: string }
}

it.each(['metadata', 'episode', 'closed', 'removed'] as const)('recovers the original first send after %s changes and the Inbox remounts', async change => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const base = fixture.capabilities
  const sourceId = 'custom/garden/review/origin'
  const originalAgent = 'agent/example/original'
  const content = 'Discuss this before answering.'
  const raw = {
    id: 'attention/review', kind: 'attention', revision: 'claim/original', episode: 'claim/original',
    person_id: 'person/operator', source_id: sourceId, conversation_id: originalAgent,
    attention_kind: 'custom.garden.review', title: 'Original review', detail: 'Review the worker tests.',
    priority: 'normal', state: 'open', alert: true, actions: [],
    requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z',
  }
  const initial = decodeUnknownSync(Attention)(raw)
  const attention = Atom.keepAlive(Atom.make<Feed<readonly Attention[]>>(observed({ value: [initial] })))
  const writes: PostedStart[] = []
  let loseFirst!: () => void
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: {
      ...base.value, transport: 'local-web', capabilities: [...base.value.capabilities,
        { id: 'threads-v1', version: 2, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.start', version: 0, state: 'granted' },
        { id: 'thread.reply', version: 0, state: 'granted' },
      ],
    } })
    if (url.pathname.endsWith('/actions')) {
      const action = JSON.parse(String(init?.body)) as PostedStart
      writes.push(action)
      if (writes.length === 1) return new Promise<Response>((_resolve, reject) => { loseFirst = () => reject(new TypeError('The committed receipt was lost')) })
      return Response.json({ ...base, value: { kind: 'action-result', action_id: action.id, operation_id: 'operation/start',
        status: 'completed', affected_ids: [action.parameters.target_id, 'message/first'], snapshot_id: base.snapshot.id } })
    }
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [{
      kind: 'message', id: 'message/first', actor_id: base.value.session_actor, content,
      accepted_at_unix_ms: 1, thread_quote_id: null,
    }], next_cursor: null } })
    const first = writes[0]
    const summary = first === undefined ? undefined : {
      id: first.parameters.target_id, agent_id: originalAgent, title: 'Original review',
      title_revision: 'claim/title', revision: 'claim/created', state: 'open', message_count: 1, unread: false,
    }
    if (url.pathname.startsWith('/v1/client/threads/')) return Response.json({ ...base, value: summary })
    const matches = url.searchParams.get('origin_episode') === 'claim/original'
    return Response.json({ ...base, value: { items: summary === undefined || !matches ? [] : [summary], next_cursor: null } })
  } })
  const registry = AtomRegistry.make()
  const threads = gatewayThreads(client, registry)
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], agents: [], missions: [], attention: [],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads, attention }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const open = vi.fn()
  const render = () => act(async () => root.render(<DataSourceProvider source={source} registry={registry}><Inbox onOpen={open} /></DataSourceProvider>))
  const button = (name: string) => [...host.querySelectorAll('button')].find(row => row.textContent === name)
  const expand = () => act(async () => host.querySelector<HTMLButtonElement>('[data-alert-id="attention/review"] button[data-alert-open]')?.click())
  try {
    await render()
    await expand()
    await vi.waitFor(() => expect(button('Start thread')).toBeDefined())
    await act(async () => button('Start thread')?.click())
    const input = host.querySelector('textarea')
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, content)
      input?.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => [...host.querySelectorAll('button')].find(row => row.getAttribute('aria-label') === 'Send')?.click())
    await vi.waitFor(() => expect(writes).toHaveLength(1))
    expect(button('Retry thread start')?.disabled).toBe(true)
    await act(async () => loseFirst())
    await vi.waitFor(() => expect(button('Retry thread start')?.disabled, host.textContent ?? '').toBe(false))
    expect(host.querySelectorAll('[data-thread-panel]')).toHaveLength(1)

    const replacement = decodeUnknownSync(Attention)({ ...raw, title: 'Current review title', conversation_id: 'agent/example/replacement',
      ...(change === 'episode' ? { episode: 'claim/replacement', revision: 'claim/replacement' } : {}),
      ...(change === 'closed' ? { state: 'resolved' } : {}),
    })
    await act(async () => registry.set(attention, observed({ value: change === 'removed' ? [] : [replacement] })))
    await act(async () => root.render(null))
    await render()
    if (change === 'metadata') await expand()
    if (change === 'removed' || change === 'closed') expect(host.querySelector('[data-alert-id="attention/review"]')).toBeNull()
    else expect(host.textContent).toContain('Current review title')
    await vi.waitFor(() => expect(button('Retry thread start')?.disabled, host.textContent ?? '').toBe(false))
    expect(host.textContent).toContain('Original review')
    expect(host.textContent).toContain(`To ${originalAgent}`)
    expect(host.textContent).toContain(content)
    expect(host.querySelectorAll('[aria-label="Thread recovery"]')).toHaveLength(change === 'metadata' ? 0 : 1)
    await act(async () => button('Retry thread start')?.click())
    await vi.waitFor(() => expect(writes).toHaveLength(2))
    expect(writes[1]?.id).toBe(writes[0]?.id)
    expect(writes[1]?.idempotency_key).toBe(writes[0]?.idempotency_key)
    expect(writes[1]?.parameters).toEqual(writes[0]?.parameters)
    await vi.waitFor(() => expect(host.querySelector('textarea')?.value).toBe(''))
    expect(button('Retry thread start')).toBeUndefined()
    expect(host.querySelectorAll('[data-thread-panel]')).toHaveLength(1)
    expect(open).not.toHaveBeenCalled()
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    vi.unstubAllGlobals()
  }
})
