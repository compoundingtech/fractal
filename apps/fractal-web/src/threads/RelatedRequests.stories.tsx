/** A thread's Related requests: the real LiveThreadPane reading a local request history page in every state. */
import { FractalClient } from '@fractal/client'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import * as Native from '@fractal/client/schema/read'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import gatewayFixture from '../data/subjectReadPort.gateway.fixtures.json'
import { observed, unavailable, waiting, type Feed } from '../data/source.ts'
import { LiveThreadPane } from './LiveThreads.tsx'
import { gatewayThreads, type ThreadDraft, type ThreadSource } from './threadSource.ts'

const actor = 'person/operator'
const agentId = 'agent/build-host-a/release-desk'
const threadId = 'thread/release-approval'
const approval = 'step-run/run-generation-7/release/approve'
const ci = 'step-run/run-generation-7/release/ci'
const scoping = 'step-run/run-generation-7/release/scope'
const abc = 'claim/ask-abc'
const def = 'run-generation/7:2:4'
const ciEpisode = 'claim/ask-ci'
const scopingEpisode = 'claim/ask-scope'
const now = Date.parse('2026-11-17T10:00:00Z')

type Scenario = 'branch' | 'long-history-cross-page' | 'unavailable' | 'hidden-predecessor' | 'loading' | 'stale' | 'refused' | 'failed' | 'unsupported'
type Args = {
  readonly scheme: 'dark' | 'light'
  /** Desktop thread tab at 1440px, or the phone layout at 390px. */
  readonly viewport: 1440 | 390
  readonly scenario: Scenario
}

const key = (source_id: string, episode: string) => ({ source_id, episode })
const runtime = (claim: string) => ({ kind: 'runtime-ask', asking_claim_id: claim })
const available = (title: string | null, text: string, provenance: object, references: readonly object[] = []) =>
  ({ availability: 'available', title, text, request: null, references, provenance })
const state = (status: string, answers: readonly object[] = []) => ({ availability: 'available', status, answers })
const page = (value: object): Native.RequestHistoryPage => Native.decodeUnknownSync(Native.RequestHistoryPage)({
  items: [], relations: [], source_episodes: [], boundary_items: [], source_support: 'person-step', next_cursor: null, complete: true, ...value,
})

/** The adopted branch: abc approved, a separate CI request asked after it, and the same step asked again for def. */
const branch = page({
  boundary_items: [{ key: key(scoping, scopingEpisode), complete: true, state: state('completed'),
    payload: available('Scope the release', 'Which services ship in 4.2?', runtime(scopingEpisode)) }],
  items: [
    { key: key(approval, abc), complete: true,
      payload: available('Approve revision abc', 'Approve revision abc for the 4.2 release.', runtime(abc), [
        { target_id: 'https://example.test/release/pull/12', revision: 'abc', semantics: 'retained-reference' },
        { target_id: 'doc/release-notes', revision: null, semantics: 'live-reference' },
      ]),
      state: state('completed', [{ ask: approval, status: 'completed', summary: 'Approved abc; rollback steps look right.', respondent: actor, answered_at_unix_ms: now - 2 * 60 * 60_000 }]) },
    { key: key(approval, def), complete: true,
      payload: available('Approve revision abc', 'Approve revision abc for the 4.2 release.\nReview def instead: it fixes the migration order.',
        { kind: 'authored-episode', mission_revision: 'revision/7', opening_claim_id: 'claim/open-def' }),
      state: state('ready') },
    { key: key(ci, ciEpisode), complete: true, payload: available('Run CI on abc', 'Start the release CI for revision abc.', runtime(ciEpisode)), state: state('ready') },
  ],
  relations: [
    { predecessor: key(scoping, scopingEpisode), request: key(approval, abc), reason: 'Scope settled, so the approval was requested.' },
    { predecessor: key(approval, abc), request: key(ci, ciEpisode), reason: 'Separate CI request for the approved revision.' },
  ],
  source_episodes: [{ source_id: approval, episodes: [key(approval, abc), key(approval, def)] }],
})

