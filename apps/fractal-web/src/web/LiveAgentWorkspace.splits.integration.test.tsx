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
import type { GlassGateway } from '../workspace/glassSync.ts'
import type { CollectionStream, Glass } from '@fractal/client'

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

const mount = async (glasses?: GlassGateway) => {
  root = createRoot(host)
  await act(async () => root!.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace {...(glasses === undefined ? {} : { glasses })} /></DataSourceProvider>))
  if (glasses !== undefined) await vi.waitFor(() => expect(strip().length).toBeGreaterThan(0))
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


// jsdom is not a Mac, so Mod is Ctrl.
const chord = async (init: KeyboardEventInit, target: EventTarget = document.body) => {
  await act(async () => { target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })) })
  await act(async () => { await new Promise(resolve => requestAnimationFrame(() => resolve(undefined))) })
}
const splitRight = () => chord({ key: 'd', code: 'KeyD', ctrlKey: true })
const strips = () => [...host.querySelectorAll<HTMLElement>('[role="tablist"][aria-label="Workspace tabs"]')]
const stripTitles = () => strips().map(list => [...list.querySelectorAll('[role="tab"]')].map(tab => tab.getAttribute('aria-label')))
const surface = (ref: string) => host.querySelector<HTMLElement>(`[data-tab-surface="${ref}"]`)
const panes = () => [...host.querySelectorAll<HTMLElement>('[data-pane]')].filter(pane => !pane.closest('[hidden]'))
const tabNamed = (title: string) => strip().find(tab => tab.getAttribute('aria-label') === title)!
const splitTab = async (title: string, item: 'Split right' | 'Split down') => {
  await act(async () => { tabNamed(title).dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true })) })
  const action = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(node => node.textContent === item)!
  await act(async () => action.click())
}

it('splits right with Mod+D into an empty pane, and Alt+W closes that pane', async () => {
  await mount()
  await splitRight()
  expect(stripTitles()).toEqual([['Agent A'], []])
  expect(host.querySelectorAll('[data-testid="workspace-empty"]')).toHaveLength(1)
  await key('KeyW')
  expect(stripTitles()).toEqual([['Agent A']])
})

it('moves a tab into a split from its menu, and splitting, zooming and closing the neighbour never remount a surface', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  const agentB = surface('agent:agent/b')!
  const agentA = surface('agent:agent/a')!
  expect(agentB).not.toBeNull()
  await splitTab('Agent B', 'Split right')
  expect(stripTitles()).toEqual([['Agent A'], ['Agent B']])
  expect(surface('agent:agent/b')).toBe(agentB)
  expect(surface('agent:agent/a')).toBe(agentA)
  // Each pane shows its own tab, and the surfaces sit over different panes.
  expect(agentA.getAttribute('aria-hidden')).toBe('false')
  expect(agentB.getAttribute('aria-hidden')).toBe('false')
  expect(agentA.getAttribute('data-surface-pane')).not.toBe(agentB.getAttribute('data-surface-pane'))
  await key('KeyO')
  expect(panes()).toHaveLength(1)
  expect(surface('agent:agent/b')).toBe(agentB)
  await key('KeyO')
  expect(panes()).toHaveLength(2)
  await act(async () => { tabNamed('Agent A').dispatchEvent(new MouseEvent('pointerdown', { bubbles: true })) })
  await key('KeyW')
  expect(stripTitles()).toEqual([['Agent B']])
  expect(surface('agent:agent/b')).toBe(agentB)
})

it('gives the focused split the accent and the URL; a press inside another split\'s surface focuses it', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await splitTab('Agent B', 'Split right')
  expect(window.location.pathname).toBe('/w/agent/b')
  const focusedTabs = () => strip().filter(tab => tab.closest('[data-pane][data-focused]') !== null).map(tab => tab.getAttribute('aria-label'))
  expect(focusedTabs()).toEqual(['Agent B'])
  await act(async () => { surface('agent:agent/a')!.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true })) })
  expect(focusedTabs()).toEqual(['Agent A'])
  expect(window.location.pathname).toBe('/w/agent/a')
})

