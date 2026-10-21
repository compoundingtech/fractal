// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { FractalClient } from '@fractal/client'
import { Agent, decodeUnknownSync } from '@fractal/client/schema'
import { openAgent, type DecodedAgent } from '@fractal/sdk/effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as Option from 'effect/Option'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed } from '../data/source.ts'
import type { Feed } from '../data/source.ts'
import { gatewayNativeDetails } from '../data/nativeDetailsSource.ts'
import recording from '../data/subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { AgentDetailsPanel } from './AgentDetailsPanel.tsx'
import { ompDetailsSource } from './ompDetailsFixtures.ts'
import { ompAgentRef, ompSubagent, ompSupplementFeeds, type OmpSupplementState } from '../../../../packages/fractal-ui/src/assistant-ui/details/omp-fixtures.ts'
import { accountingNoteSummary, spendWindowReason } from './detailsSupplement.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const row = openAgent(decodeUnknownSync(Agent)({
  kind: 'agent', id: 'agent/synthetic', name: 'Synthetic Details seat', host_id: 'host/synthetic', runtime_ids: [],
  reachability: 'reachable', state: 'running', revision: '1', incarnation_id: 'first', updated_at: '1970-01-01T00:00:04.000Z',
  usage: { aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 480_035_214, context: { used_tokens: 61_206, window_tokens: 1_000_000, used_percent: 6.1206, observed_at_unix_ms: 3000, model: 'synthetic-reported-model' } },
}))
let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
let agents: Atom.Writable<Feed<readonly DecodedAgent[]>>
const onClose = vi.fn()
beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  window.localStorage.clear()
  registry = AtomRegistry.make()
  agents = Atom.make<Feed<readonly DecodedAgent[]>>(observed({ value: [row] }))
  const source = { ...fixtureSource({ world: { now: 10_000, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), agents }
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  onClose.mockClear()
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><AgentDetailsPanel agentRef="agent/synthetic" width={320} scheme="dark" onClose={onClose} onOpen={() => undefined} /></DataSourceProvider>))
})
afterEach(async () => { await act(async () => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals() })

describe('native observations mounted on the production kit Details pane', () => {
  it('shows real native identity/context and actual header controls without fixture fallback', () => {
    expect(host.querySelector('[data-testid="details-pane"]')).not.toBeNull()
    expect(host.textContent).toContain('Synthetic Details seat')
    expect(host.textContent).toContain('synthetic-reported-model')
    const row = host.querySelector<HTMLElement>('[data-agent-inspector-tabs]')!
    expect(within(row).getAllByRole('tab').map(tab => tab.textContent)).toEqual(['Changes', 'Details'])
    expect(within(row).getByRole('tab', { name: 'Details' }).getAttribute('aria-selected')).toBe('true')
    // Details' controls and the dock's close share the tab row; no generic workbench copy/theme/panel actions.
    expect(within(row).getByRole('button', { name: 'Visible sections' })).not.toBeNull()
    expect(within(row).getByRole('button', { name: 'Close agent inspector' })).not.toBeNull()
    expect(within(host).queryAllByRole('button', { name: /theme|copy|panel|split/i })).toEqual([])
    expect(host.querySelector('[data-testid="workbench"]')).toBeNull()
    const context = within(host).getByRole('progressbar', { name: 'Context window' })
    expect(Number(context.getAttribute('aria-valuenow'))).toBeCloseTo(6.1206, 6)
    expect(host.textContent).not.toMatch(/worker-1|Small Talk release|Self vs descendants.*\$0/)
    context.setAttribute('aria-valuenow', String(480_035_214 / 1_000_000 * 100))
    expect(() => expect(Number(context.getAttribute('aria-valuenow'))).toBeCloseTo(6.1206, 6)).toThrow()
  })
  it('labels token-only occupancy as estimated on the production panel', async () => {
    const usage = Option.getOrThrow(row.usage)
    const { used_percent: _reportedPercent, ...context } = usage.context!
    await act(async () => registry.set(agents, observed({ value: [{ ...row, usage: Option.some({ ...usage, context }) }] })))
    const meter = within(host).getByRole('progressbar', { name: 'Context window' })
    expect(Number(meter.getAttribute('aria-valuenow'))).toBeCloseTo(6.1206, 6)
    expect(meter.getAttribute('aria-valuetext')).toContain('estimated')
    expect(host.textContent).toContain('6% est.')
  })
  it('preserves a reported percentage over a conflicting token-derived estimate', async () => {
    const usage = Option.getOrThrow(row.usage)
    await act(async () => registry.set(agents, observed({ value: [{ ...row, usage: Option.some({ ...usage, context: { ...usage.context!, used_percent: 42 } }) }] })))
    const meter = within(host).getByRole('progressbar', { name: 'Context window' })
    expect(Number(meter.getAttribute('aria-valuenow'))).toBe(42)
    expect(meter.getAttribute('aria-valuetext') ?? '').not.toContain('estimated')
    expect(host.textContent).toContain('42%')
    expect(host.textContent).not.toContain('42% est.')
  })
  it('hides not-supplied resource sections and names their absence once, without inventing empty counts', async () => {
    for (const name of ['Pull requests', 'CI runs', 'Reviews', 'Files touched', 'Terminals', 'Missions', 'History'])
      expect(within(host).queryByRole('region', { name })).toBeNull()
    const footer = host.querySelector<HTMLElement>('[data-details-unreported-sections]')!
    expect(footer.querySelector('[data-details-unreported-summary]')?.textContent).toContain('Not reported for this session:')
    expect(footer.querySelector('[data-details-unreported-summary]')?.textContent).toContain('CI runs')
    const reasonTrigger = within(footer).getByRole('button', { name: 'Why? Sections not reported' })
    const panel = document.getElementById(reasonTrigger.getAttribute('aria-controls')!)!
    expect(reasonTrigger.getAttribute('aria-expanded')).toBe('false')
    expect(panel.hidden).toBe(true)
    await act(async () => reasonTrigger.click())
    expect(reasonTrigger.getAttribute('aria-expanded')).toBe('true')
    expect(panel.hidden).toBe(false)
    expect([...panel.querySelectorAll('[data-details-reason-text]')].map(reason => reason.textContent)).toContain('This session does not report CI runs.')
    expect(within(host).getByRole('region', { name: 'Attention' }).textContent).toContain('No attention action observed')
  })
  it('preserves Changes as a selectable real captured-changes surface and returns to Details', async () => {
    await act(async () => within(host).getByRole('tab', { name: 'Changes' }).click())
    expect(within(host).getByRole('tab', { name: 'Changes' }).getAttribute('aria-selected')).toBe('true')
    expect(host.querySelectorAll('aside[aria-label="Changes"]')).toHaveLength(1)
    expect(host.querySelector('[data-testid="details-pane"]')).toBeNull()
    // The hidden Details pane released its controls from the shared row.
    expect(host.querySelector('[data-agent-inspector-accessory]')?.childElementCount).toBe(0)
    expect(host.textContent).toContain('Captured changes unavailable')
    await act(async () => within(host).getByRole('tab', { name: 'Details' }).click())
    expect(host.querySelector('[data-testid="details-pane"]')).not.toBeNull()
    expect(host.querySelector('aside[aria-label="Changes"]')).toBeNull()
    expect(within(host.querySelector<HTMLElement>('[data-agent-inspector-tabs]')!).getByRole('button', { name: 'Visible sections' })).not.toBeNull()
  })
  it('re-fetches and re-projects a rejected declaration tree when Retry is pressed', async () => {
    let declarationReads = 0
    const options = { baseUrl: 'http://gateway.invalid', fetchImpl: async (input: RequestInfo | URL) => {
      const url = new URL(String(input))
      if (url.pathname.endsWith('/capabilities')) return Response.json(recording.capabilities)
      let value: unknown
      if (url.pathname.endsWith('/usage')) value = { since_ms: 1000, until_ms: 5000, rows: [] }
      else if (url.pathname.includes('/agent-declarations/')) {
        declarationReads++
        value = { id: row.id, revision: '1', revisions: ['1'], kdl: '', tree: declarationReads === 1
          ? { name: 'not-an-agent', children: [] }
          : { name: 'agent', children: [{ name: 'workspace', arguments: ['withheld'] }] } }
      } else throw new TypeError(`Unexpected native request: ${url.pathname}`)
      return Response.json({ ...recording.cases.agent.payload, value })
    } }
    const source = { ...fixtureSource({ world: { now: 10_000, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), agents,
      nativeDetails: gatewayNativeDetails({ options, registry, discovery: new FractalClient(options), now: () => 10_000 }),
    }
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><AgentDetailsPanel agentRef={row.id} width={320} scheme="dark" onClose={onClose} onOpen={() => undefined} /></DataSourceProvider>))
    await act(async () => { await vi.waitFor(() => expect(host.querySelector('[data-details-sync-state]')?.getAttribute('data-details-sync-state')).toBe('Failed')) })
    expect(declarationReads).toBe(1)
    // A malformed projection remains failed and retryable even though its raw getter succeeded.
    const declaration = source.nativeDetails.declarations({ agentRef: row.id, revision: row.revision })
    expect(registry.get(declaration)).toMatchObject({ _tag: 'Unavailable', reason: 'failed' })
    await act(async () => within(host).getByRole('button', { name: 'Details for agent details' }).click())
    await act(async () => within(document.body).getByRole('button', { name: 'Retry' }).click())
    await act(async () => { await vi.waitFor(() => expect(host.textContent).toContain('Declared workspace')) })
    expect(declarationReads).toBe(2)
    expect(registry.get(declaration)).toMatchObject({ _tag: 'Observed', value: { bindings: [{ kind: 'workspace', count: 1 }] } })
  })
  it('reads through the app source registry and closes through the host', async () => {
    await act(async () => registry.set(agents, observed({ value: [{ ...row, name: 'Updated Details seat' }] })))
    expect(host.textContent).toContain('Updated Details seat')
    expect(host.textContent).not.toContain('Synthetic Details seat')
    await act(async () => within(host).getByRole('button', { name: 'Close agent inspector' }).click())
    expect(onClose).toHaveBeenCalledOnce()
  })
  it('leaves the phone sheet its own close while keeping both tabs selectable', async () => {
    const source = { ...fixtureSource({ world: { now: 10_000, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), agents }
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><AgentDetailsPanel agentRef="agent/synthetic" width="fill" scheme="dark" onClose={onClose} onOpen={() => undefined} /></DataSourceProvider>))
    expect(within(host).queryByRole('button', { name: /close/i })).toBeNull()
    expect(within(host).getByRole('tab', { name: 'Details' }).getAttribute('aria-selected')).toBe('true')
    await act(async () => within(host).getByRole('tab', { name: 'Changes' }).click())
    expect(host.querySelectorAll('aside[aria-label="Changes"]')).toHaveLength(1)
    expect(host.querySelector('[data-testid="details-pane"]')).toBeNull()
  })
})

/** Internal vocabulary that must never reach the OMP inspector's text, accessible names or disclosed reasons. */
const internalWords = /\b(native\w*|descendants?|incarnations?|provenance|series|ledger|attribution|window-global|matched|traced|self)\b|root completeness|this source|\bst resources\b|\breported window\b/i
/** The one sentence allowed to say "matched"; exactly it is stripped before the ban applies. */
const matchedSentence = /\b\d+(?:\.\d+)?% of spend matched to this agent\b/g
/** A placeholder epoch never reaches the inspector, in ISO or display form. */
const epoch = /1970-01-01|\b1 Jan(?:uary)? 1970\b|\bJan(?:uary)? 1,? 1970\b/
const copyOf = (root: HTMLElement) => {
  const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT), text: string[] = []
  while (walker.nextNode()) text.push(walker.currentNode.textContent ?? '')
  return [...text, ...[...root.querySelectorAll('[aria-label]')].map(node => node.getAttribute('aria-label') ?? '')].join(' ').replace(matchedSentence, '')
}
const assertPlain = (root: HTMLElement) => {
  expect(copyOf(root)).not.toMatch(internalWords)
  expect(copyOf(root)).not.toMatch(epoch)
}
/** A spend row's label and period as painted, without its coverage line. */
const spendLabel = (spend: HTMLElement, metric: string) => {
  const dt = spend.querySelector(`[data-details-metric="${metric}"] dt`)!.cloneNode(true) as HTMLElement
  for (const node of dt.querySelectorAll('[data-details-coverage-line],[data-details-coverage]')) node.remove()
  return (dt.textContent ?? '').replace(/\s+/g, ' ').trim()
}

