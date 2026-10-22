import type { Meta } from '@storybook/react'
import { describe, expect, it, vi } from 'vitest'

import * as scenarios from '../src/index.ts'
import type { WireSlice } from '../src/react/index.ts'
import { useScenarioSlice } from '../src/react/index.ts'
import { scenarioStoryCheck, type ScenarioMountedWindow, type ScenarioStoryParameter } from '../src/storybook/check.ts'

const text = (value: unknown): string => {
  if (typeof value !== 'object' || value === null) return ''
  return Object.entries(value).map(([key, item]) => key === 'text' && typeof item === 'string' ? item : text(item)).join(' ')
}

const Tail = ({ omit = false }: { readonly omit?: boolean }) => {
  const conversation = useScenarioSlice('conversation')
  if (conversation.loading) return <p>Loading conversation</p>
  return <div>{conversation.state.threads.map(thread => {
    const starts = thread.items.flatMap((entry, index) => entry.type === 'content' && entry.role === 'user' ? [index] : [])
    const start = starts.length <= 6 ? 0 : starts.at(-2) ?? 0
    const entries = thread.items.slice(start).filter(entry => entry.type === 'content')
    return <section key={thread.agent}>{(omit ? [] : entries).map(entry => <p key={entry.id}>{text(entry.body)}</p>)}</section>
  })}</div>
}

// Transcript.tsx uses initialTailCount={2}; EmbraceScrollViewport's historyBatchSize=4 mounts <=6 turns whole.
const scenario = { _tag: 'slices', slices: ['conversation'], world: 'long-debug-rabbit-hole', window: { _tag: 'tail', turns: 2, historyBatch: 4 } } satisfies ScenarioStoryParameter
const meta = { title: 'Mounted window proof', render: () => <Tail />, parameters: { scenario } } satisfies Meta

type Entry = WireSlice<'conversation'>['state']['threads'][number]['items'][number]
const sourceWorld = scenarios.loadWorld('fleet-mid-refactor', { now: scenarios.ANCHOR_MS })
const sourceThread = sourceWorld.slices.conversation.state.threads[0]
if (sourceThread === undefined) throw new Error('Mounted-window proof requires a source thread')
const sourceContent = sourceThread.items.find(entry => entry.type === 'content')
const sourceTool = sourceThread.items.find(entry => entry.type === 'tool_call')
if (sourceContent === undefined || sourceTool === undefined) throw new Error('Mounted-window proof requires content and tool entries')
const prompt = (index: number, marker = `Window boundary prompt ${index}`): Entry => ({
  ...sourceContent, id: `entry/window-${index}`, sequence: index + 1, role: 'user', body: { media_type: 'text/plain', text: marker },
})

// Inject only the catalog lookup: real wire folding, world overrides, portable rendering,
// read tracking and contrast checks still run. The public catalog has no exact 6/7-turn fixtures.
const withEntries = (items: readonly Entry[], check: () => void): void => {
  const world = sourceWorld.with({ conversation: {
    ...sourceWorld.slices.conversation, timeline: [],
    state: { threads: [{ ...sourceThread, items: [...items] }] },
  } })
  const lookup = vi.spyOn(scenarios, 'loadWorld').mockReturnValue(world)
  try { check() } finally { lookup.mockRestore() }
}

const Rows = ({ tail = false }: { readonly tail?: boolean }) => {
  const conversation = useScenarioSlice('conversation')
  return <div>{conversation.state.threads.flatMap(thread => {
    const items = tail ? thread.items.slice(-2) : thread.items
    return items.map(entry => <p key={entry.id}>{entry.type === 'content' ? text(entry.body) : ''}</p>)
  })}</div>
}

