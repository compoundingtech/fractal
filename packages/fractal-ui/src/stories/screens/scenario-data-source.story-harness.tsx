import { optionalSource } from '@fractal/client/markdown'
import { Agent, Attention, Mission, TerminalScreen, TimelineEntry, decodeUnknownSync } from '@fractal/client/schema'
import { useScenarioNow, useScenarioSlice, type WireSlice } from '@fractal/scenarios/react'
import type { SliceKind } from '@fractal/scenarios'
import * as Option from 'effect/Option'
import * as Schema from 'effect/Schema'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'
import { timelineItems } from '../../../../../apps/fractal-web/src/conversation/fromTimeline.ts'
import type { FeedSync } from '../../../../../apps/fractal-web/src/data/feedSync.ts'
import { fixtureSource } from '../../../../../apps/fractal-web/src/data/fixtureSource.ts'
import { DataSourceProvider } from '../../../../../apps/fractal-web/src/data/react.tsx'
import type { ConnectionState } from '@fractal/sdk/read'
import { observed, unavailable, waiting, type ConversationPage, type DataSource, type Feed } from '../../../../../apps/fractal-web/src/data/source.ts'
import { persistedAtom } from '../../../../../apps/fractal-web/src/state/persistence.ts'

type SyncStatus = WireSlice<'sync'>['status'][string]

/** Every slice a Screen reads through this harness; a Screen declares exactly these. */
export const screenSlices = ['roster', 'details', 'attention', 'conversation', 'terminal', 'sync'] as const satisfies readonly SliceKind[]

/** A wire row the current client cannot decode stays out of the product, as the live SDK decode leaves it out. */
const decodable = <TValue,>(decode: (row: unknown) => TValue, rows: readonly unknown[]): TValue[] => rows.flatMap(row => {
  try { return [decode(row)] } catch { return [] }
})
const decodeAgent = decodeUnknownSync(Agent)
const decodeMission = decodeUnknownSync(Mission)
const decodeAttention = decodeUnknownSync(Attention)
const decodeEntry = decodeUnknownSync(TimelineEntry)
const decodeScreen = decodeUnknownSync(TerminalScreen)

/** The feed a client observes for a value at this sync status. */
const feedAt = <TValue,>(value: TValue, loading: boolean, status: SyncStatus | undefined): Feed<TValue> => {
  if (loading) return waiting
  switch (status?._tag) {
    case 'Connecting': case 'Requested': case 'Progress': return waiting
    case 'Failed': return unavailable({ reason: 'failed', detail: status.cause._tag === 'Server' ? status.cause.message : status.cause._tag === 'Local' ? status.cause.detail?.message ?? status.cause.kind : 'Synchronization failed', ...(status.cause._tag === 'Server' ? { code: status.cause.code } : {}) })
    case 'Stale': return observed({ value, freshness: 'stale' })
    default: return observed({ value })
  }
}

const connectionAt = (status: SyncStatus | undefined, loading: boolean): ConnectionState => {
  if (loading) return { _tag: 'Connecting', attempt: 1 }
  switch (status?._tag) {
    case 'Connecting': return { _tag: 'Connecting', attempt: status.attempt } 
    case 'Requested': case 'Progress': return { _tag: 'Connecting', attempt: 1 } 
    case 'Stale': return status.reason._tag === 'Reconnecting' ? { _tag: 'Reconnecting', attempt: status.reason.attempt, issue: status.reason.issue, nextAt: status.reason.nextAt } : { _tag: 'Live' } 
    case 'Failed': return { _tag: 'Rejected', message: status.cause._tag === 'Server' ? status.cause.message : 'Synchronization failed', ...(status.cause._tag === 'Server' ? { code: status.cause.code } : {}) }
    default: return { _tag: 'Live' }
  }
}

const threadPage = (thread: WireSlice<'conversation'>['state']['threads'][number]): ConversationPage => {
  const hasOlder = thread.has_more || (thread.history?.committed_from_sequence ?? 1) > 1
  return thread.items.length === 0
    ? { items: [], hasOlder, observation: { empty: true } }
    : { items: timelineItems([{ entries: thread.items.map(entry => decodeEntry(entry)), replace: true, hasMore: hasOlder }]), hasOlder }
}

