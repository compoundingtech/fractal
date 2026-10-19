import { scenarioStoryCheck } from '@smalltalk/st3-scenarios/storybook'
import { expect, test } from 'vitest'
import meta, { AllStates } from './Markdown.stories.tsx'

test('Markdown AllStates consumes the declared conversation slice', () => {
  const result = scenarioStoryCheck(AllStates, meta)
  expect(result.failures).toEqual([])
  expect(result._tag).toBe('Passed')
  expect([...result.reads]).toEqual(['conversation'])
  expect(result.checks.map(({ check, _tag }) => ({ check, _tag }))).toEqual([
    { check: 1, _tag: 'Passed' },
    { check: 2, _tag: 'Passed' },
    { check: 3, _tag: 'Passed' },
    { check: 4, _tag: 'Skipped' },
    { check: 5, _tag: 'Passed' },
  ])
  expect(result.checks.find(({ check }) => check === 4)?.reason).toBe('Story does not pin a world')
})
