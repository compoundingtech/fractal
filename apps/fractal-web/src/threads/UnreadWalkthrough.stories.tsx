/**
 * One controlled world through the production workspace: the real LiveAgentWorkspace routes, LiveThreadPane's
 * unread visit controller (stable 500 ms foreground visibility), the owner-wide MarkedUnreadThreads list and the
 * real gateway unread port on a real FractalClient. Only what lies underneath is synthetic: a local thread source
 * and an in-memory fetch that serves the client's capabilities and around-reads from fixture atoms, so nothing
 * reaches a daemon or an authenticated API.
 */
import { FractalClient, type ThreadListQuery } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { useAtomValue } from '@effect/atom-react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { Button, Label, Radio, RadioGroup } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { ConversationItem } from '../conversation/model.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { observed, unavailable, type DataSource } from '../data/source.ts'
import recording from '../data/subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { fixtureProjections } from '../fixtures/projections.ts'
import { fixtureSidebarState } from '../shell/sidebar/state.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { LiveAgentWorkspace, type WorkspaceNavigation } from '../web/LiveAgentWorkspace.tsx'
import { workspaceHref } from '../web/workspaceView.ts'
import { threadAnchorAttribute } from './threadAnchor.ts'
import type { ThreadDraft, ThreadSource } from './threadSource.ts'
import { gatewayUnreadActions } from './unread-action-port.ts'

const legend = 'Demo data; nothing is sent to Fractal'
const agentRef = 'agent/build-host-a/release-worker'
const actor = 'person/operator'
const now = Date.parse('2026-11-16T12:00:00Z')
/** Cap the workspace for recordings, but let its real narrow layout fit smaller canvases. */
const stage = { width: 1200, height: 660 } as const
/** Fixture latency, so a recording sees each read and write in flight; plays also use it to outlast the 500 ms dwell. */
const settle = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms))
/** The recorded synthetic capabilities envelope, with the thread grants the unread port reads (as UnreadGateway's test serves). */
const capabilities = { ...recording.capabilities, value: { ...recording.capabilities.value, transport: 'local-web', capabilities: [
  ...recording.capabilities.value.capabilities,
  { id: 'threads-v1', version: 3, state: 'granted' },
  { id: 'thread-summary-feed', version: 1, state: 'granted' },
  { id: 'thread.mark-unread', version: 0, state: 'granted' },
  { id: 'thread.clear-unread', version: 0, state: 'granted' },
] } }

const release = 'thread/release-checklist'
const rollout = 'thread/rollout-notes'
const cancellation = 'thread/cancellation'
const migration = 'thread/migration-notes'

type Scenario = 'revisit' | 'closed' | 'around'
type Paging = 'single' | 'empty-first'
type Mark = { readonly mark_id: string; readonly anchor_message_id: string }
/** Canonical encoded timeline events, newest first, as Fractal serves them. */
type RawEvent =
  | { readonly kind: 'message'; readonly id: string; readonly actor_id: string; readonly accepted_at_unix_ms: number; readonly content: string; readonly thread_quote_id: string | null }
  | { readonly kind: 'thread.closed' | 'thread.reopened'; readonly id: string; readonly actor_id: string; readonly accepted_at_unix_ms: number }
  | { readonly kind: 'thread.renamed'; readonly id: string; readonly actor_id: string; readonly accepted_at_unix_ms: number; readonly previous_title: string; readonly title: string }
/** One fixture thread. `marks` is the summary's unread reminder: the only place mark IDs come from. */
type Row = { readonly id: string; readonly title: string; readonly state: 'open' | 'closed'; readonly titleRevision: number; readonly marks: readonly Mark[] }
type LogEntry = { readonly id: number; readonly kind: 'seek' | 'clear' | 'mark' | 'fixture'; readonly text: string; readonly details: string }

const message = (id: string, author: string, minutes: number, content: string): RawEvent =>
  ({ kind: 'message', id, actor_id: author, accepted_at_unix_ms: now + minutes * 60_000, content, thread_quote_id: null })
const mark = (mark_id: string, anchor_message_id: string): Mark => ({ mark_id, anchor_message_id })

