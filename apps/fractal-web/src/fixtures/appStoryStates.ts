/**
 * Availability and repair states for the production-mounted app stories.
 *
 * Each state is one `fixtureSource` over the deterministic fixture world (`projections.ts`):
 * the stories mount the real `LiveAgentWorkspace` / `ConversationPane` modules against it, so
 * no gateway is contacted and no session, agent or clock is invented here.
 */
import * as Atom from 'effect/reactivity/Atom'

import { stLiveFrames } from '../conversation/fixtures.ts'
import { timelineItems } from '../conversation/fromTimeline.ts'
import { fixtureSource } from '../data/fixtureSource.ts'
import {
  observed,
  unavailable,
  waiting,
  type ConversationPage,
  type DataSource,
} from '../data/source.ts'
import { fixtureProjections } from './projections.ts'

export const appStoryStates = [
  'loading',
  'empty',
  'populated',
  'failed-tool',
  'offline',
  'unavailable',
  'scripted-repair',
] as const
export type AppStoryState = (typeof appStoryStates)[number]

export interface AppStoryFixture {
  readonly source: DataSource
  readonly agentRef: string
  readonly agentName: string
}

/** The fixture world's interactive shell session; its full follow is the populated thread. */
const workbenchShell = 'agent/build-host-a/workbench-shell'
/** The fixture world's failed CAS migration: an errored shell tool call and a harness fault. */
const casJanitor = 'agent/build-host-b/cas-janitor'

/** The agent a state selects; stories seed the workspace's persisted selection with it. */
export const appStoryAgent = (state: AppStoryState): string =>
  state === 'failed-tool' ? casJanitor : workbenchShell

const storyPage = (state: AppStoryState, agentRef: string): ConversationPage => {
  // A native page that observed no entries, not a filtered page.
  if (state === 'empty') return { items: [], hasOlder: false, observation: { empty: true } }
  // The repair session as st follows it today: the final answer is still streaming.
  if (state === 'scripted-repair') return { items: timelineItems(stLiveFrames), hasOlder: true }
  const page = fixtureProjections.conversations[agentRef]
  if (page === undefined) throw new Error(`App story world has no conversation for ${agentRef}`)
  return page
}

export const createAppStoryFixture = (state: AppStoryState): AppStoryFixture => {
  const agentRef = appStoryAgent(state)
  const agent = fixtureProjections.agents.find((row) => row.id === agentRef)
  if (agent === undefined) throw new Error(`App story world has no agent ${agentRef}`)
  const source = fixtureSource({
    world: fixtureProjections,
    overrides: {
      conversation: {
        [agentRef]: state === 'loading'
          ? waiting
          : state === 'unavailable'
            ? unavailable({ reason: 'failed', detail: 'Conversation not found', code: 'not-found' })
            : observed({ value: storyPage(state, agentRef), freshness: state === 'offline' ? 'stale' : 'live' }),
      },
      ...(state === 'offline'
        ? {
            // What the socket reports while the browser is offline: no scheduled retry.
            connection: { _tag: 'Reconnecting', attempt: 1, issue: 'The browser is offline' },
            agents: observed({ value: fixtureProjections.agents, freshness: 'stale' }),
          }
        : {}),
    },
  })
  return {
    source:
      state === 'offline' ? { ...source, network: Atom.make({ _tag: 'Offline' as const }) } : source,
    agentRef,
    agentName: agent.name,
  }
}
