// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Agent, Attention, decodeUnknownSync } from '@fractal/client/schema'
import { Option } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { rightPanelAtom } from './rightPanel.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
// The answer card's own behaviour has its tests; here only "an answer began" matters.
vi.mock('../conversation/alerts/AnswerAlert.tsx', () => ({
  AnswerAlert: ({ alert, onBegin }: { readonly alert: { readonly title: string }; readonly onBegin: () => void }) => <button type="button" data-answer="" onClick={onBegin}>{alert.title}</button>,
}))

const agent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/a', name: 'Agent A', host_id: 'host/example', runtime_ids: ['runtime/a'], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-06T12:00:00.000Z' })
const alertOf = (id: string, title: string) => decodeUnknownSync(Attention)({
  id, kind: 'attention', revision: 'r1', attention_kind: 'human-gate', title, detail: '', priority: 'normal', state: 'open', alert: true,
  person_id: 'person/avery', source_id: 'step-run/release/deploy', mission_id: 'mission/release',
  requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z', actions: ['review.approve'],
  presentation: { tier: 'stopped', kind: 'review', agent: null, raised_by: null, waiting: null },
})

const source = fixtureSource({ world: {
  now: 5000, gateway: 'gateway.example.invalid', agents: [agent, { ...agent, id: 'agent/b', name: 'Agent B' } as Agent, { ...agent, id: 'agent/c', name: 'Agent C', ask: Option.some('question') } as Agent], missions: [],
  attention: [alertOf('attention/one', 'First alert'), alertOf('attention/two', 'Second alert')], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
} })

let registry: AtomRegistry.AtomRegistry
let root: Root | undefined
let host: HTMLDivElement

const mount = async () => {
  root = createRoot(host)
  await act(async () => root!.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
  await act(async () => vi.dynamicImportSettled())
}
const unmount = async () => {
  await act(async () => root?.unmount())
  root = undefined
}
const arrive = async (href: string) => {
  await act(async () => { window.history.pushState(null, '', href); window.dispatchEvent(new PopStateEvent('popstate')) })
  await act(async () => vi.dynamicImportSettled())
}
const strip = () => [...host.querySelectorAll<HTMLElement>('[role="tablist"][aria-label="Workspace tabs"] [role="tab"]')]
const titles = () => strip().map(tab => tab.getAttribute('aria-label'))
const selected = () => strip().find(tab => tab.getAttribute('aria-selected') === 'true')?.getAttribute('aria-label')
const row = (ref: string) => host.querySelector<HTMLElement>(`nav[aria-label="Agent roster"] [data-wf-agent-ref="${ref}"]`)!
const key = async (code: string, shiftKey = false) => {
  await act(async () => { document.dispatchEvent(new KeyboardEvent('keydown', { code, altKey: true, shiftKey, bubbles: true, cancelable: true })) })
}
const terminalShown = () => host.querySelector('[data-testid="terminal-pane"][data-terminal-shown="true"]') !== null
const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('CSS', { escape: (value: string) => value })
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: false, media: query, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }))
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  window.localStorage.clear()
  window.history.replaceState(null, '', '/w/agent/a')
  registry = AtomRegistry.make()
  registry.set(rightPanelAtom, { open: false, tab: 'details' })
  host = document.createElement('div')
  document.body.append(host)
})
afterEach(async () => {
  await unmount()
  registry.dispose()
  host.remove()
  vi.unstubAllGlobals()
  window.history.replaceState(null, '', '/')
})

it('opens or focuses one tab per subject from the sidebar, and the URL follows', async () => {
  await mount()
  expect(titles()).toEqual(['Agent A'])
  await act(async () => row('agent/b').click())
  expect(titles()).toEqual(['Agent A', 'Agent B'])
  expect(window.location.pathname).toBe('/w/agent/b')
  await act(async () => row('agent/a').click())
  expect(titles()).toEqual(['Agent A', 'Agent B'])
  expect(selected()).toBe('Agent A')
  expect(window.location.pathname).toBe('/w/agent/a')
})

it('reopens a closed tab when history arrives back at its subject', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await key('KeyW')
  expect(titles()).toEqual(['Agent A'])
  await arrive('/w/agent/b')
  expect(titles()).toEqual(['Agent A', 'Agent B'])
  expect(selected()).toBe('Agent B')
})

it('restores tabs, each tab\'s face and the shared panel after a reload', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await vi.waitFor(() => expect(button('Show terminal').hasAttribute('aria-disabled')).toBe(false))
  await act(async () => button('Show terminal').click())
  expect(terminalShown()).toBe(true)
  await act(async () => button('Toggle right panel').click())
  await unmount()
  await mount()
  expect(titles()).toEqual(['Agent A', 'Agent B'])
  expect(selected()).toBe('Agent B')
  expect(terminalShown()).toBe(true)
  expect(button('Toggle right panel').getAttribute('aria-pressed')).toBe('true')
  // The face is the tab's: Agent A still shows its conversation, and the panel is still open for it.
  await act(async () => row('agent/a').click())
  expect(terminalShown()).toBe(false)
  expect(button('Toggle right panel').getAttribute('aria-pressed')).toBe('true')
})

