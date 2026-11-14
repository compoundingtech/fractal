import type { Meta, StoryObj } from '@storybook/react-vite'
import * as Native from '@fractal/client/schema/read'
import { ActionResult } from '@fractal/client/schema'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { Radio, RadioGroup } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { DataSourceProvider } from '../data/react.tsx'
import { credentialExpiredMessage, observed, unavailable, waiting, type AttachmentPort } from '../data/source.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { ConversationItem } from '../conversation/model.ts'
import { LiveAgentWorkspace, type WorkspaceNavigation } from '../web/LiveAgentWorkspace.tsx'
import type { ThreadDraft, ThreadSource } from './threadSource.ts'

const review = 'Review PR #42'
const existing = 'Worker tests'
const askId = 'step-run/thread-story/review'
const episode = 'claim/thread-story/review'
const existingId = 'thread/worker-tests'
const closedId = 'thread/rollout-notes'
const actor = 'person/operator'
type Scenario = 'existing' | 'empty' | 'closed' | 'multiple' | 'custom-source' | 'clock-skew' | 'loading' | 'refused' | 'failed' | 'offline' | 'unconfirmed'

function createThreadStory(readOnly: boolean, scenario: Scenario, ordering: 'timestamp' | 'action' = 'timestamp') {
  const agentRef = 'agent/build-host-a/worker-tests'
  const registry = AtomRegistry.make()
  const now = scenario === 'clock-skew' ? Date.parse('2026-11-09T13:00:00Z') : Date.now()
  const agent = Native.decodeUnknownSync(Native.Agent)({
    kind: 'agent', id: agentRef, name: 'Worker tests', host_id: 'host/build-host-a', driver: 'omp',
    state: 'waiting', blocked_on: 'human', fault: null, reason: 'Worker report ready; waiting for follow-up checks.',
    runtime_ids: [], reachability: 'reachable', revision: '1', updated_at: new Date(now).toISOString(),
  })
  const conversation = [
    ConversationItem.cases.Text.make({ id: 'conversation/worker-request', role: 'user', text: 'Run the worker tests for PR #42 and report any remaining checks.', attachments: [], streaming: false, at: new Date(now - 26 * 60_000).toISOString() }),
    ConversationItem.cases.Text.make({ id: 'conversation/worker-report', role: 'assistant', text: 'Worker tests pass on Linux. The macOS job is still running. The Inbox report is ready for follow-up discussion.', attachments: [], streaming: false, at: new Date(now - 2 * 60_000).toISOString() }),
  ]
  const base = { source: fixtureSource({ world: {
    ...fixtureProjections, now, agents: [agent], missions: [], attention: [], events: [],
    conversations: { [agentRef]: { items: conversation, hasOlder: false } }, terminals: {}, envelopes: {}, proposed: {},
  } }) }
  const originSource = scenario === 'custom-source' ? 'custom/garden/report/v1/origin' : askId
  const sourceTitle = scenario === 'custom-source' ? 'Worker report ready' : review
  const attention = (name: string, subject: string, claim: string, unread?: true) => Native.decodeUnknownSync(Native.Attention)({
    id: `attention/${claim.split('/').at(-1)}`, kind: 'attention', revision: claim, episode: claim,
    person_id: actor, source_id: subject, requester_id: agentRef, conversation_id: agentRef, attention_kind: scenario === 'custom-source' && subject === originSource ? 'custom.garden.report' : 'agent-request',
    title: name, detail: scenario === 'custom-source' && subject === originSource ? 'The worker run finished. Discuss follow-up checks here.' : 'Can we proceed with this change?', priority: 'high', state: 'open', alert: true,
    actions: scenario === 'custom-source' && subject === originSource ? [] : ['work.done'], requested_at: new Date(now - 25 * 60_000).toISOString(), updated_at: new Date(now).toISOString(),
    ...(unread === undefined ? {} : { thread_unread: unread }),
  })
  const summary = (id: string, title: string, state: 'open' | 'closed', count: number, unread: boolean) =>
    Native.decodeUnknownSync(Native.ThreadSummary)({
      id, title, state, agent_id: agentRef, title_revision: 'claim/title', revision: 'claim/current', message_count: count, unread,
    })
  const message = (id: string, author: string, minutes: number, content: string, quote?: string) =>
    Native.decodeUnknownSync(Native.ThreadMessageEvent)({
      kind: 'message', id, actor_id: author, accepted_at_unix_ms: now + minutes * 60_000, content, thread_quote_id: quote ?? null,
    })
  const summaries = Atom.make<readonly Native.ThreadSummary[]>(scenario === 'empty' || scenario === 'custom-source' ? [] : [
    summary(existingId, existing, 'open', 3, true), summary(closedId, 'Rollout notes', 'closed', 1, false),
    ...(scenario === 'multiple' ? [summary('thread/cancellation', 'Cancellation behavior', 'open', 1, false)] : []),
  ])
  const history = Atom.make<Readonly<Record<string, readonly Native.ThreadTimelineEvent[]>>>({
    [existingId]: [
      ...(scenario === 'clock-skew' ? [
        Native.decodeUnknownSync(Native.ThreadTimelineEvent)({ kind: 'thread.reopened', id: 'event/skew-reopen', actor_id: actor, accepted_at_unix_ms: now - 120_000 }),
        Native.decodeUnknownSync(Native.ThreadTimelineEvent)({ kind: 'thread.closed', id: 'event/skew-close', actor_id: actor, accepted_at_unix_ms: now - 60_000 }),
      ] : []),
      message('message/review-ready', agentRef, -2, 'Worker tests pass on Linux. The macOS job is still running.'),
      message('message/review-scope', actor, -9, 'Can you verify the cancellation case too?'),
      message('message/review-first', agentRef, -24, 'The retry fix is ready. I will verify the worker tests before merge.'),
    ],
    [closedId]: [
      Native.decodeUnknownSync(Native.ThreadTimelineEvent)({
        kind: 'thread.closed', id: 'event/rollout-closed', actor_id: actor, accepted_at_unix_ms: now - 35 * 60_000,
      }),
      message('message/rollout-notes', actor, -40, 'Keep the original rollout plan in the release notes.'),
    ],
    'thread/cancellation': [message('message/cancellation', agentRef, -5, 'Cancellation keeps the original attempt fenced. Should we cover the retry case too?')],
  })
  const origins = Atom.make<Readonly<Record<string, string>>>({ [existingId]: originSource, [closedId]: originSource, 'thread/cancellation': originSource })
  const drafts = Atom.family((_id: string) => Atom.keepAlive(Atom.make<ThreadDraft>({ content: '' })))
  const update = (id: string, edit: (row: Native.ThreadSummary) => Native.ThreadSummary) =>
    registry.set(summaries, registry.get(summaries).map(row => row.id === id ? edit(row) : row))
  let next = 0
  const append = (id: string, content: string, quote?: string) => {
    const rows = registry.get(history)
    const previous = rows[id] ?? []
    registry.set(history, { ...rows, [id]: [
      message(`message/story-${++next}`, actor, 0, content, quote), ...previous,
    ] })
    update(id, row => ({ ...row, message_count: previous.filter(event => event.kind === 'message').length + 1 }))
  }
  // Read-state fixtures: each states only what the fixture source reports, never agent progress.
  const failing = Atom.make(scenario === 'failed')
  const unconfirmed = Atom.make<string | undefined>(scenario === 'unconfirmed' ? 'rename' : undefined)
  const threads: ThreadSource = {
    access: Atom.make({ read: true, inboxOrigins: true, actor, actions: new Set<string>(readOnly
      ? [] : ['thread.start', 'thread.reply', 'thread.rename', 'thread.close', 'thread.reopen', 'thread.read']) }),
    titles: Atom.make(get => new Map(get(summaries).map(row => [row.id, row.title]))),
    list: query => Atom.make(get => scenario === 'loading' ? waiting
      : scenario === 'refused' ? unavailable({ reason: 'unauthorized', detail: credentialExpiredMessage })
      : get(failing) ? unavailable({ reason: 'failed', detail: 'Fractal could not load threads.' })
      : observed({ freshness: scenario === 'offline' ? 'stale' : 'live', value: {
        items: get(summaries).filter(row => 'recently_closed' in query ? row.state === 'closed'
          : 'origin_source' in query ? get(origins)[row.id] === query.origin_source && query.origin_episode === episode : true),
        next_cursor: Option.none(),
      } })),
    reload: () => registry.set(failing, false),
    detail: id => Atom.make(get => {
      const row = get(summaries).find(candidate => candidate.id === id)
      return row === undefined ? unavailable({ reason: 'failed', detail: 'Fixture thread unavailable' }) : observed({ freshness: scenario === 'offline' ? 'stale' : 'live', value: { summary: row, snapshotId: 'snapshot/thread-story' } })
    }),
    timeline: id => Atom.make(get => observed({ freshness: scenario === 'offline' ? 'stale' : 'live', value: { items: scenario === 'clock-skew' && ordering === 'timestamp' ? [...(get(history)[id] ?? [])].sort((a, b) => b.accepted_at_unix_ms - a.accepted_at_unix_ms) : get(history)[id] ?? [], next_cursor: Option.none() } })),
    pendingChange: () => unconfirmed,
    retryChange: async () => { registry.set(unconfirmed, undefined); return undefined },
    draft: drafts,
    start: async (_draft, input) => {
      const id = `thread/story-${++next}`
      registry.set(summaries, [...registry.get(summaries), summary(id, input.title, 'open', 0, false)])
      registry.set(origins, { ...registry.get(origins), [id]: input.origin.source_id })
      append(id, input.content)
      return { _tag: 'Sent', threadId: id }
    },
    reply: async input => { append(input.target_id, input.content, input.thread_quote_id); return { _tag: 'Sent', threadId: input.target_id } },
    rename: async (observation, title) => {
      const id = observation.summary.id
      const event = Native.decodeUnknownSync(Native.ThreadTimelineEvent)({
        kind: 'thread.renamed', id: `event/story-${++next}`, actor_id: actor, accepted_at_unix_ms: now + next,
        previous_title: observation.summary.title, title,
      })
      registry.set(history, { ...registry.get(history), [id]: [event, ...(registry.get(history)[id] ?? [])] })
      update(id, row => Native.decodeUnknownSync(Native.ThreadSummary)({ ...row, title, title_revision: `claim/title-${next}` }))
      return undefined
    },
    changeState: async (observation, state) => {
      const id = observation.summary.id
      const event = Native.decodeUnknownSync(Native.ThreadTimelineEvent)({
        kind: state === 'closed' ? 'thread.closed' : 'thread.reopened', id: `event/story-${++next}`,
        actor_id: actor, accepted_at_unix_ms: now + next,
      })
      registry.set(history, { ...registry.get(history), [id]: [event, ...(registry.get(history)[id] ?? [])] })
      update(id, row => ({ ...row, state }))
      return undefined
    },
    markRead: async (id, _through, seen) => {
      update(id.summary.id, row => ({ ...row, unread: !seen.includes('message/review-ready') }))
      return undefined
    },
  }
  const conversationFeed = Atom.make(observed({ value: { items: conversation, hasOlder: false } }))
  const attachments: AttachmentPort = {
    capabilities: async () => ({ _tag: 'Success', value: { send: readOnly ? 'ungranted' : 'granted', upload: 'ungranted', read: 'ungranted' } }),
    upload: async () => ({ _tag: 'Refused', reason: 'ungranted', detail: 'This local text fixture does not provide image upload.' }),
    chunk: async () => ({ _tag: 'Refused', reason: 'ungranted', detail: 'This local text fixture does not provide image reads.' }),
    send: async request => {
      if (readOnly || request.parameters.to !== agentRef || request.parameters.attachments.length !== 0)
        return { _tag: 'Refused', reason: 'ungranted', detail: 'This fixture permits local text messages to Worker tests only.' }
      const id = `message/local-${++next}`
      const previous = registry.get(conversationFeed)
      if (previous._tag !== 'Observed') return { _tag: 'Refused', reason: 'failed', detail: 'Local fixture conversation is not loaded.' }
      registry.set(conversationFeed, observed({ value: { ...previous.value, items: [...previous.value.items,
        ConversationItem.cases.Text.make({ id, role: 'user', text: request.parameters.content, attachments: [], streaming: false, at: new Date(now).toISOString() }),
      ] } }))
      return { _tag: 'Success', value: Native.decodeUnknownSync(ActionResult)({
        kind: 'action-result', action_id: request.id, operation_id: `operation/local-${next}`,
        snapshot_id: 'snapshot/local-story', affected_ids: [id], status: 'completed',
      }) }
    },
  }
  const answered = Atom.make<ReadonlySet<string>>(new Set<string>())
  const source = {
    ...base.source, threads, now: Atom.make(now),
    conversation: () => conversationFeed,
    grants: Atom.make({ actions: 'granted' as const, messageSend: readOnly ? 'ungranted' as const : 'granted' as const, attachmentUpload: 'ungranted' as const, terminalInput: 'ungranted' as const, terminalControl: 'ungranted' as const }),
    ...(readOnly ? {} : { attachments }),
    attention: Atom.make(get => observed({ value: [
      attention(sourceTitle, originSource, episode, get(summaries).find(row => row.id === existingId)?.unread ? true : undefined),
      attention('Confirm the release note', 'step-run/thread-story/release', 'claim/thread-story/release'),
    ].filter(row => !get(answered).has(row.id)) })),
    ...(readOnly ? {} : { alertActions: {
      act: async ({ alert }: { readonly alert: { readonly id: string } }) => {
        registry.set(answered, new Set([...registry.get(answered), alert.id]))
        return { _tag: 'Sent' as const }
      },
    } }),
  }
  return { agentRef, registry, source }
}