/** Each scenario's world. Mark IDs are opaque fixture strings, never derived from operations or clocks. */
const seed = (scenario: Scenario) => {
  const row = (id: string, title: string, state: Row['state'], marks: readonly Mark[] = []): Row => ({ id, title, state, titleRevision: 1, marks })
  const history: Record<string, readonly RawEvent[]> = {
    [release]: [
      message('message/checklist-ready', agentRef, -6, 'Rollback rehearsal added. The canary is ready when you are.'),
      message('message/checklist-ask', actor, -22, 'Add a rollback rehearsal before the canary.'),
      message('message/checklist-draft', agentRef, -30, 'Release checklist drafted: schema migration, canary, rollback.'),
    ],
    [rollout]: [
      { kind: 'thread.closed', id: 'event/rollout-closed', actor_id: actor, accepted_at_unix_ms: now - 45 * 60_000 },
      message('message/rollout-done', agentRef, -50, 'Rollout finished in every region. The notes are in the release doc.'),
    ],
    [cancellation]: [message('message/cancellation-fenced', agentRef, -40, 'Cancellation keeps the original attempt fenced.')],
    [migration]: [message('message/migration-dry-run', agentRef, -60, 'The migration dry run finished with no schema drift.')],
  }
  switch (scenario) {
    case 'revisit': return {
      primary: release, history, pending: {},
      rows: [row(release, 'Release checklist', 'open', [mark('synthetic-mark/release-a', 'message/checklist-ready')]), row(rollout, 'Rollout notes', 'closed')],
    }
    case 'closed': return {
      primary: rollout, history, pending: {},
      rows: [row(release, 'Release checklist', 'open'), row(rollout, 'Rollout notes', 'closed', [mark('synthetic-mark/rollout-a', 'message/rollout-done')])],
    }
    case 'around': return {
      primary: release, history,
      // The pending reminder's reply exists only as a signed intent until "Deliver the pending reply".
      pending: { [cancellation]: message('message/cancellation-retry', agentRef, -1, 'Retry coverage is in: a cancelled attempt never runs again.') } as Record<string, RawEvent>,
      rows: [
        row(release, 'Release checklist', 'open', [mark('synthetic-mark/release-a', 'message/checklist-ready')]),
        row(cancellation, 'Cancellation behavior', 'open', [mark('synthetic-mark/cancellation-a', 'message/cancellation-retry')]),
        // Its reminded reply was withdrawn, so seeking around it answers unavailable.
        row(migration, 'Migration notes', 'open', [mark('synthetic-mark/migration-a', 'message/migration-superseded')]),
      ],
    }
  }
}

const agentReplies = ['The canary is at 10% with no errors.', 'The canary is at 50%; latency is flat.', 'The canary is complete. Ready to promote.']

/** One scenario's controlled world: its own registry, data source and in-app navigation over the fixture atoms. */
interface World {
  readonly scenario: Scenario
  /** The thread a scenario walks through first. */
  readonly primary: string
  readonly registry: AtomRegistry.AtomRegistry
  readonly source: DataSource
  readonly navigation: WorkspaceNavigation
  readonly rows: Atom.Writable<readonly Row[]>
  readonly pending: Atom.Writable<Readonly<Record<string, RawEvent>>>
  readonly paging: Atom.Writable<Paging>
  readonly log: Atom.Writable<readonly LogEntry[]>
  readonly updates: {
    readonly markElsewhere: (id: string) => void
    readonly agentReply: (id: string) => void
    readonly deliverPending: () => void
  }
  /** The thread the Thread tab route returns to; follows the last thread the in-app route showed. */
  lastThread: string
}