const longHistoryItems = Array.from({ length: 45 }, (_, index) => {
  const episode = `claim/release-review-${index + 1}`
  return { key: key(approval, episode), complete: true,
    payload: available(`Release review ${index + 1}`, `Review release revision ${index + 1}.`, runtime(episode)),
    state: state(index === 44 ? 'ready' : 'completed') }
})
// Forty items on the first page, including the selected request's different-source predecessor.
// The 45 episodes of the approval source stay grouped; they are never linked to each other.
const requestHistoryPages = [
  { items: [{ key: key(scoping, scopingEpisode), complete: true, state: state('completed'),
      payload: available('Scope the release', 'Which services ship in 4.2?', runtime(scopingEpisode)) }, ...longHistoryItems.slice(0, 39)],
    boundary_items: [], relations: [],
    source_episodes: [{ source_id: approval, episodes: longHistoryItems.slice(0, 39).map(item => item.key) }],
    source_support: 'person-step', next_cursor: 'history/release-review/page-2', complete: true },
  { items: longHistoryItems.slice(39), boundary_items: [],
    relations: [{ predecessor: key(scoping, scopingEpisode), request: longHistoryItems[44]!.key, reason: 'The release scope was settled before this review.' }],
    source_episodes: [{ source_id: approval, episodes: longHistoryItems.map(item => item.key) }],
    source_support: 'person-step', next_cursor: null, complete: true },
]

const initialHistory = (scenario: Scenario): Feed<Native.RequestHistoryPage> => {
  switch (scenario) {
    case 'branch': return observed({ value: branch })
    case 'stale': return observed({ value: branch, freshness: 'stale' })
    case 'long-history-cross-page': return waiting
    case 'loading': return waiting
    case 'unavailable': return observed({ value: page({
      complete: false,
      items: [
        { key: key(approval, abc), complete: false, payload: { availability: 'unavailable', reason: 'retained-definition-unavailable' }, state: { availability: 'unavailable' } },
        { key: key(approval, def), complete: true, payload: available('Review def', 'Review def instead: it fixes the migration order.', runtime(def)), state: state('ready') },
      ],
      source_episodes: [{ source_id: approval, episodes: [key(approval, abc), key(approval, def)] }],
    }) })
    case 'hidden-predecessor': return observed({ value: page({
      items: [
        { key: key(approval, abc), complete: true, payload: available('Approve revision abc', 'Approve revision abc for the 4.2 release.', runtime(abc)), state: state('completed') },
        { key: key(ci, ciEpisode), complete: true, payload: available('Run CI on abc', 'Start the release CI for revision abc.', runtime(ciEpisode)), state: state('ready') },
      ],
      // The reader returned an arrow whose predecessor it did not return: nothing about it is drawn.
      relations: [{ predecessor: key('step-run/run-generation-9/private/ask', 'claim/private'), request: key(ci, ciEpisode), reason: 'Private reason' }],
    }) })
    case 'refused': return unavailable({ reason: 'ungranted', detail: 'Related requests are not available to you.' })
    case 'failed': return unavailable({ reason: 'failed', detail: 'Fractal could not load related requests.' })
    case 'unsupported': return observed({ value: page({ source_support: 'unsupported-source' }) })
  }
}