interface ScreenWorld {
  readonly source: DataSource
  /** The thread a reviewer lands on: the first agent, in roster order, with conversation. */
  readonly agentRef: string
  readonly agentName: string
  /** World data the mounted product must show once its feed is observed, for the play's data proof. */
  readonly shownAgent: string
  readonly evidence: string
}

const screenWorld = ({ roster, details, attention, conversation, terminal, sync, now }: {
  readonly roster: WireSlice<'roster'>
  readonly details: WireSlice<'details'>
  readonly attention: WireSlice<'attention'>
  readonly conversation: WireSlice<'conversation'>
  readonly terminal: WireSlice<'terminal'>
  readonly sync: WireSlice<'sync'>
  readonly now: number
}): ScreenWorld => {
  const rank = new Map(roster.state.order.map((id, index) => [id, index]))
  const agents = decodable(decodeAgent, roster.state.agents).sort((a, b) => (rank.get(a.id) ?? rank.size) - (rank.get(b.id) ?? rank.size))
  const threads = new Map(conversation.state.threads.map(thread => [thread.agent as string, thread]))
  const primary = agents.find(agent => (threads.get(agent.id)?.items.length ?? 0) > 0) ?? agents[0]
  const pages: Record<string, Feed<ConversationPage>> = Object.fromEntries(agents.map(agent => {
    const thread = threads.get(agent.id)
    const page: ConversationPage = thread === undefined ? { items: [], hasOlder: false, observation: { empty: true } } : threadPage(thread)
    return [agent.id, feedAt(page, conversation.loading, sync.status[`conversation:${agent.id}`])]
  }))
  const missions = decodable(decodeMission, details.state.missions)
  const attentionRows = decodable(decodeAttention, attention.state.attention)
  const rosterStatus = sync.status.agents
  const rosterFeed = feedAt(agents, roster.loading, rosterStatus)
  const primaryPage = primary === undefined ? undefined : pages[primary.id]
  const base = fixtureSource({
    world: {
      now,
      events: [],
      agents,
      missions,
      attention: attentionRows,
      conversations: {},
      terminals: Object.fromEntries(terminal.state.terminals.flatMap(({ terminal: ref, screens }) => decodable(decodeScreen, screens.slice(-1).map(({ screen }) => screen)).map(screen => [ref, screen] as const))),
      envelopes: {},
      usage: { _tag: 'undeclared' },
    },
    overrides: {
      connection: connectionAt(rosterStatus, roster.loading),
      agents: rosterFeed,
      missions: feedAt(missions, details.loading, sync.status.missions),
      attention: feedAt(attentionRows, attention.loading, sync.status.attention),
      conversation: pages,
    },
  })
  const resources = { byAgent: () => Atom.make(observed({ value: { items: [], nextCursor: null } })), byId: () => Atom.make(waiting), subjects: Atom.make([]), loadMore: () => undefined, refresh: () => undefined, resolveFile: () => undefined }
  const feedSync = <TValue,>(feed: Atom.Atom<Feed<TValue>>, status: SyncStatus | undefined): Atom.Atom<FeedSync<TValue>> => Atom.make(get => {
    const value = get(feed)
    const current: SyncStatus = status ?? (value._tag === 'Waiting' ? { _tag: 'Connecting', attempt: 1, since: now } : value._tag === 'Unavailable' ? { _tag: 'Failed', cause: { _tag: 'Server', code: value.code ?? value.reason, message: value.detail ?? 'Unavailable' } } : value.freshness === 'stale' ? { _tag: 'Stale', reason: { _tag: 'Unknown' } } : { _tag: 'Live', since: now })
    return { last: value._tag === 'Observed' ? Option.some({ value: value.value, observedAt: now }) : Option.none(), sync: { observedAt: now, status: current } }
  })
  const source = {
    ...base,
    resources,
    sync: {
      gateway: Atom.make({ observedAt: now, status: rosterStatus ?? { _tag: 'Live', since: now } }),
      agents: feedSync(base.agents, rosterStatus),
      missions: feedSync(base.missions, sync.status.missions),
      attention: feedSync(base.attention, sync.status.attention),
      conversation: Atom.family((ref: string) => feedSync(base.conversation(ref), sync.status[`conversation:${ref}`])),
      terminal: Atom.family((ref: string) => feedSync(base.terminal(ref), sync.status[`terminal:${ref}`])),
    },
  } satisfies DataSource
  const evidence = threads.get(primary?.id ?? '')?.items.findLast(entry => entry.type === 'content' && typeof entry.body.text === 'string' && /^[\p{L}\p{N} ,.'-]{16}/u.test(entry.body.text))
  return {
    source,
    agentRef: primary?.id ?? '',
    agentName: primary?.name ?? '',
    shownAgent: rosterFeed._tag === 'Observed' ? primary?.name ?? '' : '',
    evidence: primaryPage?._tag === 'Observed' && evidence?.type === 'content' ? optionalSource(evidence.body.text)?.slice(0, 16) ?? '' : '',
  }
}

const schemeAtom = () => persistedAtom({ key: 'round2.scheme', schema: Schema.Literals(['dark', 'light']), defaultValue: 'dark' })

const ScreenProvider = ({ world, scheme, children }: { readonly world: ScreenWorld; readonly scheme: 'dark' | 'light'; readonly children: (world: ScreenWorld) => React.ReactNode }) => {
  // Seeded before the product mounts, the same way the embed starts a workspace from one known state.
  const [registry] = React.useState(() => {
    const value = AtomRegistry.make()
    value.set(schemeAtom(), scheme)
    value.set(persistedAtom({ key: 'round2.agent', schema: Schema.String, defaultValue: '' }), world.agentRef)
    value.set(persistedAtom({ key: 'round2.sidebarClosed', schema: Schema.Boolean, defaultValue: false }), false)
    return value
  })
  // The Theme toolbar and scheme control change the mounted product without replacing the world.
  React.useEffect(() => { registry.set(schemeAtom(), scheme) }, [registry, scheme])
  React.useEffect(() => () => registry.dispose(), [registry])
  return <div data-screen-agent={world.shownAgent} data-screen-evidence={world.evidence} style={{ display: 'contents' }}>
    <DataSourceProvider source={world.source} registry={registry}>{children(world)}</DataSourceProvider>
  </div>
}

/**
 * One real product composition over the world the Scenario toolbar selects. Each slice crosses the
 * same `DataSource` seam the live app reads: wire rows decode through the client schema and thread
 * entries fold through the live timeline projection. Usage has no scenario source here, so it stays
 * undeclared rather than borrowing the fixture world's accounts.
 */
export const ScenarioScreen = ({ scheme, children }: { readonly scheme: 'dark' | 'light'; readonly children: (world: ScreenWorld) => React.ReactNode }) => {
  const roster = useScenarioSlice('roster')
  const details = useScenarioSlice('details')
  const attention = useScenarioSlice('attention')
  const conversation = useScenarioSlice('conversation')
  const terminal = useScenarioSlice('terminal')
  const sync = useScenarioSlice('sync')
  const now = useScenarioNow()
  const world = React.useMemo(() => screenWorld({ roster, details, attention, conversation, terminal, sync, now }), [roster, details, attention, conversation, terminal, sync, now])
  const identity = React.useRef({ world, key: 0 })
  if (identity.current.world !== world) identity.current = { world, key: identity.current.key + 1 }
  return <ScreenProvider key={identity.current.key} world={world} scheme={scheme}>{children}</ScreenProvider>
}

/** Proves the product rendered this world's own data, never a harness marker alone. */
export const assertScreenWorld = (canvas: HTMLElement): void => {
  const root = canvas.querySelector<HTMLElement>('[data-screen-agent]')
  if (root === null) throw new Error('The screen harness did not mount')
  const agent = root.dataset.screenAgent ?? ''
  const evidence = root.dataset.screenEvidence ?? ''
  const shown = root.textContent ?? ''
  if (agent !== '' && !shown.includes(agent)) throw new Error(`The product does not show this world's agent ${agent}`)
  if (evidence !== '' && !shown.includes(evidence)) throw new Error(`The product does not show this world's thread text "${evidence}"`)
}
