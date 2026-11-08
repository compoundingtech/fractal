import { Agent, ResourceObservation, decodeUnknownSync, type AgentEncoded } from '@fractal/client/schema'
import { openAgent, type DecodedAgent } from '@fractal/sdk/read'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { createRoot } from 'react-dom/client'

import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { type Feed, observed } from '../data/source.ts'
import type { ResourcePage } from '../resources/agent/model.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

// Executable browser fixture: scripts/scroll-containment-proof.mjs asserts body[data-test-result=pass],
// then wheels over the panel itself and re-measures. Regression for the shell scrolling up off the
// viewport: the right panel's Resources tab renders React Aria's absolutely positioned VisuallyHidden
// text inside a static scroller, so without a containing block in the shell that text resolved against
// the initial containing block, grew the document past the viewport, and wheel chaining at the panel's
// end scrolled the whole shell up (sidebar and transcript cut off, empty canvas below).
const now = Date.parse('2026-10-08T12:00:00.000Z')
const dayAgo = new Date(now - 86_400_000).toISOString()
const agents = Array.from({ length: 3 }, (_, index) => decodeUnknownSync(Agent, 'strict')({
  kind: 'agent', id: `agent/example/seat-${index}`, name: `Synthetic seat ${index}`, host_id: 'host/example', runtime_ids: [],
  reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: dayAgo,
} satisfies AgentEncoded))
// A long first page with more to load: the panel overflows its own scroller.
const items = Array.from({ length: 120 }, (_, index) => decodeUnknownSync(ResourceObservation)({
  id: `resource/${index}`, kind: 'filesystem.file', facts: { title: `Synthetic resource ${index}`, state: 'open' },
  observed_at: dayAgo, opened_by: agents[0]!.id, opened_by_run: null,
}))
const resources = Atom.make<Feed<ResourcePage>>(observed({ value: { items, nextCursor: 'cursor/next' } })).pipe(Atom.keepAlive)
const registry = AtomRegistry.make()
const source = {
  ...fixtureSource({ world: { now, events: [], agents, missions: [], attention: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
  agents: Atom.make<Feed<readonly DecodedAgent[]>>(observed({ value: agents.map(openAgent) })).pipe(Atom.keepAlive),
  resources: { byAgent: () => resources, byId: () => Atom.make(observed({ value: items[0]! })), subjects: Atom.make([]), loadMore: () => undefined, refresh: () => undefined, resolveFile: () => undefined },
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
const control = (name: string) => [...document.querySelectorAll<HTMLElement>('button, [role="tab"]')].find(node => (node.getAttribute('aria-label') ?? node.textContent?.trim())?.startsWith(name))
const failures: string[] = []
const expect = (condition: boolean, message: string) => { if (!condition) failures.push(message) }
const shell = () => document.querySelector<HTMLElement>('[data-testid="live-agent-workspace"]')!
const panel = () => document.querySelector<HTMLElement>('[data-agent-resources]')

/** Shell geometry and every absolutely positioned shell descendant that escapes to the initial containing block. */
const measure = () => {
  const box = shell().getBoundingClientRect()
  const escaped = [...shell().querySelectorAll('*')].filter(node => getComputedStyle(node).position === 'absolute' && (node as HTMLElement).offsetParent === document.body).length
  return { innerHeight, scrollHeight: document.documentElement.scrollHeight, scrollY, shellTop: box.top, shellBottom: box.bottom, escaped, panelClientHeight: panel()?.clientHeight, panelScrollHeight: panel()?.scrollHeight }
}
Object.assign(window, { scrollContainmentMeasure: measure })

const root = createRoot(document.getElementById('root')!)
try {
  localStorage.clear()
  root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>)
  // Open the agent the way navigation does, then wait for its header (desktop tab or phone screen).
  window.history.pushState(null, '', `/w/${agents[0]!.id}`)
  window.dispatchEvent(new PopStateEvent('popstate'))
  // Wide headers toggle the docked panel; a phone header opens the same panel as a sheet.
  const opener = () => control('Toggle right panel') ?? control('Agent details')
  await until(() => document.title.startsWith(agents[0]!.name) && opener() !== undefined, 'the agent header')
  // Panel state persists across loads (state cells): open it only if it is closed.
  if (opener()!.getAttribute('aria-pressed') !== 'true') opener()!.click()
  await until(() => control('Resources') !== undefined, 'the Resources tab')
  control('Resources')!.click()
  await until(() => document.querySelector('[data-agent-resources] [data-details-why]') !== null, 'the Resources panel')
  await afterPaint()
  const opened = measure()
  expect(opened.escaped === 0, `${opened.escaped} absolutely positioned shell descendants resolve against the initial containing block`)
  expect(opened.scrollHeight === innerHeight, `document scrollHeight ${opened.scrollHeight} != viewport ${innerHeight}`)
  expect((opened.panelScrollHeight ?? 0) > (opened.panelClientHeight ?? 0), 'the Resources panel does not own its overflow')
  // The panel's own end, then a document scroll request: neither may move the shell.
  panel()!.scrollTop = panel()!.scrollHeight
  window.scrollTo(0, 100_000)
  await afterPaint()
  const requested = measure()
  expect(requested.scrollY === 0, `the document scrolled to ${requested.scrollY}`)
  expect(requested.shellTop === 0, `the shell moved to top ${requested.shellTop}`)
  window.scrollTo(0, 0)
  document.body.dataset.proof = JSON.stringify({ opened, requested, failures })
  document.body.dataset.testResult = failures.length === 0 ? 'pass' : 'fail'
} catch (error) {
  document.body.dataset.proof = String(error instanceof Error ? error.stack : error)
  document.body.dataset.testResult = 'fail'
}
// Stays mounted: the runner wheels over the panel next, as the user did.