function createWorld(scenario: Scenario, initialPaging: Paging): World {
  const registry = AtomRegistry.make()
  const world = seed(scenario)
  const rows = Atom.keepAlive(Atom.make<readonly Row[]>(world.rows))
  const history = Atom.keepAlive(Atom.make<Readonly<Record<string, readonly RawEvent[]>>>(world.history))
  const pending = Atom.keepAlive(Atom.make<Readonly<Record<string, RawEvent>>>(world.pending))
  const sought = Atom.keepAlive(Atom.make<Readonly<Record<string, string>>>({}))
  const paging = Atom.keepAlive(Atom.make<Paging>(initialPaging))
  const log = Atom.keepAlive(Atom.make<readonly LogEntry[]>([]))
  let sequence = 0, external = 0, replies = 0, marks = 0
  const record = (kind: LogEntry['kind'], text: string, details: string) => registry.set(log, [...registry.get(log), { id: ++sequence, kind, text, details }])
  const titleOf = (id: string) => registry.get(rows).find(row => row.id === id)?.title ?? id
  const edit = (id: string, change: (row: Row) => Row) => registry.set(rows, registry.get(rows).map(row => row.id === id ? change(row) : row))
  const append = (id: string, event: RawEvent) => registry.set(history, { ...registry.get(history), [id]: [event, ...(registry.get(history)[id] ?? [])] })

  const summaries = Atom.make(get => get(rows).map(row => {
    const events = get(history)[row.id] ?? []
    return Native.decodeUnknownSync(Native.ThreadSummary)({
      id: row.id, title: row.title, state: row.state, agent_id: agentRef,
      title_revision: `claim/${row.id}/title-${row.titleRevision}`, revision: `claim/${row.id}/${events.length}`,
      message_count: events.filter(event => event.kind === 'message').length, unread: false,
      unread_reminder: row.marks.length === 0 ? null : { anchor_message_id: row.marks.at(-1)!.anchor_message_id, marks: row.marks },
    })
  }))
  const access = Atom.make({ read: true, actor, actions: new Set<string>(['thread.reply', 'thread.rename', 'thread.close', 'thread.reopen', 'thread.mark-unread', 'thread.clear-unread']) })
  const detail = Atom.family((id: string) => Atom.make(get => {
    const summary = get(summaries).find(row => row.id === id)
    return summary === undefined ? unavailable({ reason: 'failed', detail: 'Fixture thread unavailable' })
      : observed({ value: { summary, snapshotId: 'snapshot/unread-walkthrough' } })
  }))
  const timeline = Atom.family((id: string) => Atom.make(get => {
    const items = get(history)[id] ?? []
    const found = get(sought)[id]
    const anchor = found !== undefined && items.some(item => item.id === found) ? { message_id: found, state: 'current' as const } : undefined
    return observed({ value: Native.decodeUnknownSync(Native.ThreadTimelinePage)({ items, next_cursor: null, ...(anchor === undefined ? {} : { anchor }) }) })
  }))
  const list = Atom.family((key: string) => Atom.make(get => {
    const query = JSON.parse(key) as ThreadListQuery
    const all = get(summaries)
    const items = 'marked_unread' in query ? all.filter(row => Option.isSome(row.unread_reminder))
      : 'recently_closed' in query ? all.filter(row => row.state === 'closed')
      : 'agent_id' in query ? all.filter(row => row.agent_id === query.agent_id) : []
    // An empty first page that names a cursor is not an empty list; the reader must follow it.
    if ('marked_unread' in query && get(paging) === 'empty-first' && query.cursor === undefined) {
      return observed({ value: { items: [], next_cursor: Option.some('cursor/marked-unread-2') } })
    }
    return observed({ value: { items, next_cursor: Option.none<string>() } })
  }))

  /**
   * The real client over an in-memory fetch. It serves only what the gateway unread port reads: the capabilities
   * envelope (the client's page limits) and the around-read that seeks a reminded reply. Anything else is refused.
   */
  const client = new FractalClient({ baseUrl: 'http://synthetic-gateway.test', fetchImpl: async input => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return Response.json(capabilities)
    const thread = /\/threads\/([a-zA-Z0-9_-]+)\/timeline$/.exec(url.pathname)?.[1]
    const around = url.searchParams.get('around')
    if (thread === undefined || around === null) throw new Error(`The walkthrough serves no ${url.pathname}`)
    await settle(300)
    const threadId = `thread/${thread}`
    const items = registry.get(history)[threadId] ?? []
    const state = items.some(item => item.id === around) ? 'current' : registry.get(pending)[threadId]?.id === around ? 'pending' : 'unavailable'
    record('seek', state === 'current' ? `Opened the reminded message in “${titleOf(threadId)}”` : state === 'pending' ? `The reminded message in “${titleOf(threadId)}” has not arrived yet` : `The reminded message in “${titleOf(threadId)}” is no longer available`, `seek around ${around} → ${state}`)
    // The wire page omits previous_cursor: a null there is not the client's optional string.
    return Response.json({ ...capabilities, value: { items, next_cursor: null, anchor: { message_id: around, state } } })
  } })
  const unreadActions = gatewayUnreadActions({
    client, registry, access, detail,
    onFound: (threadId, page) => { if (page.anchor !== undefined) registry.set(sought, { ...registry.get(sought), [threadId]: page.anchor.message_id }) },
    send: async (thread, change) => {
      await settle(150)
      const id = thread.summary.id
      // Observed-remove: only the exact mark IDs this view read from the summary leave the reminder.
      if (change.kind === 'thread.clear-unread') {
        edit(id, row => ({ ...row, marks: row.marks.filter(each => !change.observed_marks.includes(each.mark_id)) }))
        record('clear', `Reminder cleared after viewing “${titleOf(id)}”`, `clear ${change.observed_marks.join(', ')} → Sent`)
      } else {
        const next = mark(`synthetic-mark/${id.slice('thread/'.length)}-mark-${++marks}`, change.anchor_message_id)
        edit(id, row => ({ ...row, marks: [...row.marks.filter(each => !change.observed_marks.includes(each.mark_id)), next] }))
        record('mark', `Marked “${titleOf(id)}” unread`, `mark unread from ${change.anchor_message_id} → Sent`)
      }
      return { _tag: 'Sent' }
    },
  })

  const threads: ThreadSource = {
    access,
    titles: Atom.make(get => new Map(get(summaries).map(row => [row.id, row.title]))),
    list: query => list(JSON.stringify(query)),
    detail,
    timeline: id => timeline(id),
    draft: Atom.family((_id: string) => Atom.keepAlive(Atom.make<ThreadDraft>({ content: '' }))),
    start: async () => ({ _tag: 'Refused', detail: "Couldn't start this thread.", explanation: 'This walkthrough does not start threads.' }),
    reply: async input => {
      append(input.target_id, message(`message/walkthrough-${++sequence}`, actor, 0, input.content))
      return { _tag: 'Sent', threadId: input.target_id }
    },
    rename: async (observation, title) => {
      const id = observation.summary.id
      append(id, { kind: 'thread.renamed', id: `event/walkthrough-${++sequence}`, actor_id: actor, accepted_at_unix_ms: now, previous_title: observation.summary.title, title })
      edit(id, row => ({ ...row, title, titleRevision: row.titleRevision + 1 }))
      return undefined
    },
    changeState: async (observation, state) => {
      const id = observation.summary.id
      append(id, { kind: state === 'closed' ? 'thread.closed' : 'thread.reopened', id: `event/walkthrough-${++sequence}`, actor_id: actor, accepted_at_unix_ms: now })
      edit(id, row => ({ ...row, state }))
      return undefined
    },
    unreadActions,
  }

  const agent = Native.decodeUnknownSync(Native.Agent)({
    kind: 'agent', id: agentRef, name: 'Release worker', host_id: 'host/build-host-a', driver: 'omp',
    state: 'waiting', blocked_on: 'human', fault: null, reason: 'Canary ready; waiting for go-ahead.',
    runtime_ids: [], reachability: 'reachable', revision: '1', updated_at: new Date(now).toISOString(),
  })
  const conversation = [
    ConversationItem.cases.Text.make({ id: 'conversation/release-request', role: 'user', text: 'Prepare the release checklist and keep follow-ups in threads.', attachments: [], streaming: false, at: new Date(now - 35 * 60_000).toISOString() }),
    ConversationItem.cases.Text.make({ id: 'conversation/release-report', role: 'assistant', text: 'The checklist and rollout notes are in their threads below.', attachments: [], streaming: false, at: new Date(now - 5 * 60_000).toISOString() }),
  ]
  const base = fixtureSource({ world: {
    ...fixtureProjections, now, agents: [agent], missions: [], attention: [], events: [],
    conversations: { [agentRef]: { items: conversation, hasOlder: false } }, terminals: {}, envelopes: {}, proposed: {},
  } })
  const conversationFeed = Atom.make(observed({ value: { items: conversation, hasOlder: false } }))
  const source = {
    ...base, threads, now: Atom.make(now),
    conversation: () => conversationFeed,
    grants: Atom.make({ actions: 'granted' as const, messageSend: 'ungranted' as const, attachmentUpload: 'ungranted' as const, terminalInput: 'ungranted' as const, terminalControl: 'ungranted' as const }),
  }
  // The sidebar opens on the Inbox, whose footer holds the owner-wide Marked unread list.
  registry.set(fixtureSidebarState(base.gateway ?? base.mode).tab, 'alerts')

  let location = workspaceHref(agentRef, 'thread', '')
  const listeners = new Set<() => void>()
  const navigation: WorkspaceNavigation = {
    getLocation: () => location,
    subscribe: callback => { listeners.add(callback); return () => { listeners.delete(callback) } },
    navigate: next => { location = next; for (const notify of listeners) notify() },
  }

  /** Fixture-side updates: what another device, the agent or Fractal's delivery would change underneath the UI. */
  const updates = {
    markElsewhere: (id: string) => {
      const anchor = (registry.get(history)[id] ?? []).find(event => event.kind === 'message' && event.actor_id === agentRef)
      if (anchor === undefined) return
      edit(id, row => ({ ...row, marks: [...row.marks, mark(`synthetic-mark/${id.slice('thread/'.length)}-device-${++external}`, anchor.id)] }))
      record('fixture', `Another device marked “${titleOf(id)}” unread`, `external mark from ${anchor.id}`)
    },
    agentReply: (id: string) => {
      const content = agentReplies[replies % agentReplies.length]!
      append(id, message(`message/walkthrough-agent-${++replies}`, agentRef, 0, content))
      record('fixture', `The agent replied in “${titleOf(id)}”`, `append agent reply to ${id}`)
    },
    deliverPending: () => {
      const waiting = registry.get(pending)[cancellation]
      if (waiting === undefined) return
      append(cancellation, waiting)
      registry.set(pending, {})
      record('fixture', `The pending reply arrived in “${titleOf(cancellation)}”`, `deliver ${waiting.id}`)
    },
  }
  return { scenario, primary: world.primary, registry, source, navigation, rows, pending, paging, log, updates, lastThread: world.primary }
}