describe('OMP inspector copy guard', () => {
  it('rejects each banned term and every other use of matched, and the placeholder epoch', () => {
    for (const sample of ['Native session', 'Traced descendant', 'Traced sessions', 'Main session · self', 'Observed st resources', 'Reported window · 7d', 'Matched sessions', '99% matched', 'Spend matched to this agent', '100% of spend matched to this seat', '99% of spend matched to this session'])
      expect(sample.replace(matchedSentence, '')).toMatch(internalWords)
    expect('Spend incl. subagents · last 7 days. 99% of spend matched to this agent. Main session. Resources aren’t available for this session.'.replace(matchedSentence, '')).not.toMatch(internalWords)
    for (const sample of ['Main session · 1970-01-01 00:00–00:00 UTC', 'Started 1 Jan 1970', 'Jan 1, 1970']) expect(sample).toMatch(epoch)
  })
})

describe('OMP subagent activity on the production inspector', () => {
  const mount = async (state?: OmpSupplementState) => {
    await act(async () => root.render(<DataSourceProvider source={ompDetailsSource()} registry={registry}>
      <AgentDetailsPanel agentRef={ompAgentRef} width={320} scheme="dark" onClose={onClose} onOpen={() => undefined} {...(state === undefined ? {} : { detailsSupplement: ompSupplementFeeds[state] })} />
    </DataSourceProvider>))
  }
  const why = (scope: HTMLElement, name: string) => {
    const trigger = within(scope).getByRole('button', { name })
    return { trigger, panel: document.getElementById(trigger.getAttribute('aria-controls')!)! }
  }
  it('announces loading in plain words, with no technical paragraph', async () => {
    await mount('loading')
    const line = host.querySelector<HTMLElement>('[data-details-supplement]')!
    expect(line.getAttribute('data-details-supplement-state')).toBe('Waiting')
    expect(within(line).getByRole('status').textContent).toBe('Loading subagent activity…')
    expect(line.querySelector('button[data-details-why]')).toBeNull()
    assertPlain(host)
  })
  it('keeps the build composition’s unavailable reason plain and behind a collapsed Why?', async () => {
    await mount()
    const line = host.querySelector<HTMLElement>('[data-details-supplement]')!
    expect(within(line).getByRole('status').textContent).toBe("Subagent activity isn't available for this session.")
    const { trigger, panel } = why(line, 'Why? Subagent activity')
    expect(trigger.getAttribute('aria-expanded')).toBe('false')
    expect(panel.hidden).toBe(true)
    expect(panel.textContent).toBe("Subagent activity isn't reported for this session.")
    assertPlain(host)
  })
  it('never renders a source’s technical failure detail', async () => {
    await mount('unavailable')
    const line = host.querySelector<HTMLElement>('[data-details-supplement]')!
    const { trigger, panel } = why(line, 'Why? Subagent activity')
    await act(async () => trigger.click())
    expect(panel.hidden).toBe(false)
    expect(panel.textContent).toBe("Subagent activity couldn't be loaded.")
    assertPlain(host)
    // Missing spend shares one summary; which session spend belongs to is a Why?-only note, never a field.
    const spend = host.querySelector<HTMLElement>('section[aria-label$="Spend"]')!
    const partial = spend.querySelector<HTMLElement>('[data-details-not-reported="fields"]')!
    const summary = partial.querySelector('[data-details-not-reported-summary]')?.textContent ?? ''
    expect(summary).toMatch(/^Not reported: /)
    expect(summary).toContain('total spend')
    expect(summary).toContain('main session window')
    expect(summary).not.toMatch(/matching|scope/i)
    expect(spendLabel(spend, 'main-usd')).toBe('Main session')
    const reasons = why(partial, 'Why? Spend details not reported')
    expect(reasons.trigger.getAttribute('aria-expanded')).toBe('false')
    expect(reasons.panel.hidden).toBe(true)
    expect(reasons.panel.textContent).toContain('Which session this spend belongs to isn’t reported.')
    expect(copyOf(host)).not.toContain('root not found')
  })
  it('states separate spend reports in one plain sentence, with the time window behind Why?', async () => {
    await mount('reported')
    expect(host.querySelector('[data-details-supplement]')).toBeNull()
    const note = host.querySelector<HTMLElement>('[data-details-accounting-note]')!
    expect(note.querySelector('[data-details-accounting-summary]')?.textContent).toBe(accountingNoteSummary)
    const { trigger, panel } = why(note, 'Why? Agent and total spend')
    await act(async () => trigger.click())
    expect(panel.textContent).toBe(spendWindowReason)
    assertPlain(host)
    // Plain accounting: the reported period reads in words, the main session's unknown period stays unknown.
    const spend = host.querySelector<HTMLElement>('section[aria-label$="Spend"]')!
    expect(spendLabel(spend, 'total-usd')).toBe('Spend incl. subagents · last 7 days')
    expect(spendLabel(spend, 'main-usd')).toBe('Main session')
    expect(spend.querySelector('[data-details-not-reported-summary]')?.textContent).toContain('main session window')
    const coverage = spend.querySelector<HTMLElement>('[data-details-metric="total-usd"] [data-details-coverage-line]')!
    expect(coverage.querySelector('[data-details-coverage]')?.textContent).toBe('99% of spend matched to this agent')
    const linked = why(coverage, 'Why? Spend linked to this agent')
    expect(linked.trigger.getAttribute('aria-expanded')).toBe('false')
    expect(linked.panel.textContent).toContain('This percentage shows how much spend could be linked to this agent.')
    // A complete subagent list needs no count line.
    expect(host.querySelector('[data-details-subagent-window]')?.textContent).toBe('Subagents · last 7 days')
    expect(host.querySelector('[data-details-subagent-count]')).toBeNull()
  })
  it('describes retained activity honestly without the source’s failure detail', async () => {
    await mount('retained')
    const line = host.querySelector<HTMLElement>('[data-details-supplement]')!
    expect(within(line).getByRole('status').textContent).toBe('Subagent activity may be out of date (last updated just now).')
    const { trigger, panel } = why(line, 'Why? Subagent activity')
    await act(async () => trigger.click())
    expect(panel.textContent).toBe("The latest update couldn't be loaded, so earlier activity is shown.")
    assertPlain(host)
  })
  it('opens a subagent’s reported details in plain words', async () => {
    await mount('reported')
    const card = [...host.querySelectorAll<HTMLElement>('button')].find(button => button.textContent?.includes(ompSubagent.label))!
    await act(async () => card.click())
    const dialog = within(document.body).getByRole('dialog', { name: 'Reported resource details' })
    expect(dialog.textContent).toContain("Reported details only; this resource can't be opened from here.")
    expect(dialog.textContent).toContain(`Session ${ompSubagent.sessionId} · 12 requests · $2.86 · ${ompSubagent.model}`)
    assertPlain(dialog)
  })
})
