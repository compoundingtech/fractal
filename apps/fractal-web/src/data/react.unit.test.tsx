import {
  Agent,
  decodeUnknownSync,
  type AgentEncoded,
  type ResourceObservationEncoded,
  type TerminalScreen,
} from '@fractal/client/schema'
import type { DecodedAgent } from '@fractal/sdk/effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'

import { fixtureAttention, fixtureMissions } from '../missions/fixtures.ts'
import { gatewayResources } from '../resources/agent/source.ts'
import { fixtureSource, type FixtureProjections } from './fixtureSource.ts'
import { DataSourceProvider, type SubjectIndex, useSubjectIndex, useSubjectList, useGatewaySync, useAgentsSync, useMissionsSync, useAttentionSync, useConversationSync, useTerminalSync } from './react.tsx'
import { type ConversationPage, type DataSource, observed, unavailable, waiting } from './source.ts'
import { initialFeedSync, transitionFeedSync, type FeedSyncObservation } from './feedSync.ts'
import { useAtomValue } from '@effect/atom-react'
import { useAgent, useAgentAttention, useDataSource } from './react.tsx'
import { sidebarRow } from '../web/sidebarRow.ts'
import type { SidebarMissionContext } from '@fractal/ui/assistant-ui'
import type { Feed } from './source.ts'

const blocked = unavailable({ reason: 'unsupported', detail: 'This family is not served.' })
const agent = decodeUnknownSync(
  Agent,
  'strict',
)({
  kind: 'agent',
  id: 'agent/discovery',
  name: 'Discovery',
  host_id: 'host/example',
  runtime_ids: [],
  reachability: 'reachable',
  state: 'running',
  harness_state: 'idle',
  blocked_on: null,
  fault: null,
  revision: '1',
  updated_at: '2026-10-04T12:00:00.000Z',
} satisfies AgentEncoded)
const mission = fixtureMissions[0]!
const attention = fixtureAttention[0]!
const world: FixtureProjections = {
  now: 0,
  events: [],
  agents: [agent],
  missions: [mission],
  attention: [attention],
  conversations: {},
  terminals: {},
  envelopes: {},
  usage: { _tag: 'undeclared' },
}

const renderSubjects = (source: DataSource, registry: AtomRegistry.AtomRegistry): string => {
  const Consumer = () => (
    <ul>
      {useSubjectList().map((subject) => (
        <li key={subject.ref}>{subject.ref}</li>
      ))}
    </ul>
  )
  return renderToStaticMarkup(
    <DataSourceProvider source={source} registry={registry}>
      <Consumer />
    </DataSourceProvider>,
  )
}

const readIndex = (source: DataSource, registry: AtomRegistry.AtomRegistry): SubjectIndex => {
  let index: SubjectIndex | undefined
  const Consumer = () => {
    index = useSubjectIndex()
    return null
  }
  renderToStaticMarkup(
    <DataSourceProvider source={source} registry={registry}>
      <Consumer />
    </DataSourceProvider>,
  )
  return index!
}