it('moves between splits with Alt+arrows, past the left edge to the sidebar and back', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await splitTab('Agent B', 'Split right')
  const focusedPane = () => host.querySelector('[data-pane][data-focused]')
  const right = focusedPane()
  await chord({ key: 'ArrowLeft', code: 'ArrowLeft', altKey: true })
  expect(focusedPane()).not.toBe(right)
  expect(window.location.pathname).toBe('/w/agent/a')
  await chord({ key: 'ArrowLeft', code: 'ArrowLeft', altKey: true })
  expect(document.activeElement?.closest('aside[aria-label="Agents"]')).not.toBeNull()
  await chord({ key: 'ArrowRight', code: 'ArrowRight', altKey: true }, document.activeElement!)
  expect(document.activeElement?.closest('[data-pane], [data-tab-surface]')).not.toBeNull()
  expect(focusedPane()).not.toBe(right)
})

it('leaves Alt+Left and Alt+Right to a text field, and still moves with Alt+Up and Alt+Down', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await splitTab('Agent B', 'Split down')
  const field = document.createElement('textarea')
  surface('agent:agent/b')!.append(field)
  field.focus()
  await chord({ key: 'ArrowLeft', code: 'ArrowLeft', altKey: true }, field)
  expect(window.location.pathname).toBe('/w/agent/b')
  await chord({ key: 'ArrowUp', code: 'ArrowUp', altKey: true }, field)
  expect(window.location.pathname).toBe('/w/agent/a')
})

it('opens a sidebar row in a split to the right on Cmd or Ctrl-click', async () => {
  await mount()
  const target = row('agent/b')
  await act(async () => {
    target.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, ctrlKey: true }))
    target.click()
  })
  expect(stripTitles()).toEqual([['Agent A'], ['Agent B']])
  expect(window.location.pathname).toBe('/w/agent/b')
})

it('keeps one top bar unsplit, and once split gives every pane its own controls but only the first the sidebar\'s', async () => {
  await mount()
  expect(host.querySelectorAll('[data-testid="thread-header"]')).toHaveLength(1)
  await act(async () => row('agent/b').click())
  await splitTab('Agent B', 'Split right')
  const bars = [...host.querySelectorAll<HTMLElement>('[data-testid="thread-header"]')]
  expect(bars).toHaveLength(2)
  for (const bar of bars) expect(bar.querySelector('button[aria-label="Toggle right panel"]')).not.toBeNull()
  expect(host.querySelectorAll('[data-testid="global-bands"]')).toHaveLength(1)
  expect(bars[0]!.querySelector('[data-testid="global-bands"]') ?? bars[0]!.parentElement!.querySelector('[data-testid="global-bands"]')).not.toBeNull()
})

const pointer = async (target: EventTarget, type: string, init: { clientX?: number; clientY?: number } = {}) => {
  await act(async () => {
    const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init })
    Object.assign(event, { pointerId: 1, pointerType: 'mouse', width: 1, height: 1 })
    target.dispatchEvent(event)
  })
}
const box = (x: number, y: number, width: number, height: number): DOMRect =>
  ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON: () => ({}) }) as DOMRect

it('drags a tab onto a pane\'s edge with a live chip and a translucent overlay, splitting there; Escape cancels', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  const slot = host.querySelector<HTMLElement>('[data-drop-content]')!
  slot.getBoundingClientRect = () => box(0, 40, 800, 600)
  document.elementsFromPoint = () => [slot]
  const agentB = surface('agent:agent/b')
  await pointer(tabNamed('Agent B'), 'pointerdown', { clientX: 100, clientY: 20 })
  await pointer(window, 'pointermove', { clientX: 400, clientY: 300 })
  await pointer(window, 'pointermove', { clientX: 790, clientY: 300 })
  expect(document.querySelector('[data-testid="drag-chip"]')?.textContent).toBe('Agent B')
  expect(host.querySelector('[data-drop-overlay]')?.getAttribute('data-drop-overlay')).toBe('right')
  await act(async () => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })) })
  expect(document.querySelector('[data-testid="drag-chip"]')).toBeNull()
  await pointer(window, 'pointerup', { clientX: 790, clientY: 300 })
  expect(stripTitles()).toEqual([['Agent A', 'Agent B']])
  await pointer(tabNamed('Agent B'), 'pointerdown', { clientX: 100, clientY: 20 })
  await pointer(window, 'pointermove', { clientX: 400, clientY: 300 })
  await pointer(window, 'pointermove', { clientX: 790, clientY: 300 })
  await pointer(window, 'pointerup', { clientX: 790, clientY: 300 })
  expect(stripTitles()).toEqual([['Agent A'], ['Agent B']])
  expect(surface('agent:agent/b')).toBe(agentB)
})

