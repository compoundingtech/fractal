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

type Entry = WireSlice<'conversation'>['state']['threads'][number]['items'][number]
const isSourceRow = (entry: Entry) => entry.type === 'tool_call'
  || entry.type === 'content' && (entry.role === 'user' || entry.role === 'assistant') && typeof entry.body.text === 'string'

// Mirrors Transcript independently: the newest six source rows plus the latest user prompt.
const Tail = ({ omit = false }: { readonly omit?: boolean }) => {
  const conversation = useScenarioSlice('conversation')
  if (conversation.loading) return <p>Loading conversation</p>
  return <div>{conversation.state.threads.map(thread => {
    const rows = thread.items.filter(isSourceRow)
    const latestPrompt = rows.filter(entry => entry.type === 'content' && entry.role === 'user').at(-1)
    const mounted = rows.length <= 6 ? rows : rows.filter((entry, index) => index >= rows.length - 6 || entry === latestPrompt)
    const entries = mounted.filter(entry => entry.type === 'content')
    return <section key={thread.agent}>{(omit ? [] : entries).map(entry => <p key={entry.id}>{text(entry.body)}</p>)}</section>
  })}</div>
}

// Transcript.tsx exports transcriptMountedWindow = { _tag: 'sourceRows', rows: transcriptInitialSourceRowCount } (6).
const scenario = { _tag: 'slices', slices: ['conversation'], world: 'long-debug-rabbit-hole', window: { _tag: 'sourceRows', rows: 6 } } satisfies ScenarioStoryParameter
const meta = { title: 'Mounted window proof', render: () => <Tail />, parameters: { scenario } } satisfies Meta

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

const answer = (index: number): Entry => ({
  ...sourceContent, id: `entry/answer-${index}`, sequence: 100 + index, role: 'assistant', body: { media_type: 'text/plain', text: `Window boundary answer ${index}` },
})

/** Renders the given entries' content markers: all of them, or only the newest `last`. */
const Rows = ({ last, select }: { readonly last?: number; readonly select?: (items: readonly Entry[]) => readonly Entry[] }) => {
  const conversation = useScenarioSlice('conversation')
  return <div>{conversation.state.threads.flatMap(thread => {
    const items = select !== undefined ? select(thread.items) : last === undefined ? thread.items : thread.items.slice(-last)
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

  it.each([0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY])('rejects invalid source-row count %s', rows => {
    const result = scenarioStoryCheck({ parameters: { scenario: { ...scenario, window: { _tag: 'sourceRows', rows } } } }, meta)
    expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'MountedWindow', message: 'sourceRows window rows must be a positive safe integer' }))
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

  it('mounts all six source rows at the short-conversation boundary, but only the newest six at seven', () => {
    const missing = expect.objectContaining({ _tag: 'DataDependence', message: expect.stringContaining('does not render expected marker') })
    for (const count of [6, 7]) withEntries(Array.from({ length: count }, (_, index) => prompt(index)), () => {
      expect(scenarioStoryCheck({ render: () => <Rows /> }, meta).failures).toEqual([])
      expect(scenarioStoryCheck({ render: () => <Rows last={6} /> }, meta).failures).toEqual([])
      // No whole-mount slack: one row fewer than the bound always misses a mounted marker.
      expect(scenarioStoryCheck({ render: () => <Rows last={5} /> }, meta).failures).toContainEqual(missing)
    })
  })

  it('counts an assistant-only or tool-only prefix as a source row outside the newest six', () => {
    const assistant: Entry = { ...sourceContent, id: 'entry/window-prefix', role: 'assistant', body: { media_type: 'text/plain', text: 'Prefix assistant marker' } }
    for (const prefix of [assistant, sourceTool]) {
      const items = [prefix, ...Array.from({ length: 6 }, (_, index) => prompt(index))]
      withEntries(items, () => {
        // The prefix plus six prompts is seven source rows, so only the six prompts mount.
        expect(scenarioStoryCheck({ render: () => <Rows last={6} /> }, meta).failures).toEqual([])
      })
    }
  })

  it('requires the latest user prompt above the newest six rows of its long turn, without the rows between', () => {
    const items = [prompt(0), ...Array.from({ length: 9 }, (_, index) => answer(index))]
    withEntries(items, () => {
      expect(scenarioStoryCheck({ render: () => <Rows select={entries => entries.length === 0 ? [] : [entries[0]!, ...entries.slice(-6)]} /> }, meta).failures).toEqual([])
      expect(scenarioStoryCheck({ render: () => <Rows last={6} /> }, meta).failures).toContainEqual(expect.objectContaining({
        _tag: 'DataDependence', message: expect.stringContaining('does not render expected marker'),
      }))
    })
  })

  it('rejects a nonempty window excluding every marker in a marker-bearing conversation', () => {
    const items = Array.from({ length: 7 }, (_, index) => prompt(index, index === 0 ? 'Excluded marker 0' : ''))
    withEntries(items, () => {
      const result = scenarioStoryCheck({ render: () => <Rows last={6} /> }, meta)
      expect(result.failures).toContainEqual(expect.objectContaining({
        _tag: 'MountedWindow', message: 'mounted window excludes every conversation marker',
      }))
    })
  })
})