const scenarioCopy: Readonly<Record<Scenario, { readonly label: string; readonly guide: string }>> = {
  revisit: {
    label: 'Reminder & revisit',
    guide: 'Open Release checklist from Marked unread. Its saved message appears. After viewing it, the reminder clears. Mark it unread again: it stays during this visit. Switch to Agent conversation and come back to see it clear.',
  },
  closed: {
    label: 'Closed thread',
    guide: 'Closed threads keep their reminders in the owner-wide list. Open “Rollout notes”: the thread stays closed and its reminder clears once the reply is seen.',
  },
  around: {
    label: 'Around: current / pending / unavailable',
    guide: 'Opening a row first reads around its reminded reply, then routes. Pending and unavailable replies stay in the list with the reason. Deliver the pending reply, then open “Cancellation behavior”.',
  },
}

function Controls({ world, threadRoute, paging, onScenario, onPaging }: {
  readonly world: World
  /** The thread the in-app route shows in its own tab, or undefined in the agent context. */
  readonly threadRoute: string | undefined
  readonly paging: Paging
  readonly onScenario: (next: Scenario) => void
  readonly onPaging: (next: Paging) => void
}) {
  const scenario = world.scenario
  React.useEffect(() => { if (threadRoute !== undefined) world.lastThread = threadRoute }, [world, threadRoute])
  const rows = useAtomValue(world.rows)
  const pending = useAtomValue(world.pending)
  const log = useAtomValue(world.log)
  const foreground = useForeground()
  const target = threadRoute ?? world.primary
  const titleOf = (id: string) => rows.find(row => row.id === id)?.title ?? id
  const reminded = rows.filter(row => row.marks.length > 0)
  const segment = ({ isSelected, isFocusVisible }: { readonly isSelected: boolean; readonly isFocusVisible: boolean }) =>
    stylex.props(styles.segment, isSelected && styles.segmentSelected, isFocusVisible && styles.focus).className ?? ''
  const go = (route: 'agent' | 'thread') => world.navigation.navigate(workspaceHref(agentRef, route === 'agent' ? 'thread' : `${world.lastThread}:detail`, ''))
  return <header data-unread-walkthrough-controls="" {...stylex.props(styles.controls)}>
    <div {...stylex.props(styles.line)}>
      <p data-walkthrough-legend="" {...stylex.props(styles.legend)}>{legend}</p>
      <p {...stylex.props(styles.note)}>Try the real thread interface with sample conversations. Changes stay in this preview.</p>
    </div>
    <div {...stylex.props(styles.line)}>
      <RadioGroup orientation="horizontal" value={scenario} onChange={value => onScenario(value as Scenario)} {...stylex.props(styles.group)}>
        <Label {...stylex.props(styles.groupLabel)}>Scenario</Label>
        {(Object.keys(scenarioCopy) as Scenario[]).map(key => <Radio key={key} value={key} className={segment}>{scenarioCopy[key].label}</Radio>)}
      </RadioGroup>
      <RadioGroup orientation="horizontal" value={paging} onChange={value => onPaging(value as Paging)} {...stylex.props(styles.group)}>
        <Label {...stylex.props(styles.groupLabel)}>List paging</Label>
        <Radio value="single" className={segment}>One page</Radio>
        <Radio value="empty-first" className={segment}>Empty first page + cursor</Radio>
      </RadioGroup>
    </div>
    <div {...stylex.props(styles.line)}>
      <RadioGroup orientation="horizontal" value={threadRoute === undefined ? 'agent' : 'thread'} onChange={value => go(value as 'agent' | 'thread')} {...stylex.props(styles.group)}>
        <Label {...stylex.props(styles.groupLabel)}>View</Label>
        <Radio value="agent" className={segment}>Agent conversation</Radio>
        <Radio value="thread" className={segment}>Thread tab</Radio>
      </RadioGroup>
      <div role="group" aria-label="Simulate an update" {...stylex.props(styles.group)}>
        <span {...stylex.props(styles.groupLabel)}>Simulate an update · “{titleOf(target)}”</span>
        <Button onPress={() => world.updates.markElsewhere(target)} {...stylex.props(styles.action)}>Mark unread on another device</Button>
        <Button onPress={() => world.updates.agentReply(target)} {...stylex.props(styles.action)}>Agent posts a reply</Button>
        {scenario !== 'around' || pending[cancellation] === undefined ? null
          : <Button onPress={world.updates.deliverPending} {...stylex.props(styles.action)}>Deliver the pending reply</Button>}
      </div>
    </div>
    <p {...stylex.props(styles.guide)}>{scenarioCopy[scenario].guide}</p>
    <div {...stylex.props(styles.status)}>
      <dl {...stylex.props(styles.facts)}>
        <dt {...stylex.props(styles.term)}>View</dt>
        <dd data-walkthrough-route="" {...stylex.props(styles.value)}>{threadRoute === undefined ? 'Agent conversation · Release worker' : `Thread · ${titleOf(threadRoute)}`}</dd>
        <dt {...stylex.props(styles.term)}>Foreground</dt>
        <dd {...stylex.props(styles.value)}>{foreground ? 'Page focused and visible: a reminded reply in view for 500 ms clears.' : 'Page not focused: click inside the canvas. Reminders only clear while it has focus.'}</dd>
        <dt {...stylex.props(styles.term)}>Reminders</dt>
        <dd data-walkthrough-reminders="" {...stylex.props(styles.value)}>{reminded.length === 0 ? 'None' : reminded.map(row => `“${row.title}”`).join(' · ')}</dd>
      </dl>
      <section aria-label="Actions in this demo">
        <p {...stylex.props(styles.note)}>Actions in this demo</p>
        <ol data-walkthrough-log="" aria-label="What happened" {...stylex.props(styles.log)}>
          {log.length === 0 ? <li {...stylex.props(styles.logEntry)}>No changes yet.</li>
            : log.slice(-5).toReversed().map(entry => <li key={entry.id} data-walkthrough-log-kind={entry.kind} {...stylex.props(styles.logEntry)}>{entry.text}</li>)}
        </ol>
        <details>
          <summary {...stylex.props(styles.note)}>Technical details</summary>
          <ol {...stylex.props(styles.log)}>
            {log.map(entry => <li key={entry.id} {...stylex.props(styles.logEntry)}>{entry.id}. {entry.details}</li>)}
          </ol>
        </details>
      </section>
    </div>
  </header>
}

