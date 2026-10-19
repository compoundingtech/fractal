// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import type { Attention, Mission, TerminalScreen } from '@smalltalk/st3-client/schema'
import type { ConnectionState } from '@st3/sdk/effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as Option from 'effect/Option'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { initialFeedSync, type FeedSync, type FeedSyncObservation } from '../data/feedSync.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { observed, unavailable, waiting, type ConversationPage, type DataSource, type Feed } from '../data/source.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
// Evaluate the real pane before mounting the workspace's lazy boundary.
import './ConversationPane.tsx'

// Only the build-time CSS runtime and absent jsdom layout APIs are substituted.
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const observedAt = 100_000
const agent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/example', name: 'Example Agent', host_id: 'host/build-1', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'working', blocked_on: null, fault: null, revision: '1', updated_at: '1970-01-01T00:00:01.000Z' })
const page: ConversationPage = {
  items: [
    { _tag: 'Text', id: 'prompt/example', role: 'user', text: 'Keep the observed conversation visible.', attachments: [], streaming: false, at: '1970-01-01T00:00:01.000Z' },
    { _tag: 'Text', id: 'answer/example', role: 'assistant', text: 'The retained answer remains readable.', attachments: [], streaming: false, at: '1970-01-01T00:00:02.000Z' },
    { _tag: 'Status', id: 'status/example', status: 'running', at: '1970-01-01T00:00:03.000Z' },
  ],
  hasOlder: false,
  observation: { empty: false },
}
const liveSync = <TValue,>(value: TValue): FeedSync<TValue> => ({ last: Option.some({ value, observedAt }), sync: { status: { _tag: 'Live', since: observedAt }, observedAt } })
const staleSync = <TValue,>(value: TValue): FeedSync<TValue> => ({
  last: Option.some({ value, observedAt }),
  // Deliberately newer than last.observedAt: the age must measure data, not the failure transition.
  sync: { status: { _tag: 'Stale', reason: { _tag: 'Unknown' } }, observedAt: 117_000 },
})

let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
let source: DataSource
let agents: Atom.Writable<Feed<readonly Agent[]>>
let conversation: Atom.Writable<Feed<ConversationPage>>
let agentsSync: Atom.Writable<FeedSync<readonly Agent[]>>
let conversationSync: Atom.Writable<FeedSync<ConversationPage>>
let gatewaySync: Atom.Writable<FeedSyncObservation>
let connection: Atom.Writable<ConnectionState>
let now: Atom.Writable<number>

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 })
  window.localStorage.clear()
  window.history.replaceState(null, '', '/w/agent/example')
  registry = AtomRegistry.make()
  agents = Atom.make<Feed<readonly Agent[]>>(observed({ value: [agent] })).pipe(Atom.keepAlive)
  conversation = Atom.make<Feed<ConversationPage>>(observed({ value: page })).pipe(Atom.keepAlive)
  agentsSync = Atom.make(liveSync<readonly Agent[]>([agent])).pipe(Atom.keepAlive)
  conversationSync = Atom.make(liveSync(page)).pipe(Atom.keepAlive)
  gatewaySync = Atom.make<FeedSyncObservation>({ status: { _tag: 'Live', since: observedAt }, observedAt }).pipe(Atom.keepAlive)
  connection = Atom.make<ConnectionState>({ _tag: 'Live' }).pipe(Atom.keepAlive)
  now = Atom.make(118_000).pipe(Atom.keepAlive)
  const fixture = fixtureSource({ world: { now: 118_000, gateway: 'gateway.example.invalid', agents: [agent], missions: [], attention: [], events: [], conversations: { [agent.id]: page }, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } })
  source = { ...fixture, agents, conversation: () => conversation, connection, now, sync: {
    gateway: gatewaySync,
    agents: agentsSync,
    missions: Atom.make(initialFeedSync<readonly Mission[]>(0)),
    attention: Atom.make(initialFeedSync<readonly Attention[]>(0)),
    conversation: () => conversationSync,
    terminal: () => Atom.make(initialFeedSync<TerminalScreen>(0)),
  } }
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  registry.dispose()
  host.remove()
  vi.unstubAllGlobals()
  window.history.replaceState(null, '', '/')
})