/** A local thread source: one open thread with a short conversation and its request history in the chosen state. */
const createFixture = (scenario: Scenario) => {
  const registry = AtomRegistry.make()
  const agent = Native.decodeUnknownSync(Native.Agent)({
    kind: 'agent', id: agentId, name: 'Release desk', host_id: 'host/build-host-a', driver: 'omp', state: 'waiting',
    blocked_on: null, fault: null, reason: 'Waiting for approval.', runtime_ids: [], reachability: 'reachable', revision: '1', updated_at: new Date(now).toISOString(),
  })
  const summary = Native.decodeUnknownSync(Native.ThreadSummary)({
    id: threadId, title: 'Release approval', state: 'open', agent_id: agentId, title_revision: 'claim/title', revision: 'claim/current', message_count: 1, unread: false,
  })
  const messages = [Native.decodeUnknownSync(Native.ThreadMessageEvent)({
    kind: 'message', id: 'message/approval', actor_id: agentId, accepted_at_unix_ms: now - 30 * 60_000,
    content: 'def fixes the migration order; the abc approval still stands on its own.', thread_quote_id: null,
  })]
  const history = Atom.keepAlive(Atom.make(initialHistory(scenario)))
  const drafts = Atom.family((_destination: string) => Atom.keepAlive(Atom.make<ThreadDraft>({ content: '' })))
  const pagedHistory = scenario === 'long-history-cross-page' ? gatewayThreads(new FractalClient({
    baseUrl: 'http://example.test',
    fetchImpl: async input => {
      const url = new URL(String(input))
      const base = gatewayFixture.capabilities
      if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 3, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
      ] } })
      if (!url.pathname.endsWith('/request-history')) throw new Error('Unexpected request-history fixture read')
      const cursor = url.searchParams.get('cursor')
      if (cursor !== null && cursor !== requestHistoryPages[0]!.next_cursor) throw new Error('Unexpected request-history fixture cursor')
      return Response.json({ ...base, value: requestHistoryPages[cursor === null ? 0 : 1] })
    },
  }), registry) : undefined
  const noChange = { detail: 'This story does not change threads.' }
  const threads: ThreadSource = {
    access: Atom.make({ read: true, actor, actions: new Set(['thread.reply']) }),
    titles: Atom.make(new Map([[threadId, summary.title]])),
    list: () => Atom.make(observed({ value: { items: [summary], next_cursor: Option.none() } })),
    detail: () => Atom.make(observed({ value: { summary, snapshotId: 'snapshot/request-history' } })),
    timeline: () => Atom.make(observed({ value: { items: messages, next_cursor: Option.none() } })),
    draft: drafts,
    requestHistory: pagedHistory?.requestHistory ?? (() => history),
    // Retry reads again; the story's second read succeeds.
    reloadRequestHistory: pagedHistory?.reloadRequestHistory ?? (() => registry.set(history, observed({ value: branch }))),
    start: async () => ({ _tag: 'Refused', detail: 'This story does not send messages.' }),
    reply: async () => ({ _tag: 'Refused', detail: 'This story does not send messages.' }),
    rename: async () => noChange,
    changeState: async () => noChange,
  }
  const source = { ...fixtureSource({ world: {
    now, events: [], missions: [], attention: [], agents: [agent], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }), threads }
  return { registry, source }
}

const RequestHistoryStory = ({ scheme, viewport, scenario }: Args) => {
  const [fixture] = React.useState(() => createFixture(scenario))
  React.useEffect(() => () => fixture.registry.dispose(), [fixture])
  return <main data-scheme={scheme} data-viewport={viewport} style={{ width: viewport }}
    {...stylex.props(styles.canvas, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <NarrowLayoutContext.Provider value={viewport < 900}>
      <DataSourceProvider source={fixture.source} registry={fixture.registry}>
        {/* The thread's own tab fills its pane; the pane here has a fixed height. */}
        <div data-request-history-scene={scenario} {...stylex.props(styles.pane)}><LiveThreadPane threadId={threadId} /></div>
      </DataSourceProvider>
    </NarrowLayoutContext.Provider>
  </main>
}

const scenarios: readonly Scenario[] = ['branch', 'long-history-cross-page', 'unavailable', 'hidden-predecessor', 'loading', 'stale', 'refused', 'failed', 'unsupported']
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Threads/Related requests',
  component: RequestHistoryStory,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'Thread request history pages (related requests, same-source episodes, boundary predecessors, unavailable payloads, refused and unsupported reads) served by a local source; no world slice models request history. No daemon writes.' },
    docs: { description: { component: 'A thread shows the requests related to it, each with its own original wording and its own status. “Requested after” explains why one request followed another and moves to it; it never makes one a prerequisite of, or an answer to, the other. Episodes of the same step stay together. Missing original wording is marked unavailable, never replaced by today’s text; requests the reader did not return leave no trace.' } },
  },
  args: { scheme: 'dark', viewport: 1440, scenario: 'branch' },
  argTypes: {
    scheme: { options: ['dark', 'light'], control: 'inline-radio' },
    viewport: { options: [1440, 390], control: 'inline-radio' },
    scenario: { options: scenarios, control: 'select' },
  },
  render: args => <RequestHistoryStory key={`${args.scenario}:${args.viewport}:${args.scheme}`} {...args} />,
} satisfies Meta<Args>
export default meta