/** Mirrors the controller's own foreground test so a recording can see why a reminder is not clearing yet. */
function useForeground() {
  const [foreground, setForeground] = React.useState(false)
  React.useEffect(() => {
    const read = () => setForeground(document.visibilityState === 'visible' && document.hasFocus())
    read()
    window.addEventListener('focus', read)
    window.addEventListener('blur', read)
    document.addEventListener('visibilitychange', read)
    return () => {
      window.removeEventListener('focus', read)
      window.removeEventListener('blur', read)
      document.removeEventListener('visibilitychange', read)
    }
  }, [])
  return foreground
}

function Walkthrough() {
  const [paging, setPaging] = React.useState<Paging>('single')
  const [world, setWorld] = React.useState(() => createWorld('revisit', 'single'))
  const [stageNode, setStageNode] = React.useState<HTMLDivElement | null>(null)
  const [stageWidth, setStageWidth] = React.useState<number>(stage.width)
  React.useLayoutEffect(() => {
    if (stageNode === null) return
    const measure = () => setStageWidth(stageNode.clientWidth)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(stageNode)
    return () => observer.disconnect()
  }, [stageNode])
  // Each world owns its registry: dispose the replaced one after its workspace unmounts, and the last on unmount.
  React.useEffect(() => () => world.registry.dispose(), [world])
  const location = React.useSyncExternalStore(world.navigation.subscribe, world.navigation.getLocation, world.navigation.getLocation)
  const open = new URLSearchParams(location.split('?')[1]).get('open') ?? 'thread'
  const threadRoute = /^(thread\/[a-zA-Z0-9_-]+):detail$/.exec(open)?.[1]
  const log = React.useSyncExternalStore(
    React.useCallback((notify: () => void) => world.registry.subscribe(world.log, notify), [world]),
    React.useCallback(() => world.registry.get(world.log), [world]),
    React.useCallback(() => world.registry.get(world.log), [world]),
  )
  // A new world is a new registry and navigation: the provider and production workspace mount afresh for it.
  return <DataSourceProvider key={world.scenario} source={world.source} registry={world.registry}>
    <main data-unread-walkthrough="" data-scenario={world.scenario} data-route={threadRoute ?? 'agent'}
      data-clears={log.filter(entry => entry.kind === 'clear').length} data-marks={log.filter(entry => entry.kind === 'mark').length}
      {...stylex.props(styles.page, liveLegacyTheme, assistantDarkTheme)}>
      <Controls world={world} threadRoute={threadRoute} paging={paging}
        onScenario={next => setWorld(createWorld(next, paging))}
        onPaging={next => { setPaging(next); world.registry.set(world.paging, next) }} />
      <style>{'[data-unread-walkthrough-stage] [data-testid="live-agent-workspace"] { height: 100% !important; min-height: 0; }'}</style>
      <div ref={setStageNode} data-unread-walkthrough-stage="" {...stylex.props(styles.stage)} style={{ maxWidth: stage.width, height: stage.height }}>
        <LiveAgentWorkspace navigation={world.navigation} persistGeometry={false} dimensions={{ width: stageWidth, height: stage.height }} />
      </div>
    </main>
  </DataSourceProvider>
}

