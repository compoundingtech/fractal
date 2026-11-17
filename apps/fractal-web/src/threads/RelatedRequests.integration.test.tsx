// @vitest-environment jsdom
import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads, type ThreadSource } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const base = fixture.capabilities
const agent = 'agent/workshop/atlas'
const summary = { id: 'thread/one', agent_id: agent, title: 'Release approval', title_revision: 'claim/title', revision: 'claim/current', state: 'open', message_count: 0, unread: false }
const source = 'step-run/run-generation-1/release/approve'
const ci = 'step-run/run-generation-1/release/ci'
const scoping = 'step-run/run-generation-1/release/scope'
const abc = 'claim/ask-abc'
const def = 'run-generation/1:2:3'
const ciEpisode = 'claim/ask-ci'
const scopingEpisode = 'claim/ask-scope'
const now = Date.parse('2026-11-17T10:00:00Z')

const key = (source_id: string, episode: string) => ({ source_id, episode })
const runtime = (claim: string) => ({ kind: 'runtime-ask', asking_claim_id: claim })
const available = (title: string | null, text: string, provenance: object, references: readonly object[] = []) =>
  ({ availability: 'available', title, text, request: null, references, provenance })
const item = (source_id: string, episode: string, payload: object, state: { readonly availability: string }) =>
  ({ key: key(source_id, episode), payload, state, complete: state.availability === 'available' })
const status = (value: string, answers: readonly object[] = []) => ({ availability: 'available', status: value, answers })
const historyPage = (page: Partial<Record<'items' | 'relations' | 'source_episodes' | 'boundary_items', readonly object[]>> & { source_support?: string | null; complete?: boolean }) =>
  Response.json({ ...base, value: {
    items: page.items ?? [], relations: page.relations ?? [], source_episodes: page.source_episodes ?? [], boundary_items: page.boundary_items ?? [],
    source_support: page.source_support === undefined ? 'person-step' : page.source_support, next_cursor: null, complete: page.complete ?? true,
  } })

/** The adopted branch: an earlier abc approval, a separate CI request after it, and the same step asked again for def. */
const branch = () => historyPage({
  boundary_items: [item(scoping, scopingEpisode, available('Scope the release', 'Which services ship?', runtime(scopingEpisode)), status('completed'))],
  items: [
    item(source, abc, available('Approve revision abc', 'Approve abc before the release.', runtime(abc), [
      { target_id: 'https://example.test/pull/12', revision: 'abc', semantics: 'retained-reference' },
      { target_id: 'doc/release-notes', revision: null, semantics: 'live-reference' },
    ]), status('completed', [{ ask: source, status: 'completed', summary: 'Approved abc.', respondent: 'person/avery', answered_at_unix_ms: now - 60 * 60_000 }])),
    item(source, def, available('Approve revision abc', 'Approve abc before the release.\nReview def instead.', { kind: 'authored-episode', mission_revision: 'revision/2', opening_claim_id: 'claim/open-def' }), status('ready')),
    item(ci, ciEpisode, available('Run CI on abc', 'Start CI for revision abc.', runtime(ciEpisode)), status('ready')),
  ],
  relations: [
    { predecessor: key(scoping, scopingEpisode), request: key(source, abc), reason: 'Scope settled, so approval was requested.' },
    { predecessor: key(source, abc), request: key(ci, ciEpisode), reason: 'Separate CI request for the approved revision.' },
  ],
  source_episodes: [{ source_id: source, episodes: [key(source, abc), key(source, def)] }],
})