it('drops a tab into another pane\'s strip at the slot under the pointer, and the emptied pane collapses', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await splitTab('Agent B', 'Split right')
  const target = host.querySelectorAll<HTMLElement>('[data-drop-strip]')[0]!
  tabNamed('Agent A').getBoundingClientRect = () => box(0, 0, 120, 40)
  document.elementsFromPoint = () => [target]
  await pointer(tabNamed('Agent B'), 'pointerdown', { clientX: 900, clientY: 20 })
  await pointer(window, 'pointermove', { clientX: 400, clientY: 20 })
  await pointer(window, 'pointermove', { clientX: 10, clientY: 20 })
  await pointer(window, 'pointerup', { clientX: 10, clientY: 20 })
  expect(stripTitles()).toEqual([['Agent B', 'Agent A']])
})

it('switches spaces from the sidebar\'s switcher, in place of the wordmark: new, rename, Alt+G, and close behind a confirm', async () => {
  await mount()
  const switcher = () => host.querySelector<HTMLButtonElement>('[data-testid="space-switcher"]')!
  expect(host.querySelector('[data-sidebar-header]')?.textContent).not.toContain('Fractal')
  expect(switcher().textContent).toBe('Main')
  const choose = async (label: string) => {
    await act(async () => switcher().click())
    const item = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(node => node.textContent === label)!
    await act(async () => item.click())
  }
  const name = async (value: string) => {
    const field = document.querySelector<HTMLInputElement>('input[placeholder="Space name"]')!
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(field, value)
      field.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => field.form!.requestSubmit())
  }
  await choose('New space…')
  await name('Review')
  expect(switcher().textContent).toBe('Review')
  expect(stripTitles()).toEqual([[]])
  await choose('Rename…')
  await name('Reviews')
  expect(switcher().textContent).toBe('Reviews')
  await key('KeyG')
  expect(switcher().textContent).toBe('Main')
  expect(stripTitles()).toEqual([['Agent A']])
  await key('KeyG')
  await choose('Close space…')
  expect(switcher().textContent).toBe('Reviews')
  const keep = [...document.querySelectorAll<HTMLButtonElement>('button')].find(node => node.textContent === 'Close space')!
  await act(async () => keep.click())
  expect(switcher().textContent).toBe('Main')
  await act(async () => switcher().click())
  expect([...document.querySelectorAll('[role="menuitem"]')].map(node => node.textContent)).toContain('Close space…')
})

it('marks a zoomed split, and its badge unzooms', async () => {
  await mount()
  await act(async () => row('agent/b').click())
  await splitTab('Agent B', 'Split right')
  await key('KeyO')
  expect(panes()).toHaveLength(1)
  const badge = host.querySelector<HTMLButtonElement>('[data-testid="zoom-badge"]')!
  expect(badge.getAttribute('aria-label')).toBe('Unzoom split')
  await act(async () => badge.click())
  expect(panes()).toHaveLength(2)
  expect(host.querySelector('[data-testid="zoom-badge"]')).toBeNull()
})

it('collapses to the focused pane on a phone, and the pane switcher reaches the others', async () => {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 390 })
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('max-width'), media: query, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }))
  await mount()
  expect(host.querySelector('[data-testid="pane-switcher"]'), 'one pane needs no switcher').toBeNull()
  await arrive('/w/agent/b')
  await splitRight()
  expect(panes()).toHaveLength(1)
  const switcher = () => host.querySelector<HTMLButtonElement>('[data-testid="pane-switcher"]')!
  expect(switcher().getAttribute('aria-label')).toBe('Pane 2 of 2')
  expect(host.querySelector('[data-pane][data-focused] [data-testid="workspace-empty"]')).not.toBeNull()
  await act(async () => switcher().click())
  const items = [...document.querySelectorAll<HTMLElement>('[role="menuitemradio"], [role="menuitem"]')]
  expect(items.map(item => item.textContent)).toEqual(['1Agent B +1', '2Empty'])
  await act(async () => items[0]!.click())
  expect(switcher().getAttribute('aria-label')).toBe('Pane 1 of 2')
  expect(panes()).toHaveLength(1)
  expect(window.location.pathname).toBe('/w/agent/b')
  expect(surface('agent:agent/b')!.getAttribute('aria-hidden')).toBe('false')
})