type Args = { readonly placement: 'inbox' | 'tab'; readonly connection: 'local' | 'read-only'; readonly scenario: Scenario; readonly ordering?: 'timestamp' | 'action' }

function ThreadWorkspace({ placement, connection, scenario, ordering }: Args) {
  const [fixture] = React.useState(() => createThreadStory(connection === 'read-only', scenario, ordering))
  const [navigation] = React.useState<WorkspaceNavigation>(() => {
    let location = `/w/${fixture.agentRef}${placement === 'tab' ? `?open=${encodeURIComponent((scenario === 'closed' ? closedId : existingId) + ':detail')}` : ''}`
    const listeners = new Set<() => void>()
    return {
      getLocation: () => location,
      subscribe: callback => { listeners.add(callback); return () => { listeners.delete(callback) } },
      navigate: href => { location = href; for (const notify of listeners) notify() },
    }
  })
  return <DataSourceProvider source={fixture.source} registry={fixture.registry}>
    <LiveAgentWorkspace navigation={navigation} persistGeometry={false} />
  </DataSourceProvider>
}

const meta = {
  title: 'Fractal/App/InboxThreads',
  excludeStories: ['scenarioExemptions'],
  component: ThreadWorkspace,
  args: { placement: 'inbox', connection: 'local', scenario: 'existing' },
  argTypes: { placement: { control: 'inline-radio' }, connection: { control: 'inline-radio' }, scenario: { control: 'inline-radio' } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Production workspace and thread pane with deterministic Inbox sources, exact quotes and local lifecycle actions. No daemon writes.' } },
  render: args => <ThreadWorkspace key={`${args.placement}:${args.connection}:${args.scenario}`} {...args} />,
} satisfies Meta<Args>
export default meta

export const scenarioExemptions = ['AllStates:contract-fixture', 'DedicatedTab:contract-fixture', 'ReadOnly:contract-fixture', 'EmptyStart:contract-fixture', 'ClosedThread:contract-fixture', 'MultipleThreads:contract-fixture', 'CustomInboxSource:contract-fixture', 'AgentThreads:contract-fixture', 'Loading:contract-fixture', 'Refused:contract-fixture', 'RetryAfterFailure:contract-fixture', 'UpdatesPaused:contract-fixture', 'UnconfirmedChange:contract-fixture', 'CloseConfirmation:contract-fixture', 'LatestPreview:contract-fixture'] as const
type Story = StoryObj<typeof meta>

export const AllStates: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const navigation = await waitFor(() => canvas.queryByRole('tab', { name: /^Alerts\b/ })
      ?? canvas.getByRole('button', { name: 'Expand sidebar' }), { timeout: 60_000 })
    if (navigation.getAttribute('role') !== 'tab') await userEvent.click(navigation)
    await userEvent.click(await canvas.findByRole('tab', { name: /^Alerts\b/ }, { timeout: 60_000 }))
    await userEvent.click(await canvas.findByRole('button', { name: /^(Review PR #42|Worker report ready),/ }, { timeout: 60_000 }))
  },
}
/** The Inbox detail that hosts the item's threads, once AllStates has opened it. */
const alertDetails = async (canvasElement: HTMLElement) => within(await within(canvasElement).findByTestId('alert-details', {}, { timeout: 60_000 }))
export const DedicatedTab: Story = { args: { placement: 'tab' } }
/** Without the reply grant the thread keeps its timeline and states why there is no composer. */
export const ReadOnly: Story = {
  args: { placement: 'tab', connection: 'read-only' },
  play: async ({ canvasElement }) => {
    const panel = await waitFor(() => {
      const element = canvasElement.querySelector<HTMLElement>('[data-thread-panel]')
      if (element === null) throw new Error('no thread panel yet')
      return element
    }, { timeout: 60_000 })
    await within(panel).findByText('Read-only on this device. Reply from a device with control access.')
    await expect(panel.querySelector('textarea')).toBeNull()
  },
}

export const EmptyStart: Story = {
  args: { scenario: 'empty' },
  play: async context => {
    await AllStates.play?.(context)
    const details = await alertDetails(context.canvasElement)
    await details.findByText('Ask Worker tests about this item without answering it.')
    await expect(details.getByRole('button', { name: 'Start thread' })).toBeTruthy()
  },
}
export const ClosedThread: Story = { args: { placement: 'tab', scenario: 'closed' } }
export const MultipleThreads: Story = { args: { scenario: 'multiple' }, play: AllStates.play }
export const CustomInboxSource: Story = {
  args: { scenario: 'custom-source' },
  play: async context => {
    await AllStates.play?.(context)
    const canvas = within(context.canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Start thread' }))
    const details = await alertDetails(context.canvasElement)
    await details.findByRole('textbox', { name: 'Message' })
    await expect(details.getByRole('textbox', { name: 'Thread title' })).toHaveValue('Worker report ready')
    await expect(details.getByText((_content, element) => element?.tagName === 'P' && element.textContent === 'To Worker tests')).toBeTruthy()
  },
}

/** The agent view pins one collapsed summary; a row opens the thread in its own tab instead of expanding in place. */
export const AgentThreads: Story = {
  play: async ({ canvasElement }) => {
    const strip = await waitFor(() => {
      const element = canvasElement.querySelector<HTMLDetailsElement>('details[aria-label="Threads"]')
      if (element === null) throw new Error('no thread strip yet')
      return element
    }, { timeout: 60_000 })
    await expect(strip.open).toBe(false)
    await expect(strip.querySelector('summary')?.textContent).toBe('Threads, 2')
    await userEvent.click(strip.querySelector('summary')!)
    await waitFor(() => expect(strip.open).toBe(true))
    const row = within(strip).getAllByRole('button', { name: /Worker tests/ })[0]!
    await expect(row.hasAttribute('aria-expanded')).toBe(false)
    await userEvent.click(row)
    await waitFor(() => expect(canvasElement.querySelector('[data-thread-origin]')?.textContent).toBe('With Worker tests'), { timeout: 10_000 })
    await expect(strip.querySelector('textarea')).toBeNull()
  },
}

/** Read states state only what the source reports; none of them invent agent progress. */
export const Loading: Story = {
  args: { scenario: 'loading' },
  play: async context => {
    await AllStates.play?.(context)
    const details = await alertDetails(context.canvasElement)
    await details.findByText('Loading threads')
    await expect(details.queryByRole('button', { name: 'Start thread' })).toBeNull()
  },
}
/** A refusal names the reason only: no Start beside a list Fractal refused, and nothing to retry. */
export const Refused: Story = {
  args: { scenario: 'refused' },
  play: async context => {
    await AllStates.play?.(context)
    const details = await alertDetails(context.canvasElement)
    await details.findByText(credentialExpiredMessage)
    await expect(details.queryByRole('button', { name: 'Start thread' })).toBeNull()
    await expect(details.queryByRole('button', { name: /^Retry/ })).toBeNull()
  },
}
export const RetryAfterFailure: Story = {
  args: { scenario: 'failed' },
  play: async context => {
    await AllStates.play?.(context)
    const details = await alertDetails(context.canvasElement)
    await details.findByText('Fractal could not load threads.')
    await userEvent.click(details.getByRole('button', { name: /^Retry/ }))
    await waitFor(() => expect(context.canvasElement.querySelector('[data-testid="alert-details"] [data-thread-panel][aria-label="Worker tests"]')).not.toBeNull())
    await expect(details.queryByText('Fractal could not load threads.')).toBeNull()
  },
}
/** One marker at the section heading; the rows below it carry none of their own. */
export const UpdatesPaused: Story = {
  args: { scenario: 'offline' },
  play: async context => {
    await AllStates.play?.(context)
    const details = await alertDetails(context.canvasElement)
    await expect(await details.findByRole('heading', { name: /^Threads, 2.*Updates paused$/ })).toBeTruthy()
    const markers = () => context.canvasElement.querySelectorAll('[data-testid="alert-details"] [data-threads-paused]')
    await expect(markers()).toHaveLength(1)
    await userEvent.click(context.canvasElement.querySelector<HTMLElement>('[data-testid="alert-details"] [data-thread-panel][aria-label="Worker tests"] button[aria-expanded]')!)
    await details.findByText('Worker tests pass on Linux. The macOS job is still running.')
    await expect(markers()).toHaveLength(1)
  },
}
export const UnconfirmedChange: Story = {
  args: { placement: 'tab', scenario: 'unconfirmed' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByText('Fractal has not confirmed the previous rename.', {}, { timeout: 60_000 })
    await userEvent.click(canvas.getByRole('button', { name: 'Retry rename' }))
    await waitFor(() => expect(canvas.queryByText('Fractal has not confirmed the previous rename.')).toBeNull())
  },
}
/** Close acts at once; the confirmed result names the thread, and the lifecycle button beside it now reads Reopen. */
export const CloseConfirmation: Story = {
  args: { placement: 'tab' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Close thread' }))
    const confirmed = await waitFor(() => {
      const status = canvasElement.querySelector('[data-thread-confirmed]')
      if (status === null) throw new Error('no confirmation yet')
      return status as HTMLElement
    })
    await expect(confirmed.textContent).toMatch(/^Closed “.+”\.$/)
    await userEvent.click(canvas.getByRole('button', { name: 'Reopen thread' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-thread-confirmed]')?.textContent).toMatch(/^Reopened “.+”\.$/))
    await expect(await canvas.findByRole('button', { name: 'Close thread' })).toBeTruthy()
  },
}

/** Local review controls inject fixtures into the production workspace. */
function LatestPreviewHarness() {
  const [placement, setPlacement] = React.useState<'inbox' | 'tab'>('inbox')
  const [scenario, setScenario] = React.useState<Scenario>('custom-source')
  const [ordering, setOrdering] = React.useState<'timestamp' | 'action'>('timestamp')
  const segment = { display: 'flex', gap: 4, alignItems: 'center' } as const
  const radio = { padding: '6px 12px', border: '1px solid currentColor', borderRadius: 5, cursor: 'pointer' } as const
  const radioStyle = ({ isSelected, isFocusVisible }: { isSelected: boolean; isFocusVisible: boolean }) => ({ ...radio, background: isSelected ? '#303b4a' : 'transparent', color: isSelected ? '#fff' : 'inherit', outline: isFocusVisible ? '2px solid #609cff' : undefined })
  return <div data-latest-thread-preview="" style={{ height: '100dvh', display: 'grid', gridTemplateRows: 'auto auto minmax(0, 1fr)', minHeight: 0, overflow: 'hidden', background: '#101317', color: '#e7edf5', fontFamily: 'system-ui, sans-serif' }}>
    <style>{'[data-latest-thread-preview] [data-testid="live-agent-workspace"] { height: 100% !important; min-height: 0; }'}</style>
    <div style={{ padding: 12, display: 'flex', gap: 16, flexWrap: 'wrap' }}>
      <RadioGroup aria-label="Preview surface" orientation="horizontal" value={scenario === 'clock-skew' ? 'tab' : placement} onChange={value => { setPlacement(value as typeof placement); if (value === 'tab' && scenario === 'custom-source') setScenario('existing') }} style={segment}>
        <Radio isDisabled={scenario === 'clock-skew'} value="inbox" style={radioStyle}>Inbox</Radio><Radio value="tab" style={radioStyle}>Thread tab</Radio>
      </RadioGroup>
      <RadioGroup aria-label="Preview scenario" orientation="horizontal" value={scenario} onChange={value => { setScenario(value as Scenario); if (value === 'custom-source') setPlacement('inbox') }} style={segment}>
        <Radio value="custom-source" style={radioStyle}>Inbox source</Radio><Radio value="existing" style={radioStyle}>Conversation</Radio><Radio value="clock-skew" style={radioStyle}>Close → Reopen with clock skew</Radio>
      </RadioGroup>
      {scenario !== 'clock-skew' ? null : <RadioGroup aria-label="Fixture ordering" orientation="horizontal" value={ordering} onChange={value => setOrdering(value as typeof ordering)} style={segment}>
        <Radio value="timestamp" style={radioStyle}>Timestamp</Radio><Radio value="action" style={radioStyle}>Action order</Radio>
      </RadioGroup>}
    </div>
    <p role="status" style={{ margin: '0 12px 12px' }}>{scenario === 'clock-skew'
      ? `Fixture comparison: Close at 12:59, then Reopen at 12:58 (clock skew). ${ordering === 'timestamp' ? 'Timestamp order shows Reopen → Close; authoritative state Open.' : 'Action order shows Close → Reopen; authoritative state Open.'} Only timeline presentation differs; the later Reopen keeps the thread open in both fixtures. Production ordering is unchanged.`
      : 'Local fixtures in the real Fractal workspace. Conversation and thread text sends update local fixture state only; no daemon connection or device pairing is represented.'}</p>
    <div style={{ minHeight: 0, minWidth: 0, overflow: 'hidden', position: 'relative' }}>
      <ThreadWorkspace key={`${placement}:${scenario}:${ordering}`} placement={scenario === 'clock-skew' ? 'tab' : placement} connection="local" scenario={scenario} ordering={ordering} />
    </div>
  </div>
}
export const LatestPreview: Story = { render: () => <LatestPreviewHarness />, play: AllStates.play }
