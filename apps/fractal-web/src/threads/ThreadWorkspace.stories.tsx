import type { Meta, StoryObj } from '@storybook/react-vite'
import * as Native from '@fractal/client/schema/read'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { userEvent, within } from 'storybook/test'
import { DataSourceProvider } from '../data/react.tsx'
import { observed, unavailable } from '../data/source.ts'
import { createAppStoryFixture } from '../fixtures/appStoryStates.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveAgentWorkspace, type WorkspaceNavigation } from '../web/LiveAgentWorkspace.tsx'
import type { ThreadDraft, ThreadSource } from './threadSource.ts'

const review = 'Review PR #42'
const existing = 'Worker tests'
const askId = 'step-run/thread-story/review'
const episode = 'claim/thread-story/review'
const existingId = 'thread/worker-tests'
const closedId = 'thread/rollout-notes'
const actor = 'person/operator'

function createThreadStory(readOnly: boolean) {
  const base = createAppStoryFixture('populated')
  const { agentRef } = base
  const registry = AtomRegistry.make()
  const now = fixtureProjections.now
  const attention = (name: string, subject: string, claim: string, unread?: true) => Native.decodeUnknownSync(Native.Attention)({
    id: `attention/${claim.split('/').at(-1)}`, kind: 'attention', revision: claim, episode: claim,
    person_id: actor, source_id: subject, requester_id: agentRef, attention_kind: 'agent-request',
    title: name, detail: 'Can we proceed with this change?', priority: 'high', state: 'open', alert: true,
    actions: ['work.done'], requested_at: new Date(now - 25 * 60_000).toISOString(), updated_at: new Date(now).toISOString(),
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
  const summaries = Atom.make<readonly Native.ThreadSummary[]>([
    summary(existingId, existing, 'open', 3, true), summary(closedId, 'Rollout notes', 'closed', 1, false),
  ])
  const history = Atom.make<Readonly<Record<string, readonly Native.ThreadTimelineEvent[]>>>({
    [existingId]: [
      message('message/review-ready', agentRef, -2, 'Worker tests pass on Linux. The macOS job is still running.'),
      message('message/review-scope', actor, -9, 'Can you verify the cancellation case too?'),
      message('message/review-first', agentRef, -24, 'The retry fix is ready. I will verify the worker tests before merge.'),
    ],
    [closedId]: [message('message/rollout-notes', actor, -40, 'Keep the original rollout plan in the release notes.')],
  })
  const origins = Atom.make<Readonly<Record<string, string>>>({ [existingId]: askId })
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
    update(id, row => ({ ...row, message_count: previous.length + 1 }))
  }
  const threads: ThreadSource = {
    access: Atom.make({ read: true, actor, actions: new Set<string>(readOnly
      ? [] : ['thread.start', 'thread.reply', 'thread.rename', 'thread.close', 'thread.reopen', 'thread.read']) }),
    titles: Atom.make(get => new Map(get(summaries).map(row => [row.id, row.title]))),
    list: query => Atom.make(get => observed({ value: {
      items: get(summaries).filter(row => 'recently_closed' in query ? row.state === 'closed'
        : 'origin_ask' in query ? get(origins)[row.id] === query.origin_ask : true),
      next_cursor: Option.none(),
    } })),
    detail: id => Atom.make(get => {
      const row = get(summaries).find(candidate => candidate.id === id)
      return row === undefined ? unavailable({ reason: 'failed', detail: 'Fixture thread unavailable' }) : observed({ value: { summary: row, snapshotId: 'snapshot/thread-story' } })
    }),
    timeline: id => Atom.make(get => observed({ value: { items: get(history)[id] ?? [], next_cursor: Option.none() } })),
    draft: drafts,
    start: async (_draft, input) => {
      const id = `thread/story-${++next}`
      registry.set(summaries, [...registry.get(summaries), summary(id, input.title, 'open', 0, false)])
      registry.set(origins, { ...registry.get(origins), [id]: input.origin_ask })
      append(id, input.content)
      return { _tag: 'Sent', threadId: id }
    },
    reply: async input => { append(input.target_id, input.content, input.thread_quote_id); return { _tag: 'Sent', threadId: input.target_id } },
    rename: async (id, title) => { update(id.summary.id, row => ({ ...row, title })); return undefined },
    changeState: async (id, state) => { update(id.summary.id, row => ({ ...row, state })); return undefined },
    markRead: async (id, _through, seen) => {
      update(id.summary.id, row => ({ ...row, unread: !seen.includes('message/review-ready') }))
      return undefined
    },
  }
  const answered = Atom.make<ReadonlySet<string>>(new Set<string>())
  const source = {
    ...base.source, threads,
    attention: Atom.make(get => observed({ value: [
      attention(review, askId, episode, get(summaries).find(row => row.id === existingId)?.unread ? true : undefined),
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

type Args = { readonly placement: 'inbox' | 'tab'; readonly connection: 'local' | 'read-only' }

function ThreadWorkspace({ placement, connection }: Args) {
  const [fixture] = React.useState(() => createThreadStory(connection === 'read-only'))
  const [navigation] = React.useState<WorkspaceNavigation>(() => {
    let location = `/w/${fixture.agentRef}${placement === 'tab' ? `?open=${encodeURIComponent(existingId + ':detail')}` : ''}`
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
  args: { placement: 'inbox', connection: 'local' },
  argTypes: { placement: { control: 'inline-radio' }, connection: { control: 'inline-radio' } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Production workspace and thread pane with deterministic Inbox requests, exact quotes and local lifecycle actions. No daemon writes.' } },
  render: args => <ThreadWorkspace key={`${args.placement}:${args.connection}`} {...args} />,
} satisfies Meta<Args>
export default meta

export const scenarioExemptions = ['AllStates:contract-fixture', 'DedicatedTab:contract-fixture', 'ReadOnly:contract-fixture'] as const
type Story = StoryObj<typeof meta>

export const AllStates: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: /^Alerts\b/ }, { timeout: 60_000 }))
    await userEvent.click(await canvas.findByRole('button', { name: /^Review PR #42,/ }))
  },
}
export const DedicatedTab: Story = { args: { placement: 'tab' } }
export const ReadOnly: Story = { args: { placement: 'tab', connection: 'read-only' } }
