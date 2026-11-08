// @vitest-environment jsdom
// The left sidebar is the app's one global area: its header, a footer for the connection and Usage, a 48px rail when
// collapsed, and a toast region beside it. Panes keep only what is theirs, plus a band in the focused pane once the
// connection is gone.
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Agent, Attention, decodeUnknownSync, type Mission, type TerminalScreen } from '@fractal/client/schema'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { openAgent, type DecodedAgent } from '@fractal/sdk/read'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import { DataSourceProvider } from '../../data/react.tsx'
import { fixtureSource } from '../../data/fixtureSource.ts'
import { initialFeedSync, type FeedSyncObservation } from '../../data/feedSync.ts'
import { observed, type ConversationPage, type DataSource, type Feed } from '../../data/source.ts'
import { AlertAnswersContext, createAlertAnswers, type AlertAnswers } from '../../conversation/alerts/alertAnswers.ts'
import { LiveAgentWorkspace } from '../LiveAgentWorkspace.tsx'
import { writeCellsLikeAnotherWindow } from '../../state/cell/testing.ts'

vi.mock('@stylexjs/stylex', () => import('../../testStylex.ts'))

const agent = openAgent(decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/a', name: 'Agent A', host_id: 'host/example', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-06T12:00:00.000Z' }))
const other: DecodedAgent = { ...agent, id: 'agent/b', name: 'Agent B' }
const alert = decodeUnknownSync(Attention)({
  id: 'attention/one', kind: 'attention', revision: 'r1', attention_kind: 'human-gate', title: 'Ship the release', detail: '', priority: 'normal', state: 'open', alert: true,
  person_id: 'person/avery', source_id: 'step-run/release/deploy', mission_id: 'mission/release',
  requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z', actions: ['review.approve'],
  presentation: { tier: 'stopped', kind: 'review', agent: null, raised_by: null, waiting: null },
})

let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
let gatewaySync: Atom.Writable<FeedSyncObservation>
let source: DataSource
let answers: AlertAnswers
const reconnect = vi.fn()

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('CSS', { escape: (value: string) => value })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  window.localStorage.clear()
  window.history.replaceState(null, '', '/w/agent/a')
  // The sidebar's open state is a cell shared across mounts; every test starts with the dock open.
  writeCellsLikeAnotherWindow({ 'panels.leftSidebar': { value: { open: true, ratio: 256 / 1440 } } })
  registry = AtomRegistry.make()
  gatewaySync = Atom.make<FeedSyncObservation>({ status: { _tag: 'Live', since: 0 }, observedAt: 0 })
  const agents = Atom.make<Feed<readonly DecodedAgent[]>>(observed({ value: [agent, other] })).pipe(Atom.keepAlive)
  source = {
    ...fixtureSource({ world: { now: 5000, gateway: 'gateway.example.invalid', agents: [], missions: [], attention: [alert], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
    agents,
    reconnect,
    sync: {
      gateway: gatewaySync,
      agents: Atom.make(initialFeedSync<readonly DecodedAgent[]>(0)),
      missions: Atom.make(initialFeedSync<readonly Mission[]>(0)),
      attention: Atom.make(initialFeedSync<readonly Attention[]>(0)),
      conversation: () => Atom.make(initialFeedSync<ConversationPage>(0)),
      terminal: () => Atom.make(initialFeedSync<TerminalScreen>(0)),
    },
  }
  answers = createAlertAnswers()
  reconnect.mockReset()
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

const mount = async (width: number) => {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width })
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><AlertAnswersContext.Provider value={answers}><LiveAgentWorkspace /></AlertAnswersContext.Provider></DataSourceProvider>))
  await act(async () => vi.dynamicImportSettled())
}
const sidebar = () => host.querySelector<HTMLElement>('aside[aria-label="Agents"]')!
const footer = () => sidebar().querySelector<HTMLElement>(':scope > footer')
const button = (scope: HTMLElement, label: string) => within(scope).getByRole('button', { name: label })
const press = async (target: HTMLElement) => { await act(async () => target.click()) }
const reconnecting: FeedSyncObservation = { status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: 9000, issue: 'socket closed' } }, observedAt: 0 }
const failed: FeedSyncObservation = { status: { _tag: 'Failed', cause: { _tag: 'Server', code: 'forbidden', message: 'denied' } }, observedAt: 0 }
const panes = () => [...host.querySelectorAll<HTMLElement>('[data-pane]')]
const splitRight = async () => {
  await act(async () => { document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'd', code: 'KeyD', ctrlKey: true, bubbles: true, cancelable: true })) })
  await act(async () => {
    const { promise, resolve } = Promise.withResolvers<void>()
    requestAnimationFrame(() => resolve())
    await promise
  })
}

describe('the sidebar footer', () => {
  it('holds Usage below the tabs, outside every tab panel', async () => {
    await mount(1280)
    const usage = footer()?.querySelector('[data-usage-section]')
    expect(usage).not.toBeNull()
    expect(usage!.closest('[role="tabpanel"]')).toBeNull()
    expect(sidebar().querySelectorAll('[data-usage-section]')).toHaveLength(1)
  })

  it('stays quiet while live, then says Reconnecting with the next try and a Reconnect that works', async () => {
    await mount(1280)
    expect(footer()!.querySelector('[data-connection-line]')).toBeNull()
    await act(async () => registry.set(gatewaySync, reconnecting))
    const line = footer()!.querySelector<HTMLElement>('[data-connection-line]')!
    expect(line.getAttribute('data-connection-line')).toBe('reconnecting')
    expect(within(line).getByRole('status').textContent).toBe('Reconnecting')
    expect(line.textContent).toContain('Next try in')
    await press(button(line, 'Reconnect'))
    expect(reconnect).toHaveBeenCalledOnce()
    // Recovering is the footer's to say: no pane carries a band.
    expect(host.querySelector('[data-connection-state]')).toBeNull()
  })
})