describe('independent subject discovery through the data provider', () => {
  it.each([waiting, blocked])(
    'keeps observed agents and resources when missions and attention are $_tag',
    async (missing) => {
      const registry = AtomRegistry.make()
      const resource = {
        id: 'resource/example/discovery',
        kind: 'filesystem.file',
        facts: { path: 'flakes/webfractal/src/data/react.tsx' },
        observed_at: '2026-10-04T12:00:00.000Z',
        opened_by: agent.id,
        opened_by_run: null,
      } satisfies ResourceObservationEncoded
      const resources = gatewayResources({
        baseUrl: 'http://fixture.invalid',
        fetchImpl: async (input) =>
          Response.json({
            api_version: 'st3.client.v0',
            snapshot: {},
            value: String(input).endsWith('/v1/client/capabilities')
              ? { limits: { max_page_items: 100 } }
              : {
                  collection: 'resources',
                  kind: 'page',
                  filters: {},
                  items: [resource],
                  page: { has_more: false, next_cursor: null, limit: 50 },
                },
          }),
      })
      const source = {
        ...fixtureSource({ world, overrides: { missions: missing, attention: missing } }),
        resources,
      }
      const unmount = registry.mount(resources.byId(resource.id))
      try {
        await vi.waitFor(() =>
          expect(registry.get(resources.byId(resource.id))._tag).toBe('Observed'),
        )
        const markup = renderSubjects(source, registry)
        expect(markup).toContain(`<li>${agent.id}</li>`)
        expect(markup).toContain(`<li>${resource.id}</li>`)
        expect(markup).not.toContain(`<li>${mission.id}</li>`)
      } finally {
        unmount()
        registry.dispose()
      }
    },
  )

  it.each([
    { ref: mission.id, overrides: { agents: blocked, attention: waiting } },
    { ref: attention.id, overrides: { agents: blocked, missions: waiting } },
  ])('discovers $ref independently of unavailable siblings', ({ ref, overrides }) => {
    const registry = AtomRegistry.make()
    try {
      const source = fixtureSource({ world, overrides })
      expect(renderSubjects(source, registry)).toContain(`<li>${ref}</li>`)
    } finally {
      registry.dispose()
    }
  })

  it('does not delete stale agents when another family becomes unavailable or returns empty', () => {
    const registry = AtomRegistry.make()
    const missions = Atom.make(observed({ value: world.missions })).pipe(Atom.keepAlive)
    const attentionFeed = Atom.make(observed({ value: world.attention })).pipe(Atom.keepAlive)
    const source = {
      ...fixtureSource({
        world,
        overrides: { agents: observed({ value: world.agents, freshness: 'stale' }) },
      }),
      missions,
      attention: attentionFeed,
    }
    try {
      const cached = readIndex(source, registry)
      const terminal = cached.subjects.find((subject) => subject.ref === 'terminal/discovery')
      expect(terminal).toMatchObject({ icon: 'terminal', detail: expect.stringContaining('stale') })
      expect(terminal?.status).toBeUndefined()
      expect(cached.subjects.find((subject) => subject.ref === agent.id)?.detail).toContain('stale')
      expect(
        cached.subjects
          .filter((subject) => subject.ref === attention.id)
          .map((subject) => subject.ref),
      ).toEqual([attention.id])
      expect(renderSubjects(source, registry)).toContain(`<li>${agent.id}</li>`)
      registry.set(missions, blocked)
      registry.set(attentionFeed, waiting)
      expect(renderSubjects(source, registry)).toContain(`<li>${agent.id}</li>`)
      const missing = readIndex(source, registry)
      expect(missing.families.missions._tag).toBe('Unavailable')
      expect(missing.families.attention._tag).toBe('Waiting')
      expect(missing.families.agents._tag === 'Observed' && missing.families.agents.freshness).toBe(
        'stale',
      )
      expect(missing.resources).toBeUndefined()
      registry.set(missions, observed({ value: [] }))
      registry.set(attentionFeed, blocked)
      expect(renderSubjects(source, registry)).toContain(`<li>${agent.id}</li>`)
      const empty = readIndex(source, registry)
      expect(empty.families.missions._tag === 'Observed' && empty.families.missions.value).toEqual(
        [],
      )
      expect(empty.families.attention._tag).toBe('Unavailable')
      registry.set(missions, observed({ value: world.missions, freshness: 'stale' }))
      expect(renderSubjects(source, registry)).toContain(`<li>${mission.id}</li>`)
      expect(renderSubjects(source, registry)).toContain(`<li>${agent.id}</li>`)
    } finally {
      registry.dispose()
    }
  })
})

describe('SDK sync observation hooks', () => {
  it('passes exact SDK observations through independently of already observed fixture content', () => {
    const registry = AtomRegistry.make()
    const base = fixtureSource({ world })
    const agents = transitionFeedSync(initialFeedSync<readonly DecodedAgent[]>(100), { _tag: 'Requested', since: 125 }, 125)
    const gateway: FeedSyncObservation = { status: { _tag: 'Requested', since: 110 }, observedAt: 110 }
    const source: DataSource = {
      ...base,
      sync: {
        gateway: Atom.make(gateway),
        agents: Atom.make(agents),
        missions: Atom.make(initialFeedSync<typeof world.missions>(100)),
        attention: Atom.make(initialFeedSync<typeof world.attention>(100)),
        conversation: () => Atom.make(initialFeedSync<ConversationPage>(100)),
        terminal: () => Atom.make(initialFeedSync<TerminalScreen>(100)),
      },
    }
    let observations: readonly (FeedSyncObservation | undefined)[] = []
    const Consumer = () => {
      observations = [useGatewaySync(), useAgentsSync(), useMissionsSync(), useAttentionSync(), useConversationSync(agent.id), useTerminalSync('terminal/example')]
      return null
    }
    try {
      renderToStaticMarkup(<DataSourceProvider source={source} registry={registry}><Consumer /></DataSourceProvider>)
      expect(observations[0]).toBe(gateway)
      expect(observations[1]).toBe(agents.sync)
      expect(observations.slice(2).map((value) => value?.status)).toEqual(Array.from({ length: 4 }, () => ({ _tag: 'Connecting', attempt: 1, since: 100 })))
      expect(registry.get(source.agents)._tag).toBe('Observed')
    } finally {
      registry.dispose()
    }
  })

  it('does not infer Live or unavailable sync facts for a source without SDK observations', () => {
    const registry = AtomRegistry.make()
    let observations: readonly (FeedSyncObservation | undefined)[] = []
    const Consumer = () => {
      observations = [useGatewaySync(), useAgentsSync(), useMissionsSync(), useAttentionSync(), useConversationSync(agent.id), useTerminalSync('terminal/example')]
      return null
    }
    try {
      renderToStaticMarkup(<DataSourceProvider source={fixtureSource({ world })} registry={registry}><Consumer /></DataSourceProvider>)
      expect(observations).toEqual(Array.from({ length: 6 }, () => undefined))
    } finally {
      registry.dispose()
    }
  })
})