describe('story-declared mounted conversation window', () => {
  it('passes a tail-only render and preserves read, pinned-world and URL checks', () => {
    const result = scenarioStoryCheck({}, meta)
    expect(result._tag).toBe('Passed')
    expect(result.reads).toEqual(new Set(['conversation']))
    for (const check of [1, 2, 4, 5]) expect(result.checks.find(item => item.check === check)?._tag).toBe('Passed')
  })

  it('requires a marker inside the mounted window', () => {
    const result = scenarioStoryCheck({ render: () => <Tail omit /> }, meta)
    expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'DataDependence', slice: 'conversation', message: expect.stringContaining('does not render expected marker') }))
  })

  it.each([0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY])('rejects invalid tail count %s', turns => {
    const result = scenarioStoryCheck({ parameters: { scenario: { ...scenario, window: { _tag: 'tail', turns, historyBatch: 4 } } } }, meta)
    expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'MountedWindow', message: 'tail window turns must be a positive safe integer' }))
  })

  it('rejects a window that selects zero entries', () => {
    const result = scenarioStoryCheck({ args: { conversation: 'empty' } }, meta)
    expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'MountedWindow', message: 'mounted window selects zero conversation entries' }))
  })

  it('keeps the all-source marker rule without a window or with an explicit all window', () => {
    for (const window of [undefined, { _tag: 'all' }] satisfies readonly (ScenarioMountedWindow | undefined)[]) {
      const result = scenarioStoryCheck({ parameters: { scenario: { ...scenario, window } } }, meta)
      expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'DataDependence', message: expect.stringContaining('does not render expected marker') }))
    }
  })

  it.each(['empty', 'default'])('treats all and omitted windows identically for a %s baseline', conversation => {
    const story = { render: () => <Rows />, args: { conversation } }
    const omitted = scenarioStoryCheck({ ...story, parameters: { scenario: { ...scenario, window: undefined } } }, meta)
    const all = scenarioStoryCheck({ ...story, parameters: { scenario: { ...scenario, window: { _tag: 'all' } } } }, meta)
    expect(omitted._tag).toBe('Passed')
    expect(all).toEqual(omitted)
  })

  it('does not waive declared/read mismatches', () => {
    const result = scenarioStoryCheck({ render: () => <p>Fixed</p> }, meta)
    expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'Reads', check: 1 }))
  })

  it('checks unpinned world contrasts within the same mount policy', () => {
    const unpinned = { ...meta, parameters: { scenario: { ...scenario, world: undefined } } }
    const result = scenarioStoryCheck({}, unpinned, { scenario: 'long-debug-rabbit-hole' })
    expect(result.failures).toEqual([])
    expect(result.checks.find(item => item.check === 3)?._tag).toBe('Passed')
  })

  it('mounts all six turns at the short-conversation boundary, but only the tail at seven', () => {
    for (const count of [6, 7]) withEntries(Array.from({ length: count }, (_, index) => prompt(index)), () => {
      const complete = scenarioStoryCheck({ render: () => <Rows tail={count === 7} /> }, meta)
      expect(complete.failures).toEqual([])
      const tailOnly = scenarioStoryCheck({ render: () => <Rows tail /> }, meta)
      if (count === 6) expect(tailOnly.failures).toContainEqual(expect.objectContaining({
        _tag: 'DataDependence', message: expect.stringContaining('does not render expected marker'),
      }))
      else expect(tailOnly.failures).toEqual([])
    })
  })

  it('counts an assistant-only or tool-only prefix as one turn before the first user turn', () => {
    const assistant: Entry = { ...sourceContent, id: 'entry/window-prefix', role: 'assistant', body: { media_type: 'text/plain', text: 'Prefix assistant marker' } }
    for (const prefix of [assistant, sourceTool]) {
      const items = [prefix, ...Array.from({ length: 6 }, (_, index) => prompt(index))]
      withEntries(items, () => {
        // Six user prompts plus the prefix is seven turns, so only two prompts mount.
        expect(scenarioStoryCheck({ render: () => <Rows tail /> }, meta).failures).toEqual([])
      })
    }
  })

  it('rejects a nonempty window excluding every marker in a marker-bearing conversation', () => {
    const items = Array.from({ length: 7 }, (_, index) => prompt(index, index < 5 ? `Excluded marker ${index}` : ''))
    withEntries(items, () => {
      const result = scenarioStoryCheck({ render: () => <Rows tail /> }, meta)
      expect(result.failures).toContainEqual(expect.objectContaining({
        _tag: 'MountedWindow', message: 'mounted window excludes every conversation marker',
      }))
    })
  })
})