// A harness around the production workspace, not a module suite: the spaced title keeps it out of the
// one-module-per-`Fractal/App/<Module>` identity check, like 'Inbox threads'.
const meta = {
  title: 'Fractal/App/Threads/Unread walkthrough',
  excludeStories: ['scenarioExemptions'],
  component: Walkthrough,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'Production workspace, LiveThreadPane unread visit controller, MarkedUnreadThreads and the gateway unread port on a real FractalClient, over synthetic thread atoms and an in-memory fetch. No daemon or authenticated API.' },
    docs: { description: { component: 'A controlled world for the private unread reminder. Scenario, list paging and in-app route are switched by the controls above the workspace; fixture updates stand in for another device, the agent and delivery of a pending reply.' } },
  },
} satisfies Meta<typeof Walkthrough>
export default meta

export const scenarioExemptions = ['ControlledWorld:contract-fixture'] as const
type Story = StoryObj<typeof meta>

const need = <T,>(value: T | null | undefined, what: string): T => {
  if (value === null || value === undefined) throw new Error(`no ${what} yet`)
  return value
}

export const ControlledWorld: Story = {
  play: async ({ canvasElement }) => {
    delete canvasElement.dataset.unreadWalkthroughProof
    const canvas = within(canvasElement)
    // A scenario switch mounts a new world, so every lookup reads the current harness again.
    const root = () => need(canvasElement.querySelector<HTMLElement>('[data-unread-walkthrough]'), 'walkthrough')
    const fact = (key: 'scenario' | 'route' | 'clears' | 'marks') => root().dataset[key]
    const stageView = () => within(need(root().querySelector<HTMLElement>('[data-unread-walkthrough-stage]'), 'stage'))
    const indicator = () => root().querySelector('[data-unread-walkthrough-stage] [data-thread-unread-reminder]')
    const row = (id: string) => canvasElement.querySelector<HTMLElement>(`[data-marked-unread-thread="${id}"]`)
    const openRow = async (id: string) => {
      const target = await waitFor(() => need(row(id), `${id} row`), { timeout: 60_000 })
      // On a phone the Inbox is in the real Agents drawer, not beside the thread.
      const opener = stageView().queryByRole('button', { name: 'Expand sidebar' })
      if (opener !== null) await userEvent.click(opener)
      await userEvent.click(within(target).getByRole('button'))
      await waitFor(() => expect(target).toHaveAttribute('aria-busy', 'false'))
      const close = stageView().queryByRole('button', { name: 'Close agents' })
      if (close !== null) await userEvent.click(close)
    }
    const viewReply = async (id: string) => {
      const body = await waitFor(() => need(root().querySelector<HTMLElement>(`[${threadAnchorAttribute}="${id}"] [data-thread-message-content]`), `${id} body`))
      // Route changes do not scroll the outer story. Actually expose the reply,
      // including after returning from the controls above the workspace.
      body.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'instant' })
      await waitFor(() => {
        const rect = body.getBoundingClientRect()
        expect(rect.left).toBeGreaterThanOrEqual(0)
        expect(rect.right).toBeLessThanOrEqual(window.innerWidth)
        expect(rect.bottom).toBeLessThanOrEqual(window.innerHeight)
      })
    }
    const radios = (name: string) => within(canvas.getByRole('radiogroup', { name }))
    await waitFor(root, { timeout: 60_000 })
    await expect(canvasElement.querySelector('[data-walkthrough-legend]')?.textContent).toBe(legend)

    // 1. The owner-wide list reads around the reminded reply first, then changes the in-app route.
    await expect(fact('route')).toBe('agent')
    await openRow(release)
    await waitFor(() => expect(fact('route')).toBe(release), { timeout: 10_000 })
    await expect(canvasElement.querySelector('[data-walkthrough-log-kind="seek"]')?.textContent).toContain('Opened the reminded message')
    // The entry captured the reminder; seeing its reply for a stable 500 ms clears it.
    await viewReply('message/checklist-ready')
    await waitFor(() => expect(fact('clears')).toBe('1'), { timeout: 10_000 })
    await waitFor(() => expect(indicator()).toBeNull())
    await canvas.findByText('No marked-unread threads')

    // 2. A deliberate Mark unread keeps the current visit sticky, well past the dwell.
    await userEvent.click(await stageView().findByRole('button', { name: 'Mark unread from the first visible reply' }))
    await waitFor(() => expect(fact('marks')).toBe('1'))
    await waitFor(() => expect(indicator()).not.toBeNull())
    await settle(1_500)
    await expect(fact('clears')).toBe('1')
    await expect(indicator()).not.toBeNull()
    await waitFor(() => expect(row(release)).not.toBeNull())

    // 3. Leaving for the agent context ends the visit without clearing.
    await userEvent.click(radios('View').getByRole('radio', { name: 'Agent conversation' }))
    await waitFor(() => expect(fact('route')).toBe('agent'))
    await waitFor(() => expect(root().querySelector('[data-unread-walkthrough-stage] details[aria-label="Threads"]')).not.toBeNull(), { timeout: 30_000 })
    await settle(700)
    await expect(fact('clears')).toBe('1')

    // 4. A deliberate return is a new visit: it captures the mark and clears it once seen.
    await userEvent.click(radios('View').getByRole('radio', { name: 'Thread tab' }))
    await waitFor(() => expect(fact('route')).toBe(release))
    await viewReply('message/checklist-ready')
    await waitFor(() => expect(fact('clears')).toBe('2'), { timeout: 10_000 })
    await waitFor(() => expect(indicator()).toBeNull())

    // 5. A mark made elsewhere during this visit was never captured, so it stays.
    await userEvent.click(canvas.getByRole('button', { name: 'Mark unread on another device' }))
    await waitFor(() => expect(indicator()).not.toBeNull())
    await settle(1_200)
    await expect(fact('clears')).toBe('2')

    // 6. Around reads: pending and unavailable stay in the list with the reason; current routes.
    await userEvent.click(radios('Scenario').getByRole('radio', { name: 'Around: current / pending / unavailable' }))
    await waitFor(() => expect(fact('scenario')).toBe('around'))
    await expect(fact('route')).toBe('agent')
    await openRow(cancellation)
    await waitFor(() => expect(row(cancellation)).toHaveTextContent('This reminder’s message has not arrived yet.'), { timeout: 10_000 })
    await openRow(migration)
    await waitFor(() => expect(row(migration)).toHaveTextContent('This reminder’s message is no longer available.'), { timeout: 10_000 })
    await expect(fact('route')).toBe('agent')
    await expect(fact('clears')).toBe('0')
    // An empty first page that names a cursor still lists every reminder.
    await userEvent.click(radios('List paging').getByRole('radio', { name: 'Empty first page + cursor' }))
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-marked-unread-thread]')).toHaveLength(3))
    await expect(canvas.queryByText('No marked-unread threads')).toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: 'Deliver the pending reply' }))
    await openRow(cancellation)
    await waitFor(() => expect(fact('route')).toBe(cancellation), { timeout: 10_000 })
    await viewReply('message/cancellation-retry')
    await waitFor(() => expect(fact('clears')).toBe('1'), { timeout: 10_000 })
    await userEvent.click(radios('List paging').getByRole('radio', { name: 'One page' }))

    // 7. A closed thread opens from the list, stays closed, and clears the same way.
    await userEvent.click(radios('Scenario').getByRole('radio', { name: 'Closed thread' }))
    await waitFor(() => expect(fact('scenario')).toBe('closed'))
    await openRow(rollout)
    await waitFor(() => expect(fact('route')).toBe(rollout), { timeout: 10_000 })
    await stageView().findByText('This thread is closed. Replying won’t reopen it.', {}, { timeout: 10_000 })
    await viewReply('message/rollout-done')
    await waitFor(() => expect(fact('clears')).toBe('1'), { timeout: 10_000 })
    await expect(stageView().getByRole('button', { name: 'Reopen thread' })).toBeVisible()

    // Leave the canvas at the starting scene for a manual recording.
    await userEvent.click(radios('Scenario').getByRole('radio', { name: 'Reminder & revisit' }))
    await waitFor(() => expect(fact('scenario')).toBe('revisit'))
    await waitFor(() => expect(row(release)).not.toBeNull(), { timeout: 30_000 })
    await expect(fact('route')).toBe('agent')
    canvasElement.dataset.unreadWalkthroughProof = 'passed'
  },
}

