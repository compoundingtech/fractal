// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import { Agent, decodeUnknownSync as decodeNative } from '@fractal/client/schema'
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
  { connection: 'local browser', local: true, browserGrant: false, canStart: true, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 3, episode: 'claim/episode' },
  { connection: 'approved paired browser', local: false, browserGrant: true, canStart: true, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 3, episode: 'claim/episode' },
  { connection: 'legacy paired device', local: false, browserGrant: false, canStart: false, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 3, episode: 'claim/episode' },
  { connection: 'custom Inbox source', local: true, browserGrant: false, canStart: true, kind: 'custom.garden.review', sourceId: 'custom/garden/review/origin', responsible: true, protocol: 3, episode: 'claim/episode' },
  { connection: 'unlisted configured recipient', local: true, browserGrant: false, canStart: true, kind: 'custom.garden.review', sourceId: 'custom/garden/review/origin', responsible: false, protocol: 3, episode: 'claim/episode' },
  { connection: 'old thread protocol', local: true, browserGrant: false, canStart: false, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 1, episode: 'claim/episode' },
  { connection: 'cached row without an episode', local: true, browserGrant: false, canStart: false, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 3, episode: undefined },
  { connection: 'resolved ask', local: true, browserGrant: false, canStart: true, kind: 'agent-request', sourceId: 'step-run/review', responsible: true, protocol: 3, episode: 'claim/episode', resolved: true },
  { connection: 'resolved authored step', local: true, browserGrant: false, canStart: true, kind: 'person-step', sourceId: 'step-run/release/review', responsible: true, protocol: 3, episode: 'claim/step-opening', resolved: true },
  { connection: 'resolved typed notice', local: true, browserGrant: false, canStart: true, kind: 'custom.release.notice', sourceId: 'custom/attention/source/release', responsible: true, protocol: 3, episode: 'claim/notice-opening', resolved: true },
  { connection: 'resolved system notice', local: true, browserGrant: false, canStart: true, kind: 'model-drift', sourceId: 'system/notice/release', responsible: false, protocol: 3, episode: 'claim/system-notice-opening', resolved: true },
  { connection: 'auto-resolved ask', local: true, browserGrant: false, canStart: true, kind: 'person-step', sourceId: 'step-run/release/default', responsible: true, protocol: 3, episode: 'claim/default-opening', resolved: true, autoResolved: true },
])('offers an inline Inbox composer only when $connection supports unsigned sends', async ({ local, browserGrant, canStart, kind, sourceId, responsible, protocol, episode, ...scenario }) => {
  const resolved = 'resolved' in scenario && scenario.resolved === true
  const autoResolved = 'autoResolved' in scenario && scenario.autoResolved === true
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const base = fixture.capabilities
  const ask = decodeUnknownSync(Attention)({
    id: 'attention/review', kind: 'attention', revision: 'claim/episode', ...(episode === undefined ? {} : { episode }),
    person_id: 'person/operator', source_id: sourceId, requester_id: 'person/operator', ...(responsible ? { conversation_id: 'agent/example/atlas' } : {}),
    attention_kind: kind, title: 'Review PR #42', detail: 'Can we merge?', priority: 'high',
    state: resolved ? 'resolved' : 'open', alert: true, actions: resolved ? [] : ['work.done'], thread_unread: true,
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
  const createdIds: string[] = []
  const summary = (id: string) => ({ id, agent_id: id === 'thread/original' ? 'agent/example/original' : 'agent/example/atlas',
    title: id === 'thread/original' ? 'Original discussion' : 'Review PR #42',
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
      createdIds.push(action.parameters.target_id)
      return Response.json({ ...base, value: { kind: 'action-result', action_id: action.id,
        operation_id: 'operation/start', status: 'completed', affected_ids: [createdId, 'message/first'], snapshot_id: base.snapshot.id } })
    }
    if (url.pathname.endsWith('/request-history')) return Response.json({ ...base, value: {
      items: [], boundary_items: [], relations: [], source_episodes: [], complete: true, next_cursor: null, source_support: null,
    } })
    if (url.pathname === '/v1/client/threads/start-context') return Response.json({ ...base, value: {
      kind: 'thread-start-context', source_id: url.searchParams.get('source_id'), episode: url.searchParams.get('episode'),
      start_recipient: { agent_id: 'agent/example/atlas', basis: { _tag: responsible ? 'original' : 'configured' } },
    } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [{
      kind: 'message', id: 'message/first', actor_id: base.value.session_actor,
      content: 'Discuss this review before answering.', accepted_at_unix_ms: 1, thread_quote_id: null,
    }], next_cursor: null } })
    if (url.pathname.startsWith('/v1/client/threads/')) return Response.json({ ...base, value: summary(`thread/${decodeURIComponent(url.pathname.split('/').at(-1)!)}`) })
    return Response.json({ ...base, value: { items: url.searchParams.get('recently_closed') === 'true' ? [] :
      [...(resolved ? [summary('thread/original')] : []), ...createdIds.map(summary)], next_cursor: null } })
  } })
  const registry = AtomRegistry.make()
  const answer = vi.fn(async () => ({ _tag: 'Sent' as const }))
  const source = { ...fixtureSource({ world: {
    now: Date.parse('2026-11-01T11:05:00Z'), events: [], missions: [], attention: autoResolved ? [] : [ask],
    // The roster serves the seat's declared name.
    agents: responsible ? [decodeNative(Agent)({ kind: 'agent', id: 'agent/example/atlas', name: 'atlas', host_id: 'host/example', runtime_ids: [], reachability: 'reachable',
      state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-11-01T11:00:00.000Z' })] : [],
    conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads: gatewayThreads(client, registry), alertActions: { act: answer, autoResolved: async () => ({ _tag: 'Observed' as const,
    asks: autoResolved ? [{ resolution_id: 'a'.repeat(64), title: ask.title, requester_id: 'agent/example/atlas',
      resolved_at: '2026-11-01T11:00:00Z', target_id: sourceId, episode: episode! }] : [], complete: true }),
    override: async () => ({ _tag: 'Sent' as const }) } }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const open = vi.fn()
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><Inbox onOpen={open} /></DataSourceProvider>))
    const selector = autoResolved ? '[data-auto-resolved-id] button' : resolved ? '[data-resolved-alert-id="attention/review"] button' : '[data-alert-id="attention/review"] button[data-alert-open]'
    await vi.waitFor(() => expect(host.querySelector(selector)).not.toBeNull())
    const row = host.querySelector<HTMLButtonElement>(selector)
    expect(row).not.toBeNull()
    if (!resolved) {
      expect(row?.getAttribute('aria-label')).toContain('unread thread replies')
      expect(host.querySelectorAll('[data-alert-id]')).toHaveLength(1)
    }
    await act(async () => row?.click())
    if (episode !== undefined && protocol >= 3) {
      await vi.waitFor(() => expect(requests.some(url =>
        url.searchParams.get('origin_source') === sourceId && url.searchParams.get('origin_episode') === episode)).toBe(true))
    } else {
      expect(requests.some(url => url.searchParams.has('origin_source'))).toBe(false)
    }
    // Start is offered only once the item's thread list is observed.
    const findStart = () => Array.from(host.querySelectorAll('button')).find(each => each.textContent === 'Start thread')
    if (!canStart) {
      await act(async () => {})
      expect(findStart()).toBeUndefined()
      expect(host.querySelector('[aria-label="Message composer, Review PR #42"]')).toBeNull()
      return
    }
    await vi.waitFor(() => expect(findStart()).toBeDefined())
    const button = findStart()
    if (!resolved) expect(host.querySelector('[data-testid="alert-details"]')?.textContent).toContain(`Ask ${responsible ? 'atlas' : 'an agent'} about this item without answering it.`)
    await act(async () => button?.click())
    const composer = host.querySelector('[data-testid="alert-details"] [aria-label="Message composer, Review PR #42"]')
    expect(composer).not.toBeNull()
    let panel = composer?.closest('section')
    expect(panel?.getAttribute('aria-label')).toBe('Review PR #42')
    expect(document.activeElement).toBe(panel?.querySelector('textarea'))
    expect(Array.from(panel?.querySelectorAll('button') ?? []).some(each => each.textContent === 'Send' || each.getAttribute('aria-label') === 'Send')).toBe(true)
    expect(panel?.querySelector<HTMLInputElement>('input')?.value).toBe('Review PR #42')
    expect([...(panel?.querySelectorAll('label') ?? [])].map(label => label.textContent)).toContain('Thread title')
    expect(panel?.textContent).toContain(`To ${responsible ? 'atlas' : 'an agent'}`)
    expect(panel?.querySelector('[data-start-recipient-id]')?.textContent).toBe('agent/example/atlas')
    expect(panel?.querySelector('[data-start-recipient-basis]')?.textContent).toBe(responsible ? 'Original recipient' : 'Configured recipient')
    if (kind === 'agent-request' && !resolved) expect(host.querySelector('[data-testid="alert-details"] [data-alert-kind="agent-request"]')?.textContent).toContain('Merge')
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
      const restart = [...host.querySelectorAll('button')].find(button => button.textContent === 'Continue draft')
      expect(restart).toBeDefined()
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
    if (resolved) {
      for (const content of ['First independent discussion.', 'Second independent discussion.']) {
        const editor = host.querySelector('[data-testid="alert-details"] input')!.closest('section')!
        const input = editor.querySelector('textarea')!
        await act(async () => {
          Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, content)
          input.dispatchEvent(new Event('input', { bubbles: true }))
        })
        const send = [...editor.querySelectorAll('button')].find(button => button.getAttribute('aria-label') === 'Send' || button.textContent === 'Send')!
        await act(async () => send.click())
        await vi.waitFor(() => expect(writes).toHaveLength(content.startsWith('First') ? 1 : 2))
        await vi.waitFor(() => expect(findStart()).toBeDefined())
        if (content.startsWith('First')) await act(async () => findStart()!.click())
      }
      expect(new Set(createdIds).size).toBe(2)
      expect(createdIds).not.toContain('thread/original')
      for (const write of writes) expect(write).toMatchObject({ type: 'thread.start', parameters: {
        agent_id: 'agent/example/atlas', origin: { source_id: sourceId, episode },
      } })
      expect(host.textContent).toContain('Original discussion')
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