type Story = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'Branch:contract-fixture',
  'BranchLight:contract-fixture',
  'BranchNarrow:contract-fixture',
  'BranchNarrowLight:contract-fixture',
  'Failed:contract-fixture',
  'HiddenPredecessor:contract-fixture',
  'Loading:contract-fixture',
  'LongHistoryCrossPage:contract-fixture',
  'Refused:contract-fixture',
  'Stale:contract-fixture',
  'Unavailable:contract-fixture',
  'Unsupported:contract-fixture',
  'Walkthrough:contract-fixture',
]

const row = (canvasElement: HTMLElement, source: string, episode: string) =>
  canvasElement.querySelector<HTMLElement>(`[data-request-source="${source}"][data-request-episode="${episode}"]`)
const statusOf = (canvasElement: HTMLElement, source: string, episode: string) =>
  row(canvasElement, source, episode)?.querySelector('[data-request-status-label]')?.textContent

/** Each request keeps its own status: the completed abc approval does not complete CI or the def episode. */
const expectBranch = async (canvasElement: HTMLElement) => {
  await waitFor(() => expect(row(canvasElement, ci, ciEpisode)).not.toBeNull())
  await expect(statusOf(canvasElement, approval, abc)).toBe('Completed')
  await expect(statusOf(canvasElement, approval, def)).toBe('Open')
  await expect(statusOf(canvasElement, ci, ciEpisode)).toBe('Open')
  await expect(row(canvasElement, ci, ciEpisode)?.querySelector('[data-request-after]')?.textContent).toContain('Separate CI request for the approved revision.')
  await expect(canvasElement.querySelector(`[data-request-same-source="${approval}"]`)?.querySelectorAll('[data-request-episode]')).toHaveLength(2)
}
const branchPlay: Story['play'] = ({ canvasElement }) => expectBranch(canvasElement)
export const Branch: Story = { play: branchPlay }

/** Local cursor-paged fixture, not authenticated live API. */
export const LongHistoryCrossPage: Story = {
  args: { scenario: 'long-history-cross-page' },
  parameters: { docs: { description: { story: '45 episodes of one source across two local request-history pages, with a predecessor on the earlier page; not authenticated live API.' } } },
  play: async ({ canvasElement }) => {
    const selectedEpisode = longHistoryItems[44]!.key.episode
    await waitFor(() => expect(row(canvasElement, approval, selectedEpisode)).not.toBeNull())
    await expect(canvasElement.querySelector(`[data-request-same-source="${approval}"]`)?.querySelectorAll('[data-request-episode]')).toHaveLength(45)
    const selected = row(canvasElement, approval, selectedEpisode)!
    const predecessor = row(canvasElement, scoping, scopingEpisode)!
    await expect(predecessor).not.toBeNull()
    await userEvent.click(within(selected).getByRole('button', { name: 'Requested after “Scope the release”' }))
    await waitFor(() => expect(canvasElement.ownerDocument.activeElement).toBe(predecessor))
    await expect(predecessor.getAttribute('data-request-source')).toBe(scoping)
    await expect(predecessor.getAttribute('data-request-episode')).toBe(scopingEpisode)
  },
}
export const BranchLight: Story = { args: { scheme: 'light' }, play: branchPlay }
export const BranchNarrow: Story = { args: { viewport: 390 }, play: branchPlay }
export const BranchNarrowLight: Story = { args: { viewport: 390, scheme: 'light' }, play: branchPlay }

