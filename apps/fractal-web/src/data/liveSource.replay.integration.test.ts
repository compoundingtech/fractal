import { it } from '@effect/vitest'
import { catalog, loadWorld, type World } from '@smalltalk/st3-scenarios'
import { createReplay, manualClock, type ManualClock } from '@smalltalk/st3-scenarios/replay'
import { Effect } from 'effect'
import { afterEach, beforeEach, describe, expect, vi } from 'vitest'
import { liveSource, type LiveSource } from './liveSource.ts'

// Non-recorded edge world: a small roster, tolerant decoding and a timed conversation append.
const world = loadWorld('unknown-fields', { now: '2026-10-01T12:00:00.000Z' })

let frames = new Map<number, FrameRequestCallback>()
beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  frames = new Map()
  let nextFrame = 0
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = (nextFrame += 1)
    frames.set(id, callback)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
})
afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

/** Release replay socket opens, then drain the real SDK, browser decode tasks and frame writer. */
const settle = (clock: ManualClock) => Effect.promise(async () => {
  for (let round = 0; round < 10; round += 1) {
    clock.advance(0)
    await new Promise<void>((resolve) => setImmediate(resolve))
    await vi.advanceTimersByTimeAsync(1)
    const pending = [...frames.values()]
    frames.clear()
    for (const callback of pending) callback(0)
  }
})

const withReplay = <A>(input: World, test: (live: LiveSource, clock: ManualClock) => Effect.Effect<A>) =>
  Effect.gen(function* () {
    const clock = manualClock(input.now)
    const replay = yield* Effect.acquireRelease(
      Effect.sync(() => createReplay(input, { clock })),
      (replay) => Effect.sync(() => replay.close()),
    )
    const live = yield* Effect.acquireRelease(
      Effect.sync(() => liveSource({ options: {
        baseUrl: 'http://gateway.test', maxFollows: 2, socket: replay.socket, fetch: replay.fetch,
      } })),
      (live) => Effect.promise(() => live.dispose()),
    )
    yield* Effect.promise(() => live.ready)
    const agent = input.slices.conversation.state.threads[0]!.agent
    const releases = [
      live.registry.mount(live.source.agents),
      live.registry.mount(live.source.conversationInterest!(agent)),
      live.registry.mount(live.source.conversation(agent)),
    ]
    yield* Effect.addFinalizer(() => Effect.sync(() => {
      for (const release of releases.reverse()) release()
    }))
    yield* settle(clock)
    expect(replay.served().has('roster')).toBe(true)
    expect(replay.served().has('conversation')).toBe(true)
    return yield* test(live, clock)
  }).pipe(Effect.scoped)

const rosterIds = (live: LiveSource) => {
  const roster = live.registry.get(live.source.agents)
  expect(roster._tag).toBe('Observed')
  if (roster._tag !== 'Observed') throw new Error('Replay roster was not observed')
  return roster.value.map((agent) => agent.id).sort()
}

const conversationItems = (live: LiveSource) => {
  const thread = live.registry.get(live.source.conversation(world.slices.conversation.state.threads[0]!.agent))
  expect(thread._tag).toBe('Observed')
  if (thread._tag !== 'Observed') throw new Error('Replay conversation was not observed')
  return thread.value.items
}

describe('scenario replay through the real live source', () => {
  it.live('decodes the world roster and conversation, then publishes a timed append', () =>
    withReplay(world, (live, clock) => Effect.gen(function* () {
      expect(catalog.some((entry) => entry.id === world.id)).toBe(true)
      const expectedAgents = world.slices.roster.state.agents
      expect(expectedAgents.length).toBeGreaterThan(0)
      expect(rosterIds(live)).toEqual(expectedAgents.map((agent) => agent.id).sort())
      expect(live.registry.get(live.source.agents)).toMatchObject({
        _tag: 'Observed', value: [{ id: expectedAgents[0]!.id, state: { _tag: 'Unknown', raw: 'reconciling' } }],
      })

      const thread = world.slices.conversation.state.threads[0]!
      const initial = conversationItems(live)
      const initialIds = initial.map((item) => item.id)
      // Mail headers can be joined with their content; the future diagnostic entry must survive intact.
      expect(initial.length).toBeGreaterThan(0)
      expect(initialIds.every((id) => thread.items.some((entry) => entry.id === id))).toBe(true)
      expect(initial).toContainEqual(expect.objectContaining({
        _tag: 'UnknownEvent', id: thread.items.at(-1)!.id, eventType: 'diagnostic_summary',
      }))

      const event = world.slices.conversation.timeline[0]!
      if (event._tag !== 'entries') throw new Error('Expected the edge world timed append')
      clock.advance(event.at_ms + 1)
      yield* settle(clock)
      const updated = conversationItems(live)
      expect(updated).toHaveLength(initial.length + event.items.length)
      expect(updated.map((item) => item.id)).toEqual([...initialIds, ...event.items.map((entry) => entry.id)])
      expect(updated.at(-1)).toMatchObject({
        _tag: 'UnknownEvent', eventType: 'diagnostic_summary', data: { status: 'passed' },
      })
    })),
  )

  it.live('changes the real projection when the replay roster slice removes an agent', () =>
    Effect.gen(function* () {
      const original = yield* withReplay(world, (live) => Effect.succeed(rosterIds(live)))
      const roster = world.slices.roster
      const removed = roster.state.agents[0]!.id
      const changed = world.with({ roster: {
        ...roster,
        state: { ...roster.state, agents: roster.state.agents.filter((agent) => agent.id !== removed), order: roster.state.order.filter((id) => id !== removed) },
      } })
      const altered = yield* withReplay(changed, (live) => Effect.succeed(rosterIds(live)))
      expect(original).toContain(removed)
      expect(altered).not.toContain(removed)
      expect(altered).toEqual(original.filter((id) => id !== removed))
      expect(altered).not.toEqual(original)
    }),
  )
})