const mount = async () => {
  // React act drains the actual workspace lazy boundary; no timer or pane mock is needed.
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
}
const sidebar = () => host.querySelector('aside[aria-label="Agents"]')!
const roster = () => sidebar().querySelector('nav[aria-label="Agent roster"]')!
const retry = (within: ParentNode) => [...within.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.trim() === 'Retry')
const transcript = () => host.querySelector('[data-testid="transcript-scroll"], [data-testid="transcript-unavailable"]')!
describe('RM1 workspace source states', () => {
  it('replaces roster loading with an observed empty roster, not a filter no-match', async () => {
    registry.set(agents, waiting)
    registry.set(agentsSync, initialFeedSync(118_000))
    await mount()
    expect(sidebar().querySelector('[data-system-state="loading"]')?.textContent).toContain('Loading agents')
    expect(roster().querySelector('[data-wf-roster-skeleton]')).not.toBeNull()
    expect(sidebar().textContent).not.toContain('No agents match')
    await act(async () => {
      registry.set(agents, observed({ value: [] }))
      registry.set(agentsSync, liveSync<readonly Agent[]>([]))
    })
    expect(sidebar().querySelector('[data-system-state="loading"]')).toBeNull()
    expect(roster().querySelector('[data-wf-roster-skeleton]')).toBeNull()
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(0)
    expect(sidebar().querySelector('[data-roster-empty]')?.textContent).toContain('No agents yet')
    expect(sidebar().textContent).not.toContain('No agents match')
    expect(retry(sidebar())).toBeUndefined()
  })

  it('distinguishes a filter no-match from an empty or failed source and clears it', async () => {
    await mount()
    const input = sidebar().querySelector<HTMLInputElement>('input[aria-label="Search agents"]')!
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'nonexistent-rm1-agent')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(sidebar().querySelector('[data-roster-no-match]')?.textContent).toContain('No agents match “nonexistent-rm1-agent”')
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(0)
    expect(registry.get(agents)).toEqual(observed({ value: [agent] }))
    expect(retry(sidebar())).toBeUndefined()
    const clear = [...sidebar().querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === 'Clear filter')!
    await act(async () => clear.click())
    expect(input.value).toBe('')
    expect(roster().textContent).toContain(agent.name)
    expect(sidebar().textContent).not.toContain('No agents match')
  })

  it('keeps conversation loading distinct from an authoritatively observed empty page', async () => {
    registry.set(conversation, waiting)
    registry.set(conversationSync, initialFeedSync(118_000))
    await mount()
    expect(host.querySelector('[data-testid="transcript-placeholder"][aria-label="Loading conversation"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="transcript-empty"]')).toBeNull()
    expect(retry(transcript())).toBeUndefined()
    const empty: ConversationPage = { items: [], hasOlder: false, observation: { empty: true } }
    await act(async () => {
      registry.set(conversation, observed({ value: empty }))
      registry.set(conversationSync, liveSync(empty))
    })
    expect(host.querySelector('[data-testid="transcript-placeholder"]')).toBeNull()
    expect(host.querySelector('[data-testid="transcript-empty"]')).not.toBeNull()
    expect(host.textContent).not.toContain('Conversation unavailable')
    expect(retry(transcript())).toBeUndefined()
  })

  it('recovers a roster fetch failure through the supported source Retry action', async () => {
    const retryAgents = vi.fn(() => {
      registry.set(agents, waiting)
      registry.set(agentsSync, initialFeedSync(118_000, 2))
    })
    source = { ...source, retryAgents }
    registry.set(agents, unavailable({ reason: 'failed', detail: 'private roster transport diagnostic' }))
    await mount()
    expect(sidebar().querySelector('[data-system-state="error"]')?.textContent).toContain('Couldn’t load agents')
    expect(sidebar().textContent).not.toContain('private roster transport diagnostic')
    expect(sidebar().textContent).not.toContain('No agents match')
    expect(retry(sidebar())).toBeDefined()
    await act(async () => retry(sidebar())!.click())
    expect(retryAgents).toHaveBeenCalledOnce()
    expect(sidebar().querySelector('[data-system-state="loading"]')).not.toBeNull()
    await act(async () => {
      registry.set(agents, observed({ value: [agent] }))
      registry.set(agentsSync, liveSync<readonly Agent[]>([agent]))
    })
    expect(roster().textContent).toContain(agent.name)
    expect(sidebar().querySelector('[data-system-state="error"]')).toBeNull()
    expect(retry(sidebar())).toBeUndefined()
  })

  it('recovers a conversation fetch failure without routing Retry to the roster', async () => {
    const retryAgents = vi.fn()
    const retryConversation = vi.fn((_ref: string) => {
      registry.set(conversation, waiting)
      registry.set(conversationSync, initialFeedSync(118_000, 2))
    })
    source = { ...source, retryAgents, retryConversation }
    registry.set(conversation, unavailable({ reason: 'failed', detail: 'private conversation transport diagnostic' }))
    await mount()
    expect(host.querySelector('[data-wf-unavailable="failed"]')).not.toBeNull()
    expect(host.textContent).toContain('Conversation unavailable')
    expect(host.textContent).not.toContain('private conversation transport diagnostic')
    expect(retry(transcript())).toBeDefined()
    await act(async () => retry(transcript())!.click())
    expect(retryConversation).toHaveBeenCalledExactlyOnceWith(agent.id)
    expect(retryAgents).not.toHaveBeenCalled()
    expect(host.querySelector('[data-testid="transcript-placeholder"]')).not.toBeNull()
    await act(async () => {
      registry.set(conversation, observed({ value: page }))
      registry.set(conversationSync, liveSync(page))
    })
    expect(host.querySelector('[data-wf-unavailable="failed"]')).toBeNull()
    expect(transcript().textContent).toContain('The retained answer remains readable.')
    expect(host.querySelector('[data-testid="transcript-placeholder"]')).toBeNull()
    expect(retry(transcript())).toBeUndefined()
  })

  it('does not invent Retry when neither failed feed supplies a retry port', async () => {
    registry.set(agents, unavailable({ reason: 'failed', detail: 'roster read failed' }))
    registry.set(conversation, unavailable({ reason: 'failed', detail: 'conversation read failed' }))
    await mount()
    expect(sidebar().querySelector('[data-system-state="error"]')).not.toBeNull()
    expect(host.querySelector('[data-wf-unavailable="failed"]')).not.toBeNull()
    expect(retry(sidebar())).toBeUndefined()
    expect(retry(transcript())).toBeUndefined()
  })

  it('uses retained observation time and the shared clock for stale roster and conversation ages', async () => {
    registry.set(agents, observed({ value: [agent], freshness: 'stale' }))
    registry.set(agentsSync, staleSync<readonly Agent[]>([agent]))
    registry.set(conversation, observed({ value: page, freshness: 'stale' }))
    registry.set(conversationSync, staleSync(page))
    await mount()
    expect(sidebar().textContent).toContain('Last observed 18 s ago')
    expect(host.textContent).toContain('Last seen working · 18 s ago')
    expect(host.querySelector('[data-last-seen]')?.textContent).toContain('Last seen working · 18 s ago')
    expect(host.querySelector('[data-testid="run-progress"]')).toBeNull()
    expect(roster().querySelector('[data-status="working"][data-frozen="true"]')).not.toBeNull()
    expect(transcript().textContent).toContain('The retained answer remains readable.')
    const retainedAgents = registry.get(agents)
    const retainedConversation = registry.get(conversation)
    await act(async () => registry.set(now, 123_000))
    expect(sidebar().textContent).toContain('Last observed 23 s ago')
    expect(host.textContent).toContain('Last seen working · 23 s ago')
    expect(host.textContent).not.toContain('Last seen working · 18 s ago')
    expect(registry.get(agents)).toBe(retainedAgents)
    expect(registry.get(conversation)).toBe(retainedConversation)
    await act(async () => {
      registry.set(agents, observed({ value: [agent] }))
      registry.set(agentsSync, liveSync<readonly Agent[]>([agent]))
      registry.set(conversation, observed({ value: page }))
      registry.set(conversationSync, liveSync(page))
    })
    expect(sidebar().textContent).not.toContain('Last observed')
    expect(host.textContent).not.toContain('Last seen working')
    expect(transcript().textContent).toContain('The retained answer remains readable.')
  })

  it('keeps a disconnected in-progress run completion-unverified after a tool has completed', async () => {
    const inProgress: ConversationPage = {
      ...page,
      items: [
        page.items[0]!,
        { _tag: 'ToolCall', id: 'tool/completed', callId: 'call/completed', name: 'bash', input: { command: 'pnpm test' }, status: 'success', callSeen: true, result: { content: 'Tests passed', mediaType: 'text/plain', isError: false, at: '1970-01-01T00:00:02.000Z' }, at: '1970-01-01T00:00:01.500Z' },
        ...page.items.slice(1),
      ],
    }
    registry.set(conversation, observed({ value: inProgress }))
    registry.set(conversationSync, liveSync(inProgress))
    await mount()
    expect(transcript().textContent).toContain('Run running')
    await act(async () => registry.set(connection, { _tag: 'Connecting', attempt: 2 }))
    const log = transcript().querySelector('[data-testid="work-log"]')!
    expect(log.textContent).toContain('Work · completion unverified')
    expect(log.textContent).not.toContain('Worked')
    expect(log.querySelector('[data-testid="work-log-children"]')).not.toBeNull()
    expect(log.querySelector('[data-tool-status="success"]')).not.toBeNull()
    expect(log.querySelector('button[aria-expanded="true"]')).not.toBeNull()
    expect(host.textContent).toContain('Last seen working · 18 s ago')
    expect(transcript().textContent).not.toContain('Run running')
    expect(host.querySelector('[data-testid="run-progress"]')).toBeNull()
  })

  it('freezes the working cue on disconnect without requiring replacement feed values', async () => {
    await mount()
    const retainedAgents = registry.get(agents)
    const retainedConversation = registry.get(conversation)
    await act(async () => {
      registry.set(connection, { _tag: 'Connecting', attempt: 2 })
      registry.set(gatewaySync, { status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: 120_000, issue: 'synthetic socket closed' } }, observedAt: 117_000 })
    })
    expect(host.querySelector('[data-connection-state="reconnecting"]')).not.toBeNull()
    expect(sidebar().textContent).toContain('Last observed 18 s ago')
    expect(host.textContent).toContain('Last seen working · 18 s ago')
    expect(roster().querySelector('[data-observation-freshness="stale"]')).not.toBeNull()
    expect(transcript().textContent).toContain('The retained answer remains readable.')
    expect(registry.get(agents)).toBe(retainedAgents)
    expect(registry.get(conversation)).toBe(retainedConversation)
    await act(async () => {
      registry.set(connection, { _tag: 'Live' })
      registry.set(gatewaySync, { status: { _tag: 'Live', since: 118_000 }, observedAt: 118_000 })
    })
    expect(host.querySelector('[data-connection-state]')).toBeNull()
    expect(host.textContent).not.toContain('Last seen working')
    expect(roster().querySelector('[data-observation-freshness="live"]')).not.toBeNull()
  })

  it('never presents an idle retained observation as last seen working', async () => {
    const idle: Agent = { ...agent, harness_state: Option.some('idle') }
    registry.set(agents, observed({ value: [idle], freshness: 'stale' }))
    registry.set(agentsSync, staleSync<readonly Agent[]>([idle]))
    const idlePage: ConversationPage = { ...page, items: [...page.items.slice(0, -1), { _tag: 'Status', id: 'status/idle', status: 'completed', at: '1970-01-01T00:00:04.000Z' }] }
    registry.set(conversation, observed({ value: idlePage, freshness: 'stale' }))
    registry.set(conversationSync, staleSync(idlePage))
    await mount()
    expect(sidebar().textContent).toContain('Last observed 18 s ago')
    expect(host.textContent).not.toContain('Last seen working')
    expect(host.querySelector('[data-last-seen]')?.textContent).toContain('Last seen idle · 18 s ago')
    expect(roster().querySelector('[data-status="idle"][data-frozen="true"]')).not.toBeNull()
    expect(roster().querySelector('[data-observation-freshness="stale"]')).not.toBeNull()
    expect(roster().textContent).toContain(agent.name)
  })

  it('retains the last roster and its observation age when a refresh fails, then retries that feed', async () => {
    const retryAgents = vi.fn()
    source = { ...source, retryAgents }
    registry.set(agents, {
      _tag: 'Observed', value: [agent], freshness: 'stale',
      error: { reason: 'failed', detail: 'private refresh diagnostic' },
    })
    registry.set(agentsSync, staleSync<readonly Agent[]>([agent]))
    await mount()
    expect(roster().textContent).toContain(agent.name)
    expect(sidebar().querySelector('[data-roster-error]')?.textContent).toContain('Couldn’t refresh agents')
    expect(sidebar().textContent).toContain('Last observed 18 s ago')
    expect(sidebar().textContent).not.toContain('private refresh diagnostic')
    expect(roster().querySelector('[data-status="working"][data-frozen="true"]')).not.toBeNull()
    await act(async () => retry(sidebar())!.click())
    expect(retryAgents).toHaveBeenCalledOnce()
    await act(async () => {
      registry.set(agents, observed({ value: [agent] }))
      registry.set(agentsSync, liveSync<readonly Agent[]>([agent]))
    })
    expect(roster().textContent).toContain(agent.name)
    expect(sidebar().querySelector('[data-roster-error]')).toBeNull()
    expect(roster().querySelector('[data-frozen="true"]')).toBeNull()
    expect(retry(sidebar())).toBeUndefined()
  })
})