async function mount({ history, withoutReader = false }: { readonly history: (url: URL, reads: number) => Response; readonly withoutReader?: boolean }) {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {}; unobserve() {}; disconnect() {} })
  let historyReads = 0
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async input => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, transport: 'local-web', capabilities: [
      ...base.value.capabilities,
      { id: 'threads-v1', version: 3, state: 'granted' },
      { id: 'thread-summary-feed', version: 1, state: 'granted' },
      { id: 'thread.reply', version: 0, state: 'granted' },
    ] } })
    if (url.pathname.endsWith('/request-history')) return history(url, ++historyReads)
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [], next_cursor: null } })
    return Response.json({ ...base, value: summary })
  } })
  const registry = AtomRegistry.make()
  const live = gatewayThreads(client, registry)
  // A source without the optional reader, as an older connection's would be.
  const { requestHistory, reloadRequestHistory, ...withoutHistory } = live
  const threads: ThreadSource = withoutReader ? withoutHistory : live
  const sourceWithThreads = { ...fixtureSource({ world: {
    now, events: [], missions: [], attention: [], agents: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  await act(async () => root.render(<DataSourceProvider source={sourceWithThreads} registry={registry}><LiveThreadPane threadId="thread/one" /></DataSourceProvider>))
  const section = () => host.querySelector<HTMLElement>('[data-request-history]')
  const row = (sourceId: string, episode: string) => host.querySelector<HTMLElement>(`[data-request-source="${sourceId}"][data-request-episode="${episode}"]`)
  return {
    host, section, row, reads: () => historyReads,
    dispose: async () => {
      await act(async () => root.unmount())
      registry.dispose()
      host.remove()
      vi.unstubAllGlobals()
    },
  }
}

it('shows each related request’s original wording and its own status, with the reason it was asked after another', async () => {
  const view = await mount({ history: branch })
  try {
    await vi.waitFor(() => expect(view.row(ci, ciEpisode)).not.toBeNull())
    expect(view.section()?.querySelector('h3')?.textContent).toBe('Related requests')
    const first = view.row(source, abc)!
    expect(first.querySelector('[data-request-title]')?.textContent).toBe('Approve revision abc')
    expect(first.querySelector('[data-request-text]')?.textContent).toBe('Approve abc before the release.')
    expect(first.querySelector('[data-request-status-label]')?.textContent).toBe('Completed')
    expect(first.querySelector('[data-request-answer]')?.textContent).toContain('Approved abc.')
    expect(first.querySelector('[data-request-answer]')?.textContent).toContain('Answered by avery')
    // A completed predecessor never answers the separate request asked after it.
    const after = view.row(ci, ciEpisode)!
    expect(after.querySelector('[data-request-status-label]')?.textContent).toBe('Open')
    expect(after.querySelector('[data-request-after]')?.textContent).toBe('Requested after “Approve revision abc”Separate CI request for the approved revision.')
    expect(after.querySelector('[data-request-answer]')).toBeNull()
    // Predecessors come first; the boundary predecessor from an earlier page is shown, not counted.
    const order = [...view.host.querySelectorAll('[data-request-episode]')].map(each => each.getAttribute('data-request-episode'))
    expect(order.indexOf(scopingEpisode)).toBeLessThan(order.indexOf(abc))
    expect(order.indexOf(abc)).toBeLessThan(order.indexOf(ciEpisode))
    // The conversation is still the thread's own.
    expect(view.host.querySelector('textarea')).not.toBeNull()
  } finally { await view.dispose() }
})

it('keeps same-source episodes together, each with its own status and wording', async () => {
  const view = await mount({ history: branch })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('[data-request-same-source]')).not.toBeNull())
    const group = view.host.querySelector<HTMLElement>(`[data-request-same-source="${source}"]`)!
    expect([...group.querySelectorAll('[data-request-episode]')].map(each => each.getAttribute('data-request-episode'))).toEqual([abc, def])
    expect(view.row(source, abc)?.querySelector('[data-request-status-label]')?.textContent).toBe('Completed')
    expect(view.row(source, def)?.querySelector('[data-request-status-label]')?.textContent).toBe('Open')
    expect(view.row(source, def)?.querySelector('[data-request-text]')?.textContent).toBe('Approve abc before the release.\nReview def instead.')
    // Episodes of one source are grouped, never linked as predecessor and successor.
    expect(view.row(source, def)?.querySelector('[data-request-after]')).toBeNull()
  } finally { await view.dispose() }
})

it('moves to the predecessor when Requested after is pressed', async () => {
  const view = await mount({ history: branch })
  try {
    await vi.waitFor(() => expect(view.row(ci, ciEpisode)).not.toBeNull())
    const link = view.row(ci, ciEpisode)!.querySelector<HTMLButtonElement>('[data-request-after] button')!
    await act(async () => link.click())
    expect(document.activeElement).toBe(view.row(source, abc))
  } finally { await view.dispose() }
})

it('keeps references as references: a pinned revision and a live target that may have changed', async () => {
  const view = await mount({ history: branch })
  try {
    await vi.waitFor(() => expect(view.row(source, abc)).not.toBeNull())
    const references = [...view.row(source, abc)!.querySelectorAll('[data-request-reference]')]
    expect(references.map(each => each.getAttribute('data-request-reference'))).toEqual(['retained-reference', 'live-reference'])
    expect(references[0]?.querySelector('a')?.getAttribute('href')).toBe('https://example.test/pull/12')
    expect(references[0]?.textContent).toContain('at revision abc')
    expect(references[1]?.querySelector('a')).toBeNull()
    expect(references[1]?.textContent).toContain('Live, may have changed since')
  } finally { await view.dispose() }
})

