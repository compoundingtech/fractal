// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
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

it.each([
  { connection: 'local browser', local: true, browserGrant: false, canStart: true, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 2, episode: 'claim/episode' },
  { connection: 'approved paired browser', local: false, browserGrant: true, canStart: true, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 2, episode: 'claim/episode' },
  { connection: 'legacy paired device', local: false, browserGrant: false, canStart: false, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 2, episode: 'claim/episode' },
  { connection: 'custom Inbox source', local: true, browserGrant: false, canStart: true, kind: 'custom.garden.review', sourceId: 'custom/garden/review/origin', responsible: true, protocol: 2, episode: 'claim/episode' },
  { connection: 'unassigned Inbox source', local: true, browserGrant: false, canStart: false, kind: 'custom.garden.review', sourceId: 'custom/garden/review/origin', responsible: false, protocol: 2, episode: 'claim/episode' },
  { connection: 'old thread protocol', local: true, browserGrant: false, canStart: false, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 1, episode: 'claim/episode' },
  { connection: 'cached row without an episode', local: true, browserGrant: false, canStart: false, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 2, episode: undefined },
])('offers an inline Inbox composer only when $connection supports unsigned sends', async ({ local, browserGrant, canStart, kind, sourceId, responsible, protocol, episode }) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const base = fixture.capabilities
  const ask = decodeUnknownSync(Attention)({
    id: 'attention/review', kind: 'attention', revision: 'claim/episode', ...(episode === undefined ? {} : { episode }),
    person_id: 'person/operator', source_id: sourceId, requester_id: 'person/operator', ...(responsible ? { conversation_id: 'agent/example/atlas' } : {}),
    attention_kind: kind, title: 'Review PR #42', detail: 'Can we merge?', priority: 'high',
    state: 'open', alert: true, actions: ['work.done'], thread_unread: true,
    ...(kind === 'agent-request' ? { request: {
      version: 1, type: 'decision', question: 'Can we merge?', why_person: 'You own the release.',
      answers: [
        { id: 'merge', label: 'Merge', consequence: 'It joins the queue.', outcome: 'accept' },
        { id: 'hold', label: 'Hold', consequence: 'The builder waits.', outcome: 'decline' },
      ],
    } } : {}),
    requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z',
  })
  const requests: URL[] = []
  const writes: Array<{ type: string; parameters: { target_id: string; title: string } }> = []
  let createdId: string | undefined
  const summary = (id: string) => ({ id, agent_id: 'agent/example/atlas', title: 'Review PR #42',
    title_revision: 'claim/title', revision: 'claim/current', state: 'open', message_count: 1, unread: false })
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    requests.push(url)
    if (url.pathname.endsWith('/capabilities')) return Response.json({
      ...base, value: { ...base.value, transport: local ? 'local-web' : base.value.transport, capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: protocol, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.start', version: 0, state: 'granted' },
        { id: 'thread.reply', version: 0, state: 'granted' },
        ...(browserGrant ? [{ id: 'thread-browser-send', version: 1, state: 'granted' }] : []),
      ] },
    })
    if (url.pathname.endsWith('/actions')) {
      const action = JSON.parse(String(init?.body))
      writes.push(action)
      createdId = action.parameters.target_id
      return Response.json({ ...base, value: { kind: 'action-result', action_id: action.id,
        operation_id: 'operation/start', status: 'completed', affected_ids: [createdId, 'message/first'], snapshot_id: base.snapshot.id } })
    }
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [{
      kind: 'message', id: 'message/first', actor_id: base.value.session_actor,
      content: 'Discuss this review before answering.', accepted_at_unix_ms: 1, thread_quote_id: null,
    }], next_cursor: null } })
    if (createdId !== undefined && url.pathname.startsWith('/v1/client/threads/')) return Response.json({ ...base, value: summary(createdId) })
    return Response.json({ ...base, value: { items: createdId === undefined || url.searchParams.get('recently_closed') === 'true' ? [] : [summary(createdId)], next_cursor: null } })
  } })
  const registry = AtomRegistry.make()
  const answer = vi.fn(async () => ({ _tag: 'Sent' as const }))
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], agents: [], missions: [], attention: [ask],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads: gatewayThreads(client, registry), alertActions: { act: answer } }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const open = vi.fn()
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><Inbox onOpen={open} /></DataSourceProvider>))
    const row = host.querySelector<HTMLButtonElement>('[data-alert-id="attention/review"] button[data-alert-open]')
    expect(row).not.toBeNull()
    expect(row?.getAttribute('aria-label')).toContain('unread thread replies')
    expect(host.querySelectorAll('[data-alert-id]')).toHaveLength(1)
    await act(async () => row?.click())
    if (episode !== undefined && protocol >= 2) {
      await vi.waitFor(() => expect(requests.some(url =>
        url.searchParams.get('origin_source') === sourceId && url.searchParams.get('origin_episode') === episode)).toBe(true))
    } else {
      expect(requests.some(url => url.searchParams.has('origin_source'))).toBe(false)
    }
    const button = Array.from(host.querySelectorAll('button')).find(each => each.textContent === 'Start thread')
    if (!canStart) {
      expect(button).toBeUndefined()
      if (!responsible) expect(host.textContent).toContain('No responsible agent is configured')
      expect(host.querySelector('[aria-label="Message composer, Review PR #42"]')).toBeNull()
      return
    }
    expect(button).toBeDefined()
    await act(async () => button?.click())
    const composer = host.querySelector('[data-testid="alert-details"] [aria-label="Message composer, Review PR #42"]')
    expect(composer).not.toBeNull()
    let panel = composer?.closest('section')
    expect(panel?.getAttribute('aria-label')).toBe('Review PR #42')
    expect(document.activeElement).toBe(panel?.querySelector('textarea'))
    expect(Array.from(panel?.querySelectorAll('button') ?? []).some(each => each.textContent === 'Send' || each.getAttribute('aria-label') === 'Send')).toBe(true)
    if (kind === 'agent-request') expect(host.querySelector('[data-testid="alert-details"] [data-alert-kind="agent-request"]')?.textContent).toContain('Merge')
    expect(answer).not.toHaveBeenCalled()
    expect(open).not.toHaveBeenCalled()
    if (kind === 'custom.garden.review') {
      const input = panel?.querySelector('textarea')
      expect(input).not.toBeNull()
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, 'Discuss this review before answering.')
        input?.dispatchEvent(new Event('input', { bubbles: true }))
      })
      const hide = [...(panel?.querySelectorAll('button') ?? [])].find(button => button.textContent === 'Hide draft')
      await act(async () => hide?.click())
      expect(host.querySelector('textarea')).toBeNull()
      const restart = [...host.querySelectorAll('button')].find(button => button.textContent === 'Start thread')
      await act(async () => restart?.click())
      panel = host.querySelector('[aria-label="Message composer, Review PR #42"]')?.closest('section')
      expect(panel?.querySelector('textarea')?.value).toBe('Discuss this review before answering.')
      const send = [...(panel?.querySelectorAll('button') ?? [])].find(button => button.getAttribute('aria-label') === 'Send' || button.textContent === 'Send')
      await act(async () => send?.click())
      await vi.waitFor(() => expect(writes).toHaveLength(1))
      expect(writes[0]).toMatchObject({ type: 'thread.start', parameters: {
        agent_id: 'agent/example/atlas', origin: { source_id: sourceId, episode },
        title: 'Review PR #42', content: 'Discuss this review before answering.',
      } })
      await vi.waitFor(() => expect(host.querySelector('textarea')?.value).toBe(''))
      expect(host.querySelectorAll('[data-thread-panel]')).toHaveLength(1)
      await vi.waitFor(() => expect(host.textContent).toContain('Discuss this review before answering.'))
      expect(answer).not.toHaveBeenCalled()
      expect(open).not.toHaveBeenCalled()
    }
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    vi.unstubAllGlobals()
  }
})
