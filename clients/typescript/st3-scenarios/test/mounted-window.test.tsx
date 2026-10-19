import type { Meta } from '@storybook/react'
import { describe, expect, it } from 'vitest'

import { useScenarioSlice } from '../src/react/index.ts'
import { scenarioStoryCheck, type ScenarioStoryParameter } from '../src/storybook/check.ts'

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
const scenario = { slices: ['conversation'], world: 'long-debug-rabbit-hole', window: { _tag: 'tail', turns: 2, historyBatch: 4 } } satisfies ScenarioStoryParameter
const meta = { title: 'Mounted window proof', render: () => <Tail />, parameters: { scenario } } satisfies Meta

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
    const result = scenarioStoryCheck({ parameters: { scenario: { window: { _tag: 'tail', turns, historyBatch: 4 } } } }, meta)
    expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'MountedWindow', message: 'tail window turns must be a positive safe integer' }))
  })

  it('rejects a window that selects zero entries', () => {
    const result = scenarioStoryCheck({ args: { conversation: 'empty' } }, meta)
    expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'MountedWindow', message: 'mounted window selects zero conversation entries' }))
  })

  it('keeps the all-source marker rule without a window or with an explicit all window', () => {
    for (const window of [undefined, { _tag: 'all' }] satisfies readonly (ScenarioStoryParameter['window'])[]) {
      const result = scenarioStoryCheck({ parameters: { scenario: { ...scenario, window } } }, meta)
      expect(result.failures).toContainEqual(expect.objectContaining({ _tag: 'DataDependence', message: expect.stringContaining('does not render expected marker') }))
    }
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
})
