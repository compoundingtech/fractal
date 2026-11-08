/** @vitest-environment jsdom */
/**
 * Runs the kit's entity hover card stories in jsdom: each story's scenario declaration and the file's reviewed exemption
 * list (as scenario.story-check.test.ts does repo-wide), then every play function against the real composition.
 * Layout assertions in the plays read zero in jsdom; the browser play gate stays the proof for geometry.
 */
import { scenarioStoryCheck } from '@fractal/scenarios/storybook/check'
import { composeStories } from '@storybook/react'
import { isExportStory } from 'storybook/internal/csf'
import { createRoot } from 'react-dom/client'
import { flushSync } from 'react-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'

// Node rendering exercises the real composition, not the CSS compiler.
// The kit's other modules also call `when.*`, `defaultMarker` and `firstThatWorks` at module scope.
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  defineConsts: (constants: unknown) => constants,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
  defaultMarker: () => ({}),
  defineMarker: () => ({}),
  firstThatWorks: (...values: unknown[]) => values[0],
  positionTry: () => 'test-position',
  viewTransitionClass: () => 'test-view-transition',
  when: new Proxy({}, { get: () => () => ':test-when' }),
}))

import * as module from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityHoverCard.stories.tsx'

const { default: meta, scenarioExemptions } = module
const storyNames = Object.keys(module).filter(name => name !== 'default' && isExportStory(name, meta))
const composed = composeStories(module)

describe('Entity hover card stories', () => {
  it('lists exactly its stories as reviewed exemptions', () => {
    expect([...scenarioExemptions].map(entry => entry.split(':')[0])).toEqual([...storyNames].sort())
    expect(isExportStory('scenarioExemptions', meta)).toBe(false)
  })

  for (const name of storyNames) {
    it(`${name} validates its scenario mode`, () => {
      const result = scenarioStoryCheck(module[name as keyof typeof module] as never, meta as never)
      expect(result.failures).toEqual([])
      expect(result._tag).toBe('Passed')
      expect(result.exemptions).toHaveLength(1)
    })
  }

  let host: HTMLElement | undefined
  let root: ReturnType<typeof createRoot> | undefined
  afterEach(() => {
    root?.unmount()
    host?.remove()
  })

  for (const [name, Story] of Object.entries(composed)) {
    it(`${name} plays`, async () => {
      host = document.body.appendChild(document.createElement('div'))
      root = createRoot(host)
      flushSync(() => root!.render(<Story />))
      await Story.play?.({ canvasElement: host } as never)
    })
  }
})