const styles = stylex.create({
  page: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', width: '100%', minHeight: '100vh', boxSizing: 'border-box', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  controls: { display: 'flex', flexDirection: 'column', gap: s.sm, boxSizing: 'border-box', width: '100%', maxWidth: 1232, padding: s.md },
  line: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, minWidth: 0 },
  legend: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.primary, borderRadius: r.sm, color: c.fg, fontSize: t.metaSize, fontWeight: 600, whiteSpace: 'nowrap' },
  note: { margin: 0, color: c.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  group: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.xs },
  groupLabel: { marginInlineEnd: s.xs, color: c.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap' },
  segment: { display: 'inline-flex', alignItems: 'center', boxSizing: 'border-box', height: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm, color: c.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap', cursor: 'pointer', outline: 'none', ':hover': { backgroundColor: c.rowHover, color: c.fg } },
  segmentSelected: { borderColor: c.primary, backgroundColor: c.rowHover, color: c.fg },
  focus: { outline: `${g.focusRing} solid ${c.primary}` },
  action: { display: 'inline-flex', alignItems: 'center', boxSizing: 'border-box', height: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.sm, backgroundColor: c.controlFill, color: c.fg, fontSize: t.metaSize, whiteSpace: 'nowrap', cursor: 'pointer', outline: { default: 'none', ':focus-visible': `${g.focusRing} solid ${c.primary}` }, ':hover': { backgroundColor: c.rowHover } },
  guide: { margin: 0, maxWidth: '80ch', color: c.fg, fontSize: t.metaSize, lineHeight: t.metaLeading },
  status: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 1fr)', gap: s.md, minWidth: 0 },
  facts: { display: 'grid', gridTemplateColumns: 'auto minmax(0, 1fr)', columnGap: s.sm, rowGap: s.xs, margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  term: { color: c.fgMuted },
  value: { margin: 0, minWidth: 0, overflowWrap: 'anywhere', fontVariantNumeric: 'tabular-nums' },
  log: { display: 'flex', flexDirection: 'column', gap: 2, margin: 0, paddingInlineStart: 0, listStyle: 'none', color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums' },
  logEntry: { minWidth: 0, overflowWrap: 'anywhere' },
  stage: { position: 'relative', flexShrink: 0, width: `calc(100% - ${s.md} - ${s.md})`, boxSizing: 'border-box', overflow: 'hidden', marginInline: s.md, marginBlockEnd: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border },
})