/** Missing original wording says so and never borrows the step's current text; the page is marked incomplete. */
export const Unavailable: Story = {
  args: { scenario: 'unavailable' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(row(canvasElement, approval, abc)).not.toBeNull())
    await expect(row(canvasElement, approval, abc)?.querySelector('[data-request-title]')?.textContent).toBe('Original request unavailable')
    await expect(row(canvasElement, approval, abc)?.textContent).not.toContain('def')
    await expect(canvasElement.querySelector('[data-request-history-incomplete]')).not.toBeNull()
  },
}

/** An arrow to a request the reader did not return draws nothing: no link, reason or identifier. */
export const HiddenPredecessor: Story = {
  args: { scenario: 'hidden-predecessor' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(row(canvasElement, ci, ciEpisode)).not.toBeNull())
    await expect(canvasElement.querySelector('[data-request-after]')).toBeNull()
    await expect(canvasElement.innerHTML).not.toContain('private')
    await expect(canvasElement.textContent).not.toContain('Private reason')
  },
}

export const Loading: Story = { args: { scenario: 'loading' } }

/** A paused read keeps the last rows and offers Retry. */
export const Stale: Story = {
  args: { scenario: 'stale' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelector('[data-request-history] [data-system-state="stale"]')).not.toBeNull())
    await expect(row(canvasElement, ci, ciEpisode)).not.toBeNull()
  },
}

/** A refused read names no request; the conversation is unchanged. */
export const Refused: Story = {
  args: { scenario: 'refused' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelector('[data-request-history] [data-system-state="error"]')).not.toBeNull())
    await expect(canvasElement.querySelector('[data-request-episode]')).toBeNull()
    await expect(canvasElement.querySelector('textarea')).not.toBeNull()
  },
}

/** A failed read offers Retry; reading again shows the history. */
export const Failed: Story = {
  args: { scenario: 'failed' },
  play: async ({ canvasElement }) => {
    const section = await waitFor(() => {
      const found = canvasElement.querySelector<HTMLElement>('[data-request-history]')
      if (found === null) throw new Error('The section is not shown yet')
      return found
    })
    await userEvent.click(within(section).getByRole('button', { name: 'Retry' }))
    await waitFor(() => expect(row(canvasElement, ci, ciEpisode)).not.toBeNull())
  },
}

export const Unsupported: Story = {
  args: { scenario: 'unsupported' },
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelector('[data-request-history-unsupported]')).not.toBeNull())
  },
}

/** Read the branch back from the CI request to the approval it followed, then to the scoping request before that. */
export const Walkthrough: Story = {
  play: async ({ canvasElement }) => {
    await expectBranch(canvasElement)
    const followed = row(canvasElement, ci, ciEpisode)!
    await userEvent.click(within(followed).getByRole('button', { name: 'Requested after “Approve revision abc”' }))
    await waitFor(() => expect(canvasElement.ownerDocument.activeElement).toBe(row(canvasElement, approval, abc)))
    // The approval itself followed the scoping request, shown at the boundary of this page.
    await userEvent.click(within(row(canvasElement, approval, abc)!).getByRole('button', { name: 'Requested after “Scope the release”' }))
    await waitFor(() => expect(canvasElement.ownerDocument.activeElement).toBe(row(canvasElement, scoping, scopingEpisode)))
  },
}

/** Compare every state in one gallery. */
export const AllStates: Story = {
  render: args => <div style={{ display: 'flex', flexWrap: 'wrap', gap: 16 }}>
    {scenarios.map(scenario => <section key={scenario} aria-label={scenario}>
      <h2>{scenario}</h2>
      <RequestHistoryStory {...args} scenario={scenario} viewport={390} />
    </section>)}
  </div>,
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  pane: { position: 'relative', height: 760, minWidth: 0 },
})
