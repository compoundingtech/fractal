import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { describe, expect, it } from 'vitest'
import * as Option from 'effect/Option'
import { appStoryAgent, appStoryStates, createAppStoryFixture } from './appStoryStates.ts'
import { fixtureProjections } from './projections.ts'

describe('production app story states', () => {
  for (const state of appStoryStates) {
    it(`serves ${state} through fixtureSource without a gateway`, () => {
      const fixture = createAppStoryFixture(state)
      expect(fixture.agentRef).toBe(appStoryAgent(state))
      expect(fixtureProjections.agents.some(agent => agent.id === fixture.agentRef)).toBe(true)
      const registry = AtomRegistry.make()
      try {
        expect(registry.get(fixture.source.now)).toBe(fixtureProjections.now)
        const feed = registry.get(fixture.source.conversation(fixture.agentRef))
        expect(feed._tag).toBe(state === 'loading' ? 'Waiting' : state === 'unavailable' ? 'Unavailable' : 'Observed')
        if (fixture.source.sync === undefined) throw new Error('Production status channels must be present')
        const sync = registry.get(fixture.source.sync.conversation(fixture.agentRef))
        expect(sync.sync.status._tag).toBe(state === 'loading' ? 'Connecting' : state === 'unavailable' ? 'Failed' : state === 'offline' ? 'Stale' : 'Live')
        expect(registry.get(fixture.source.sync.gateway).status._tag).toBe(state === 'offline' ? 'Stale' : 'Live')
        expect(Option.isSome(sync.last)).toBe(feed._tag === 'Observed')
        if (feed._tag !== 'Observed') return
        expect(feed.freshness).toBe(state === 'offline' ? 'stale' : 'live')
        expect(feed.value.items.length === 0).toBe(state === 'empty')
        if (state === 'failed-tool')
          expect(feed.value.items.some(item => item._tag === 'ToolCall' && item.status === 'error')).toBe(true)
        if (state === 'scripted-repair')
          expect(feed.value.items.some(item => item._tag === 'Text' && item.role === 'assistant' && item.streaming)).toBe(true)
        if (state === 'offline') {
          expect(registry.get(fixture.source.connection)._tag).toBe('Reconnecting')
          expect(fixture.source.network === undefined ? undefined : registry.get(fixture.source.network)).toEqual({ _tag: 'Offline' })
        } else expect(registry.get(fixture.source.connection)._tag).not.toBe('Reconnecting')
      } finally { registry.dispose() }
    })
  }

  it('gives the repair session and the populated thread distinct transcripts', () => {
    const registry = AtomRegistry.make()
    try {
      const page = (state: 'populated' | 'scripted-repair') => {
        const fixture = createAppStoryFixture(state)
        const feed = registry.get(fixture.source.conversation(fixture.agentRef))
        if (feed._tag !== 'Observed') throw new Error(`${state} must be observed`)
        return feed.value.items
      }
      expect(page('scripted-repair')).not.toEqual(page('populated'))
    } finally { registry.dispose() }
  })
})