it('marks a missing original request unavailable and never shows today’s text in its place', async () => {
  const view = await mount({ history: () => historyPage({
    complete: false,
    items: [
      item(source, abc, { availability: 'unavailable', reason: 'payload-unavailable' }, { availability: 'unavailable' }),
      item(source, def, available('Review def instead', 'Review def instead.', runtime(def)), status('ready')),
    ],
    source_episodes: [{ source_id: source, episodes: [key(source, abc), key(source, def)] }],
  }) })
  try {
    await vi.waitFor(() => expect(view.row(source, abc)).not.toBeNull())
    const missing = view.row(source, abc)!
    expect(missing.getAttribute('data-request-payload')).toBe('unavailable')
    expect(missing.querySelector('[data-request-title]')?.textContent).toBe('Original request unavailable')
    expect(missing.querySelector('[data-request-text]')?.textContent).toBe('Fractal no longer has this request’s original wording.')
    expect(missing.querySelector('[data-request-status-label]')?.textContent).toBe('Status unknown')
    expect(missing.textContent).not.toContain('def')
    expect(view.host.querySelector('[data-request-history-incomplete]')?.textContent).toBe('Some of this history could not be read, so requests may be missing.')
  } finally { await view.dispose() }
})

it('draws no arrow, reason or identifier for a predecessor the reader did not return', async () => {
  const hidden = 'step-run/run-generation-1/private/ask'
  const view = await mount({ history: () => historyPage({
    items: [
      item(source, abc, available('Approve revision abc', 'Approve abc.', runtime(abc)), status('completed')),
      item(ci, ciEpisode, available('Run CI on abc', 'Start CI.', runtime(ciEpisode)), status('ready')),
    ],
    relations: [{ predecessor: key(hidden, 'claim/private'), request: key(ci, ciEpisode), reason: 'Private reason' }],
  }) })
  try {
    await vi.waitFor(() => expect(view.row(ci, ciEpisode)).not.toBeNull())
    expect(view.host.querySelector('[data-request-after]')).toBeNull()
    expect(view.host.innerHTML).not.toContain(hidden)
    expect(view.host.innerHTML).not.toContain('claim/private')
    expect(view.host.textContent).not.toContain('Private reason')
  } finally { await view.dispose() }
})

it('shows a refused history read without any request identity and keeps the conversation', async () => {
  const view = await mount({ history: () => Response.json({
    api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
    code: 'forbidden', message: 'This request history is not available to you.', retryable: false, details: {},
  }, { status: 403 }) })
  try {
    await vi.waitFor(() => expect(view.section()?.querySelector('[data-system-state="error"]')).not.toBeNull())
    expect(view.section()?.innerHTML).not.toContain('step-run/')
    expect(view.section()?.querySelector('[data-request-episode]')).toBeNull()
    expect(view.host.querySelector('textarea')).not.toBeNull()
  } finally { await view.dispose() }
})

it('offers Retry after a failed history read and shows the history once it is read again', async () => {
  const view = await mount({ history: (_url, reads) => reads === 1
    ? Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/failed', code: 'internal', message: 'Read failed.', retryable: true, details: {} }, { status: 500 })
    : branch() })
  try {
    const retry = await vi.waitFor(() => {
      const found = [...(view.section()?.querySelectorAll('button') ?? [])].find(each => each.textContent === 'Retry')
      if (found === undefined) throw new Error('Retry is not offered yet')
      return found
    })
    await act(async () => retry.click())
    await vi.waitFor(() => expect(view.row(ci, ciEpisode)).not.toBeNull())
    expect(view.reads()).toBeGreaterThanOrEqual(2)
  } finally { await view.dispose() }
})

it('says Fractal keeps no request history for an unsupported source', async () => {
  const view = await mount({ history: () => historyPage({ source_support: 'unsupported-source' }) })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('[data-request-history-unsupported]')).not.toBeNull())
    expect(view.host.querySelector('[data-request-history-unsupported]')?.textContent).toBe('Fractal keeps no request history for this item’s source.')
    expect(view.section()?.querySelector('[data-request-episode]')).toBeNull()
  } finally { await view.dispose() }
})

it('shows nothing for a lone complete origin request or a thread without history', async () => {
  for (const history of [
    () => historyPage({ items: [item(source, abc, available('Approve revision abc', 'Approve abc.', runtime(abc)), status('ready'))] }),
    () => historyPage({ source_support: null }),
  ]) {
    const view = await mount({ history })
    try {
      await vi.waitFor(() => expect(view.reads()).toBeGreaterThan(0))
      await vi.waitFor(() => expect(view.host.querySelector('textarea')).not.toBeNull())
      await vi.waitFor(() => expect(view.section()).toBeNull())
    } finally { await view.dispose() }
  }
})

it('omits the section when the source has no request history reader', async () => {
  const view = await mount({ history: branch, withoutReader: true })
  try {
    await vi.waitFor(() => expect(view.host.querySelector('textarea')).not.toBeNull())
    expect(view.section()).toBeNull()
    expect(view.reads()).toBe(0)
  } finally { await view.dispose() }
})