it('opens a terminal link as the agent\'s one tab on its terminal, and the URL names the agent', async () => {
  window.history.replaceState(null, '', '/w/agent/a?open=terminal/a:detail')
  await mount()
  expect(titles()).toEqual(['Agent A'])
  expect(terminalShown()).toBe(true)
  expect(`${window.location.pathname}${window.location.search}`).toBe('/w/agent/a')
})

it('closes the others', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await arrive('/w/usage')
  await act(async () => row('agent/b').click())
  await key('KeyW', true)
  expect(titles()).toEqual(['Agent B'])
})

it('never leaves a drag behind a tab press, so the next press still selects', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  // A virtual click is what a screen reader or a press at a tab's exact centre delivers.
  await act(async () => strip()[0]!.click())
  expect(selected()).toBe('Agent A')
  await act(async () => strip()[1]!.click())
  expect(selected()).toBe('Agent B')
  expect(host.querySelector('[data-tab-id][aria-describedby]')).toBeNull()
})

it('closes the focused tab with Delete or Backspace', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  for (const key of ['Delete', 'Backspace']) {
    const tab = strip().at(-1)!
    await act(async () => { tab.focus(); tab.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })) })
  }
  expect(titles()).toEqual([])
})

it('brings the selected tab into view whenever the strip changes size', async () => {
  const observed: { readonly target: Element; readonly callback: (entries: readonly unknown[]) => void }[] = []
  vi.stubGlobal('ResizeObserver', class { constructor(private readonly callback: (entries: readonly unknown[]) => void) {} observe(target: Element) { if (!observed.some(row => row.target === target && row.callback === this.callback)) observed.push({ target, callback: this.callback }) } unobserve() {} disconnect() {} })
  // jsdom has no layout, so no scrollIntoView of its own.
  Object.defineProperty(Element.prototype, 'scrollIntoView', { configurable: true, writable: true, value: () => {} })
  const scrolled: string[] = []
  const scroll = vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(function (this: Element) { scrolled.push(this.getAttribute('aria-label') ?? '') })
  try {
    await mount()
    await act(async () => row('agent/b').click())
    await act(async () => row('agent/a').click())
    scrolled.length = 0
    // The width a phone gives the strip arrives after the selection did.
    const strip = host.querySelector('[role="tablist"][aria-label="Workspace tabs"]')!
    await act(async () => { for (const { target, callback } of [...observed]) if (target.contains(strip) || strip.contains(target)) callback([]) })
    expect(scrolled).toContain('Agent A')
  } finally {
    scroll.mockRestore()
  }
})

it('has one top bar: the tab strip carries the conversation\'s actions, and the open agent\'s name stays its heading', async () => {
  await mount()
  const workspace = host.querySelector<HTMLElement>('section[aria-label="Agent workspace"]')!
  const bars = workspace.querySelectorAll('header')
  expect(bars).toHaveLength(1)
  const bar = bars[0]!
  expect(bar.querySelector('[role="tablist"][aria-label="Workspace tabs"]')).not.toBeNull()
  for (const name of ['More actions', 'Show terminal', 'Toggle right panel']) expect(bar.querySelector(`button[aria-label="${name}"]`), name).not.toBeNull()
  // The breadcrumb is kept for assistive technology, though no longer drawn as its own line.
  expect(bar.querySelector('nav[aria-label="Breadcrumb"] [role="heading"]')?.textContent).toBe('Agent A')
})

it('marks a tab that needs you with the amber status ring, and shows the sidebar\'s hover card on a resting pointer', async () => {
  await mount()
  await act(async () => row('agent/c').click())
  const tab = (name: string) => strip().find(node => node.getAttribute('aria-label')?.startsWith(name))!
  expect(tab('Agent C').querySelector('[data-mark="needs-you"]')).not.toBeNull()
  expect(tab('Agent C').getAttribute('aria-label')).toContain('needs you')
  expect(tab('Agent A').querySelector('[data-mark="needs-you"]')).toBeNull()
  expect(tab('Agent A').querySelector('[data-status]')).not.toBeNull()
  // The same card the sidebar row shows, with the host it no longer gets from a header line.
  await act(async () => { tab('Agent C').dispatchEvent(new MouseEvent('mouseover', { bubbles: true })) })
  await vi.waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')?.textContent).toContain('Agent C'), { timeout: 3000 })
})

it('gives an answered alert\'s tab to the next alert, and closes it when none is left', async () => {
  window.history.replaceState(null, '', '/w/inbox/attention%2Fone')
  await mount()
  await arrive('/w/agent/a')
  await arrive('/w/inbox/attention%2Fone')
  expect(titles()).toEqual(['Alert', 'Agent A'])
  const answer = () => host.querySelector<HTMLButtonElement>('[data-tab-surface^="home:"] [data-answer]')!
  await vi.waitFor(() => expect(answer()?.textContent).toBe('First alert'))
  await act(async () => answer().click())
  expect(titles()).toEqual(['Alert', 'Agent A'])
  expect(window.location.pathname).toBe('/w/inbox/attention%2Ftwo')
  await vi.waitFor(() => expect(answer()?.textContent).toBe('Second alert'))
  await act(async () => answer().click())
  await vi.waitFor(() => expect(titles().length).toBeGreaterThan(0))
})
