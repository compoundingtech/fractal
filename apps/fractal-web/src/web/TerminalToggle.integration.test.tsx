// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Agent, decodeUnknownSync } from '@fractal/client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, unavailable, waiting, type Feed, type Grants } from '../data/source.ts'
import { terminalSubjectForAgent } from '../data/projections.ts'
import { makeScreen } from '../terminal/fixtures.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const agent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/example', name: 'Example Agent', host_id: 'host/example', runtime_ids: ['runtime/example'], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-04T12:00:00.000Z' })
const screen = { ...makeScreen({ scene: 'F3', columns: 40, rows: 4, frame: 0 }), terminal_id: 'terminal/example' as const }
let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  // jsdom has no font loading; the registered terminal faces resolve as loaded.
  const faces = ['Wf Terminal Mono', 'Wf Terminal Nerd Mono'].map((family) => ({ family, load: async () => [] }))
  Object.defineProperty(document, 'fonts', { configurable: true, value: Object.assign(new EventTarget(), { ready: Promise.resolve(), [Symbol.iterator]: () => faces[Symbol.iterator]() }) })
  window.history.replaceState(null, '', '/w/agent/example?open=thread')
  registry = AtomRegistry.make()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => { await act(async () => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals() })
const mount = async (agents: Feed<readonly Agent[]>, terminal: Feed<typeof screen> = observed({ value: screen }), retryTerminal?: (ref: string) => void, grants?: Grants) => {
  const source = fixtureSource({ world: { now: 0, agents: [agent], missions: [], attention: [], events: [], conversations: { [agent.id]: { items: [], hasOlder: false } }, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } }, overrides: { agents, terminal: { 'terminal/example': terminal }, ...(grants === undefined ? {} : { grants }) } })
  await act(async () => root.render(<DataSourceProvider source={{ ...source, ...(retryTerminal === undefined ? {} : { retryTerminal }) }} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
}
const toggle = () => host.querySelector<HTMLButtonElement>('[data-testid="thread-header"] button[aria-label="Show terminal"], [data-testid="thread-header"] button[aria-label="Show conversation"]')
const terminalPane = () => host.querySelector<HTMLElement>('[data-testid="terminal-pane"]')
const terminalShown = () => terminalPane()?.getAttribute('data-terminal-shown') === 'true'
const toggleDescription = () => (toggle()?.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(Boolean)
  .map(id => document.getElementById(id)?.textContent?.trim()).filter(Boolean).join(' | ')
// A control with a reason stays focusable: aria-disabled, never the native attribute, and the kit tooltip instead of a title.
const toggleDisabled = () => toggle()?.getAttribute('aria-disabled') === 'true' && toggle()?.disabled === false
const waitingReason = 'Waiting for the live agent roster to find this agent’s terminal.'
const noTerminal = { ...unavailable({ reason: 'failed', detail: 'This agent has no terminal.', code: 'no-terminal' }), retryable: false }
const grid = () => host.querySelector(`[data-testid="terminal-pane"] [aria-label="Terminal: ${screen.title}"]`)

describe('agent roster notice', () => {
  it.each([
    ['ungranted', 'No access to the agent rosterRead access has not been granted.'],
    ['unsupported', 'Agent roster not supportedThis connection does not offer it.'],
    ['failed', 'Couldn’t load agentsThe agent list did not load. Open conversations keep working.'],
  ] as const)('states %s with fixed copy and keeps the read detail out of the page', async (reason, copy) => {
    await mount(unavailable({ reason, detail: 'gateway said EACCES at /private/socket' }))
    const notice = host.querySelector(`[data-wf-roster-reason="${reason}"]`)
    expect(notice?.textContent).toBe(copy)
    expect(host.textContent).not.toContain('EACCES')
    expect(host.textContent).not.toContain('/private/socket')
  })
})

describe('terminal navigation toggle', () => {
  it.each([
    ['agent/example-seat', 'terminal/example-seat'],
    ['agent/team/seat', 'terminal/team/seat'],
  ])('does not offer an unverified terminal for %s before the first roster frame', async (id, terminal) => {
    window.history.replaceState(null, '', `/w/${id}?open=thread`)
    await mount(waiting)
    expect(host.querySelectorAll('button[aria-label="Show terminal"]')).toHaveLength(1)
    expect(terminalSubjectForAgent(id)).toBe(terminal)
    expect(toggleDisabled()).toBe(true)
    expect(toggleDescription()).toBe(waitingReason)
    expect(toggle()?.closest('[title]')).toBeNull()
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
    expect(host.querySelector('[data-testid="terminal-pane"]')).toBeNull()
  })
  it.each(['example-seat', 'team/seat'])('does not invent a terminal subject for invalid agent identity %s while waiting', async id => {
    window.history.replaceState(null, '', `/w/${id}?open=thread`)
    await mount(waiting)
    expect(toggle()).toBeNull()
  })
  it('enables the same named control only after the roster proves a runtime is present', async () => {
    await mount(waiting)
    expect(host.querySelector('[data-system-state="loading"]')?.textContent).toBe('Loading agents…')
    expect(toggleDisabled()).toBe(true)
    expect(toggleDescription()).toBe(waitingReason)
    await mount(observed({ value: [agent] }))
    expect(host.querySelectorAll('button[aria-label="Show terminal"]')).toHaveLength(1)
    expect(toggle()?.hasAttribute('aria-disabled')).toBe(false)
    expect(toggle()?.disabled).toBe(false)
    expect(toggleDescription()).toBe('')
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('terminal/example:detail')
    await vi.waitFor(() => expect(grid()).not.toBeNull(), { timeout: 10_000 })
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
  })
  it('opens the live terminal and returns to the thread on the second click', async () => {
    await mount(observed({ value: [agent] }))
    const historyLength = window.history.length
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('terminal/example:detail')
    expect(window.history.length).toBe(historyLength + 1)
    // The name says what a press does next, so it carries no toggle state that would read "Show conversation, pressed".
    expect(toggle()?.getAttribute('aria-label')).toBe('Show conversation')
    expect(toggle()?.hasAttribute('aria-pressed')).toBe(false)
    expect(terminalShown()).toBe(true)
    await vi.waitFor(() => expect(grid()).not.toBeNull(), { timeout: 10_000 })
    expect(host.textContent).not.toContain('Terminal unavailable')
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
    expect(window.history.length).toBe(historyLength + 2)
    expect(toggle()?.getAttribute('aria-label')).toBe('Show terminal')
    expect(terminalShown()).toBe(false)
  })
})

describe('terminal failure stays local to the terminal pane', () => {
  const thread = () => host.querySelector('[data-testid="transcript-scroll"]')
  const composer = () => host.querySelector<HTMLTextAreaElement>('textarea')
  const resources = () => host.querySelector('[data-testid="thread-header"] button[aria-label="More actions"]')
  const breadcrumb = () => host.querySelector('[aria-label="Breadcrumb"]')
  const waitForThread = async () => {
    await vi.waitFor(async () => {
      await act(async () => {})
      expect(thread()).not.toBeNull()
      expect(composer()).not.toBeNull()
    }, { timeout: 15_000 })
  }
  const retained = () => ({ thread: thread(), composer: composer(), resources: resources(), breadcrumb: breadcrumb() })
  interface RetainedThread {
    readonly thread: Element | null
    readonly composer: HTMLTextAreaElement | null
    readonly resources: Element | null
    readonly breadcrumb: Element | null
  }
  const expectThreadRetained = (before: RetainedThread, shown = true) => {
    expect(retained()).toEqual(before)
    expect(resources()).not.toBeNull()
    expect(breadcrumb()?.textContent).toBe('example/Example Agent')
    expect(composer()?.closest('form')?.inert === true).toBe(!shown)
    expect(thread()?.closest('[aria-hidden]')?.getAttribute('aria-hidden')).toBe(String(!shown))
    expect(host.textContent).not.toMatch(/\b(undefined|unknown)\b/i)
  }
  it('never enables the first visible control for an agent the roster does not list', async () => {
    window.history.replaceState(null, '', '/w/agent/example/no-terminal?open=thread')
    await mount(waiting)
    expect(host.querySelectorAll('button[aria-label="Show terminal"]')).toHaveLength(1)
    expect(toggleDisabled()).toBe(true)
    expect(toggleDescription()).toBe(waitingReason)
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
    await mount(observed({ value: [agent] }))
    const reason = 'This agent is not in the live agent roster, so it has no terminal.'
    expect(toggleDisabled()).toBe(true)
    expect(toggleDescription()).toBe(reason)
    expect(toggle()?.closest('[title]')).toBeNull()
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
    expect(host.querySelector('[data-testid="terminal-pane"]')).toBeNull()
    expect(host.textContent).not.toMatch(/\b(undefined|unknown)\b/i)
  })
  it('disables a known terminal-less agent with a reason tooltip and leaves its thread usable', async () => {
    await mount(observed({ value: [{ ...agent, runtime_ids: [] }] }))
    await waitForThread()
    const before = retained()
    expect(toggleDisabled()).toBe(true)
    expect(toggleDescription()).toBe('This agent has no terminal.')
    expect(toggle()?.closest('[title]')).toBeNull()
    await act(async () => toggle()!.click())
    expect(host.querySelector('[data-testid="terminal-pane"]')).toBeNull()
    expectThreadRetained(before)
  })
  it('replaces a terminal-less deep link so Back leaves instead of reopening the missing terminal', async () => {
    const previous = '/w/agent/previous?open=thread'
    window.history.replaceState(null, '', previous)
    window.history.pushState(null, '', '/w/agent/example?open=terminal/example:detail')
    const historyLength = window.history.length
    await mount(observed({ value: [{ ...agent, runtime_ids: [] }] }), noTerminal)
    await waitForThread()
    await vi.waitFor(() => expect(terminalShown()).toBe(false))
    expect(toggle()?.getAttribute('aria-label')).toBe('Show terminal')
    expect(toggleDisabled()).toBe(true)
    expect(toggleDescription()).toBe('This agent has no terminal.')
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
    expect(window.history.length).toBe(historyLength)
    await act(async () => {
      window.history.back()
      await vi.waitFor(() => expect(`${window.location.pathname}${window.location.search}`).toBe(previous))
    })
    expect(terminalShown()).toBe(false)
    await act(async () => {
      window.history.forward()
      await vi.waitFor(() => expect(`${window.location.pathname}${window.location.search}`).toBe('/w/agent/example?open=thread'))
    })
    expect(terminalShown()).toBe(false)
  })
  it('disables the toggle after an attach finds no terminal, returns to the conversation, and re-enables when the runtimes change', async () => {
    await mount(observed({ value: [agent] }), noTerminal)
    await waitForThread()
    expect(toggle()?.hasAttribute('aria-disabled')).toBe(false)
    const historyLength = window.history.length
    await act(async () => toggle()!.click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
    await waitForThread()
    expect(terminalShown()).toBe(false)
    expect(toggle()?.getAttribute('aria-label')).toBe('Show terminal')
    expect(toggleDisabled()).toBe(true)
    expect(toggleDescription()).toBe('This agent has no terminal.')
    // Opening is an ordinary push; the automatic return replaces that entry.
    expect(window.history.length).toBe(historyLength + 1)
    // The same roster frame again keeps the remembered failure.
    await mount(observed({ value: [{ ...agent }] }), noTerminal)
    expect(toggleDisabled()).toBe(true)
    // A changed runtime list is new evidence: the toggle is offered again.
    await mount(observed({ value: [{ ...agent, runtime_ids: ['runtime/example-next'] } as unknown as Agent] }), observed({ value: screen }))
    expect(toggle()?.hasAttribute('aria-disabled')).toBe(false)
    expect(toggleDescription()).toBe('')
  })
  it.each([
    ['ungranted', 'Terminal viewing is refused by the gateway.', false],
    ['failed', 'The terminal service is temporarily unavailable.', true],
  ] as const)('shows %s reason and only offers a useful Retry without replacing the thread', async (reason, detail, retryable) => {
    const retry = vi.fn()
    const feed = { ...unavailable({ reason, detail }), retryable }
    await mount(observed({ value: [agent] }), feed, retry)
    await waitForThread()
    const before = retained()
    await act(async () => toggle()!.click())
    await vi.waitFor(() => expect(terminalPane()?.textContent).toContain(detail))
    expectThreadRetained(before, false)
    const retryButton = [...host.querySelectorAll<HTMLButtonElement>('[data-testid="terminal-pane"] button')].find(button => button.textContent === 'Retry')
    expect(retryButton !== undefined).toBe(retryable)
    if (retryable) {
      await act(async () => retryButton!.click())
      expect(retry).toHaveBeenCalledWith('terminal/example')
      expectThreadRetained(before, false)
    }
    await act(async () => toggle()!.click())
    expect(terminalShown()).toBe(false)
    expectThreadRetained(before)
  })
  it('swaps the panes in both directions, retaining each one, its draft and its node, and moving focus to what is shown', async () => {
    await mount(observed({ value: [agent] }))
    await waitForThread()
    // This fixture never reaches a sendable composer; only the focus target's enabled state matters here.
    composer()!.disabled = false
    const before = retained()
    const setDraft = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!
    await act(async () => { setDraft.call(composer()!, 'keep this draft'); composer()!.dispatchEvent(new Event('input', { bubbles: true })) })
    await act(async () => toggle()!.click())
    await vi.waitFor(() => expect(grid()).not.toBeNull(), { timeout: 10_000 })
    const shownGrid = grid()
    expect(terminalShown()).toBe(true)
    expect(terminalPane()?.hasAttribute('inert')).toBe(false)
    // The pane takes focus, not the grid: focusing a live grid would arm typing without a gesture.
    expect(document.activeElement).toBe(terminalPane())
    expectThreadRetained(before, false)
    await act(async () => toggle()!.click())
    expect(terminalShown()).toBe(false)
    expect(terminalPane()?.hasAttribute('inert')).toBe(true)
    expect(terminalPane()?.getAttribute('aria-hidden')).toBe('true')
    expect(grid()).toBe(shownGrid)
    expectThreadRetained(before)
    expect(composer()?.value).toBe('keep this draft')
    expect(document.activeElement).toBe(composer())
  })
  it('keeps a read-only device read-only across a hide and reveal', async () => {
    await mount(observed({ value: [agent] }), undefined, undefined, { actions: 'ungranted', messageSend: 'ungranted', attachmentUpload: 'ungranted', terminalInput: 'ungranted', terminalControl: 'ungranted' })
    const inputPath = () => terminalPane()?.querySelector('[data-terminal-input-path]')?.getAttribute('data-terminal-input-path')
    await act(async () => toggle()!.click())
    await vi.waitFor(() => expect(inputPath()).toBe('ReadOnly'), { timeout: 10_000 })
    await act(async () => toggle()!.click())
    expect(inputPath()).toBe('ReadOnly')
    await act(async () => toggle()!.click())
    expect(inputPath()).toBe('ReadOnly')
    expect(terminalPane()?.querySelector('[data-terminal-input="live"]')).toBeNull()
  })
  it('does not take focus or switch the view for a plain URL change', async () => {
    await mount(observed({ value: [agent] }))
    await waitForThread()
    const outside = document.body.appendChild(document.createElement('button'))
    outside.focus()
    await act(async () => { window.history.pushState(null, '', '/w/agent/example?open=terminal/example:detail'); window.dispatchEvent(new PopStateEvent('popstate')) })
    expect(terminalShown()).toBe(true)
    expect(document.activeElement).toBe(outside)
    outside.remove()
  })
  it('hides workspace tabs while the terminal is shown and never adds it as a tab', async () => {
    window.history.replaceState(null, '', '/w/agent/example?open=thread')
    await mount(observed({ value: [agent] }))
    await act(async () => toggle()!.click())
    expect(host.querySelector('[role="toolbar"][aria-label="Workspace views"]')).toBeNull()
    await act(async () => toggle()!.click())
    expect(host.querySelector('[role="toolbar"][aria-label="Workspace views"]')).toBeNull()
  })
  it('remembers the view per agent, and releases a terminal when its agent is left', async () => {
    const other = { ...agent, id: 'agent/other', name: 'Other Agent' } as Agent
    await mount(observed({ value: [agent, other] }))
    await act(async () => toggle()!.click())
    expect(terminalShown()).toBe(true)
    const row = (ref: string) => host.querySelector<HTMLElement>(`[data-wf-agent-ref="${ref}"]`)!
    await vi.waitFor(() => expect(row('agent/other')).not.toBeNull())
    await act(async () => row('agent/other').click())
    expect(window.location.pathname).toBe('/w/agent/other')
    expect(new URLSearchParams(window.location.search).get('open')).toBe('thread')
    expect(terminalPane()).toBeNull()
    await act(async () => row('agent/example').click())
    expect(new URLSearchParams(window.location.search).get('open')).toBe('terminal/example:detail')
    expect(terminalShown()).toBe(true)
  })
})
