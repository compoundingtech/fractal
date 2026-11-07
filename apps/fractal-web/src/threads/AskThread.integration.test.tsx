// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import { Attention, decodeUnknownSync } from '@fractal/client/schema/read'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { AlertsTab } from '../web/AlertsTab.tsx'
import { gatewayThreads } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it('opens an active Inbox ask’s first composer inline with the ordinary Send control', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  const base = fixture.capabilities
  const ask = decodeUnknownSync(Attention)({
    id: 'attention/review', kind: 'attention', revision: 'claim/episode', episode: 'claim/episode',
    person_id: 'person/operator', source_id: 'step-run/review', requester_id: 'agent/example/atlas',
    attention_kind: 'agent-request', title: 'Review PR #42', detail: 'Can we merge?', priority: 'high',
    state: 'open', alert: true, actions: ['work.done'],
    request: {
      version: 1, type: 'decision', question: 'Can we merge?', why_person: 'You own the release.',
      answers: [
        { id: 'merge', label: 'Merge', consequence: 'It joins the queue.', outcome: 'accept' },
        { id: 'hold', label: 'Hold', consequence: 'The builder waits.', outcome: 'decline' },
      ],
    },
    requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z',
  })
  const requests: URL[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async input => {
    const url = new URL(String(input))
    requests.push(url)
    if (url.pathname.endsWith('/capabilities')) return Response.json({
      ...base, value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 1, state: 'granted' },
        { id: 'thread.start', version: 0, state: 'granted' },
      ] },
    })
    return Response.json({ ...base, value: { items: [], next_cursor: null } })
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
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><AlertsTab onOpen={open} /></DataSourceProvider>))
    const row = host.querySelector<HTMLButtonElement>('[data-alert-id="attention/review"] button[aria-expanded]')
    expect(row).not.toBeNull()
    await act(async () => row?.click())
    await vi.waitFor(() => expect(requests.some(url =>
      url.searchParams.get('origin_ask') === 'step-run/review' && url.searchParams.get('origin_ask_episode') === 'claim/episode')).toBe(true))
    const button = Array.from(host.querySelectorAll('button')).find(each => each.textContent === 'Start thread')
    expect(button).toBeDefined()
    await act(async () => button?.click())
    const composer = host.querySelector('[data-alert-id="attention/review"] [aria-label="Message composer, Review PR #42"]')
    expect(composer).not.toBeNull()
    const panel = composer?.closest('section')
    expect(panel?.getAttribute('aria-label')).toBe('Review PR #42')
    expect(document.activeElement).toBe(panel?.querySelector('textarea'))
    expect(Array.from(panel?.querySelectorAll('button') ?? []).some(each => each.textContent === 'Send' || each.getAttribute('aria-label') === 'Send')).toBe(true)
    expect(Array.from(host.querySelectorAll('[data-alert-kind="agent-request"] button')).some(each => each.textContent === 'Merge')).toBe(true)
    expect(answer).not.toHaveBeenCalled()
    expect(open).not.toHaveBeenCalled()
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    vi.unstubAllGlobals()
  }
})