describe('sidebar retained mission feed plumbing', () => {
  it('updates mission and waiting context independently of an unchanged agent roster', () => {
    const registry = AtomRegistry.make()
    const native = decodeUnknownSync(Agent, 'strict')({
      kind: 'agent', id: 'agent/build-1/worker', name: 'Worker', host_id: 'host/build-1',
      runtime_ids: [], reachability: 'reachable', state: 'running', revision: '1',
      updated_at: '2026-10-20T12:00:00Z',
      current_work: [{
        id: 'work/build-1/review', mission_id: mission.id, mission_run_id: 'mission-run/build-1/run',
        path: 'review', title: 'Review changes', state: 'claimed', since: '2026-10-20T12:00:00Z',
      }],
    } satisfies AgentEncoded)
    const missions = Atom.make<Feed<typeof world.missions>>(waiting)
    const cards = Atom.make<Feed<typeof world.attention>>(observed({ value: [] }))
    const base = fixtureSource({ world: { ...world, agents: [native], attention: [] } })
    const source: DataSource = { ...base, missions, attention: cards }
    let context: SidebarMissionContext | undefined
    const Consumer = () => {
      const feed = useAgent(native.id)
      const missionFeed = useAtomValue(useDataSource().missions)
      const attentionFeed = useAgentAttention(native.id)
      context = feed._tag === 'Observed' && feed.value !== undefined ? sidebarRow({
        agent: feed.value, stale: false,
        missions: missionFeed._tag === 'Observed' ? missionFeed.value : undefined,
        attention: attentionFeed._tag === 'Observed' ? attentionFeed.value : undefined,
      }).missionContext : undefined
      return null
    }
    const render = () => renderToStaticMarkup(<DataSourceProvider source={source} registry={registry}><Consumer /></DataSourceProvider>)
    try {
      render()
      expect(context).toMatchObject({ progress: { _tag: 'Unknown' }, step: { _tag: 'Step', name: 'Review changes' } })
      registry.set(missions, observed({ value: [{ ...mission, title: 'Actual published title' }] }))
      render()
      expect(context).toMatchObject({ slug: mission.id.split('/').at(-1) })
      const run = { ...mission.run_details![0]!, id: 'mission-run/build-1/run' as const, generation_id: undefined,
        progress: { done: 1, total: 4 } }
      const hydrated = { ...mission, run_generations: {}, run_details: [run] }
      registry.set(missions, observed({ value: [hydrated] }))
      render()
      expect(context).toMatchObject({ progress: { _tag: 'Known', done: 1, total: 4 } })
      registry.set(missions, observed({ value: [{ ...hydrated, run_details: [{ ...run, progress: { done: 2, total: 4 } }] }] }))
      render()
      expect(context).toMatchObject({ progress: { _tag: 'Known', done: 2, total: 4 } })
      const card = { ...attention, attention_kind: 'agent-request' as const, state: 'open' as const,
        actions: ['work.done' as const], conversation_id: native.id, detail: 'Approve the plan?', update: undefined }
      registry.set(cards, observed({ value: [card] }))
      render()
      expect(context).toMatchObject({ step: { _tag: 'Waiting', question: 'Approve the plan?' } })
      registry.set(cards, observed({ value: [{ ...card, detail: 'Approve the revised plan?' }] }))
      render()
      expect(context).toMatchObject({ step: { _tag: 'Waiting', question: 'Approve the revised plan?' } })
      registry.set(cards, observed({ value: [] }))
      render()
      expect(context).toMatchObject({ step: { _tag: 'Step', name: 'Review changes' } })
      expect(registry.get(source.agents)).toMatchObject({ value: [native] })
    } finally {
      registry.dispose()
    }
  })
})
