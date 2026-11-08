import { Agent, decodeUnknownSync, type AgentEncoded } from '@fractal/client/schema'
import { openAgent, type DecodedAgent } from '@fractal/sdk/read'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { createRoot } from 'react-dom/client'

import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { type Feed, observed } from '../data/source.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

// Executable browser fixture for scripts/pane-aside-proof.mjs: each pane owns its side panel. It opens by default
// when the pane is wide enough, an explicit toggle holds only within the pane's width band, splitting gives each pane
// its own, the panel's tab is remembered per kind, and a phone opens it as a bottom sheet. Nothing global is docked.
const scheme = new URLSearchParams(location.search).get('scheme') === 'light' ? 'light' : 'dark'
const now = Date.parse('2026-10-08T12:00:00.000Z')
const dayAgo = new Date(now - 86_400_000).toISOString()
const agents = Array.from({ length: 2 }, (_, index) => decodeUnknownSync(Agent, 'strict')({
  kind: 'agent', id: `agent/example/seat-${index}`, name: `Synthetic seat ${index}`, host_id: 'host/example', runtime_ids: [],
  reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: dayAgo,
} satisfies AgentEncoded))
const registry = AtomRegistry.make()
const source = {
  ...fixtureSource({ world: { now, events: [], agents, missions: [], attention: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
  agents: Atom.make<Feed<readonly DecodedAgent[]>>(observed({ value: agents.map(openAgent) })).pipe(Atom.keepAlive),
}

const afterPaint = () => {
  const { promise, resolve } = Promise.withResolvers<void>()
  requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
  return promise
}
const until = async (ready: () => boolean, what: string) => {
  for (let frame = 0; frame < 300 && !ready(); frame++) await afterPaint()
  if (!ready()) throw new Error(`timed out waiting for ${what}`)
}
const failures: string[] = []
const expect = (condition: boolean, message: string) => { if (!condition) failures.push(message) }
const panes = () => [...document.querySelectorAll<HTMLElement>('[data-pane]')]
const asideOf = (pane: HTMLElement) => pane.querySelector<HTMLElement>('[data-pane-aside]')
const toggleOf = (pane: HTMLElement) => pane.querySelector<HTMLButtonElement>('button[aria-label="Toggle right panel"]')
const press = (button: HTMLElement | null | undefined, what: string) => { if (button == null) throw new Error(`no ${what}`); button.click() }
const tab = (scope: ParentNode, name: string) => [...scope.querySelectorAll<HTMLElement>('[role="tab"]')].find(node => node.textContent?.trim() === name)

const root = createRoot(document.getElementById('root')!)
const facts: Record<string, unknown> = { scheme }
try {
  localStorage.clear()
  const entry = (value: unknown) => ({ version: 1, value, clock: 1, writer: 'fixture' })
  localStorage.setItem('wf.ui@1', JSON.stringify({ version: 1, entries: { 'round2.scheme': entry(scheme) } }))
  root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>)
  window.history.pushState(null, '', `/w/${agents[0]!.id}`)
  window.dispatchEvent(new PopStateEvent('popstate'))
  await until(() => document.title.startsWith(agents[0]!.name), 'the agent')
  expect(document.querySelector('[data-dock]') === null, 'a global dock is still mounted')
  expect([...document.querySelectorAll('[role="separator"]')].every(node => !/^Resize (Details|Changes|resources)/.test(node.getAttribute('aria-label') ?? '')), 'a global dock separator is still mounted')
  if (innerWidth >= 900) {
    await until(() => panes().length === 1 && toggleOf(panes()[0]!) !== null, 'the pane and its panel toggle')
    const [first] = panes()
    // Wide enough for a conversation and its panel: open by default, inside the pane.
    await until(() => asideOf(first!) !== null, 'the default-open pane aside')
    const pane = first!.getBoundingClientRect(), aside = asideOf(first!)!.getBoundingClientRect()
    facts.wide = { pane: pane.width, aside: aside.width }
    expect(aside.right <= pane.right + 1 && aside.left >= pane.left - 1, 'the aside is not inside its pane')
    expect(toggleOf(first!)!.getAttribute('aria-pressed') === 'true', 'the toggle does not say the panel is open')
    // The panel's tab is remembered per kind: choose Changes here.
    await until(() => tab(asideOf(first!)!, 'Changes') !== undefined, 'the Changes tab')
    press(tab(asideOf(first!)!, 'Changes'), 'Changes tab')
    await afterPaint()
    // An explicit close holds in this band.
    press(toggleOf(first!), 'panel toggle')
    await afterPaint(); await afterPaint()
    expect(asideOf(first!) === null, 'an explicit close did not hold')
    press(toggleOf(first!), 'panel toggle')
    await until(() => asideOf(first!) !== null, 'the reopened aside')
    // Split: each half is too narrow for the default, so neither shows, and the earlier explicit open no longer holds.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'd', code: 'KeyD', metaKey: navigator.platform.includes('Mac'), ctrlKey: !navigator.platform.includes('Mac'), bubbles: true }))
    await until(() => panes().length === 2, 'the split')
    await afterPaint(); await afterPaint()
    facts.split = panes().map(node => node.getBoundingClientRect().width)
    expect(panes().every(node => asideOf(node) === null), 'a half-width pane shows its aside by default')
    // The new pane is focused and empty: open the other agent there. An empty pane has no panel and no toggle.
    const second = panes()[1]!
    expect(toggleOf(second) === null || second.querySelector('[data-pane-slot-content] [data-workspace-empty]') === null, 'an empty pane offers a panel toggle')
    window.history.pushState(null, '', `/w/${agents[1]!.id}`)
    window.dispatchEvent(new PopStateEvent('popstate'))
    await until(() => toggleOf(panes()[1]!) !== null, 'the second pane panel toggle')
    expect(asideOf(panes()[1]!) === null, 'a half-width pane shows its aside by default')
    // Opening one pane's panel opens only that one, on the kind's remembered tab.
    press(toggleOf(second), 'second pane toggle')
    await until(() => asideOf(second) !== null, 'the second pane aside')
    expect(asideOf(panes()[0]!) === null, 'opening one pane opened another')
    await until(() => tab(asideOf(second)!, 'Changes') !== undefined, 'the second pane Changes tab')
    expect(tab(asideOf(second)!, 'Changes')!.getAttribute('aria-selected') === 'true', 'the panel tab is not remembered per kind')
  } else {
    // A phone has no inline aside: the bar's panel button opens the focused pane's panel as a bottom sheet.
    expect(document.querySelector('[data-pane-aside]') === null, 'a phone shows an inline aside')
    const opener = () => [...document.querySelectorAll<HTMLButtonElement>('button')].find(node => node.getAttribute('aria-label') === 'Agent details')
    await until(() => opener() !== undefined, 'the phone panel button')
    press(opener(), 'phone panel button')
    const sheet = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="Agent details"]')
    await until(() => sheet() !== null, 'the sheet')
    await afterPaint()
    const box = sheet()!.getBoundingClientRect()
    facts.sheet = { top: box.top, bottom: box.bottom, width: box.width, side: sheet()!.dataset.narrowSheet }
    expect(sheet()!.dataset.narrowSheet === 'bottom', 'the phone panel is not a bottom sheet')
    expect(Math.abs(box.width - innerWidth) <= 1, `the bottom sheet is ${box.width}px wide, not the viewport`)
    expect(box.top > 40, 'the bottom sheet covers the whole screen')
  }
  const canvas = getComputedStyle(document.querySelector<HTMLElement>('[data-testid="live-agent-workspace"]')!).colorScheme
  facts.colorScheme = canvas
  expect(document.documentElement.dataset.scheme === scheme, `the document scheme is ${document.documentElement.dataset.scheme}, not ${scheme}`)
  document.body.dataset.proof = JSON.stringify({ ...facts, failures })
  document.body.dataset.testResult = failures.length === 0 ? 'pass' : 'fail'
} catch (error) {
  document.body.dataset.proof = JSON.stringify({ ...facts, failures, error: String(error instanceof Error ? error.stack : error) })
  document.body.dataset.testResult = 'fail'
}