describe('the focused pane band', () => {
  it('appears in the focused pane alone once the connection fails, beside the footer line', async () => {
    await mount(1440)
    await splitRight()
    expect(panes()).toHaveLength(2)
    await act(async () => registry.set(gatewaySync, failed))
    const bands = [...host.querySelectorAll<HTMLElement>('[data-connection-state]')]
    expect(bands).toHaveLength(1)
    expect(bands[0]!.getAttribute('data-connection-state')).toBe('failed')
    expect(bands[0]!.closest('[data-pane]')!.hasAttribute('data-focused')).toBe(true)
    expect(footer()!.querySelector('[data-connection-line]')?.getAttribute('data-connection-line')).toBe('failed')
  })

  it('appears for an offline network, on a phone too', async () => {
    source = { ...source, network: Atom.make({ _tag: 'Offline' } as const) }
    await mount(440)
    const band = host.querySelector<HTMLElement>('[data-connection-state]')!
    expect(band.getAttribute('data-connection-state')).toBe('offline')
    expect(band.closest('[data-pane]')).not.toBeNull()
  })
})

describe('the collapsed rail', () => {
  it('replaces the dock with a 48px rail of the global controls, and no pane offers to expand', async () => {
    await mount(1280)
    await press(button(sidebar(), 'Collapse agents'))
    const rail = sidebar()
    expect(rail.getAttribute('data-sidebar-presentation')).toBe('rail')
    expect([...rail.querySelectorAll('button')].map(node => node.getAttribute('aria-label'))).toEqual(['Expand sidebar', 'Space: Main', 'Command palette', 'Agents, 2', 'Alerts, 1 needs you', 'Missions', 'Usage', 'Settings'])
    expect(host.querySelector('[data-resizer="live-agent-sidebar"]')).toBeNull()
    for (const header of host.querySelectorAll<HTMLElement>('[data-testid="thread-header"]')) expect(within(header).queryByRole('button', { name: 'Expand sidebar' })).toBeNull()
  })

  it('keeps a lost connection visible, and its control expands to the footer', async () => {
    await mount(1280)
    await press(button(sidebar(), 'Collapse agents'))
    await act(async () => registry.set(gatewaySync, reconnecting))
    const control = within(sidebar()).getByRole('button', { name: /^Connection: Reconnecting/ })
    await press(control)
    expect(sidebar().getAttribute('data-sidebar-presentation')).toBe('dock')
    expect(footer()!.querySelector('[data-connection-line="reconnecting"]')).not.toBeNull()
  })

  it('opens the sidebar on the tab a rail icon names', async () => {
    await mount(1280)
    await press(button(sidebar(), 'Collapse agents'))
    await press(button(sidebar(), 'Alerts, 1 needs you'))
    expect(sidebar().getAttribute('data-sidebar-presentation')).toBe('dock')
    expect(within(sidebar()).getByRole('tab', { selected: true }).getAttribute('data-sidebar-tab')).toBe('alerts')
  })

  it('hands focus to its counterpart when the control under it goes', async () => {
    await mount(1280)
    const collapse = button(sidebar(), 'Collapse agents')
    collapse.focus()
    await press(collapse)
    expect(document.activeElement).toBe(button(sidebar(), 'Expand sidebar'))
    await press(button(sidebar(), 'Expand sidebar'))
    expect(document.activeElement).toBe(button(sidebar(), 'Collapse agents'))
  })
})

describe('the toast region', () => {
  it('says a refused answer once, beside the sidebar and outside every pane, without taking focus', async () => {
    await mount(1280)
    const before = document.activeElement
    await act(async () => {
      answers.send(alert, 'Approve', async () => ({ _tag: 'Refused', detail: 'Fractal no longer offers this answer here.' }))
      await Promise.resolve()
    })
    const toast = await vi.waitFor(() => host.querySelector<HTMLElement>('[data-testid="alert-flash"]')!)
    expect(toast.getAttribute('role')).toBe('alert')
    expect(toast.closest('[role="region"][aria-label="Notifications"]')).not.toBeNull()
    expect(toast.closest('[data-pane]')).toBeNull()
    expect(toast.closest('aside')).toBeNull()
    expect(document.activeElement).toBe(before)
    await press(button(toast, 'Dismiss'))
    expect(host.querySelector('[data-testid="alert-flash"]')).toBeNull()
  })
})

describe('the phone drawer', () => {
  it('marks its opener while the connection is lost or alerts need the person, and carries the same footer', async () => {
    await mount(440)
    const opener = () => within(host).getByRole('button', { name: 'Expand sidebar' })
    const description = () => document.getElementById(opener().getAttribute('aria-describedby')!)?.textContent
    expect(description()).toBe('Alerts, 1 needs you')
    await act(async () => registry.set(gatewaySync, reconnecting))
    expect(description()).toBe('Reconnecting · Alerts, 1 needs you')
    expect(opener().querySelector('[data-chrome-badge] [data-status-dot="attention"]')).not.toBeNull()
    await press(opener())
    const drawer = within(host).getByRole('dialog', { name: 'Agents' })
    const drawerFooter = drawer.querySelector<HTMLElement>('aside > footer')!
    expect(drawerFooter.querySelector('[data-connection-line="reconnecting"]')).not.toBeNull()
    expect(drawerFooter.querySelector('[data-usage-section]')).not.toBeNull()
  })
})
