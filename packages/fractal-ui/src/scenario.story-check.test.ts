import { scenarioStoryCheck } from '@smalltalk/st3-scenarios/storybook'
import { expect, test } from 'vitest'
import meta, { AllStates } from './Markdown.stories.tsx'
import syncMeta, { Scenario, LocalFailure, UnknownFailure, ServerFailure } from './SyncLine.stories.tsx'
import * as systemModule from './SystemState.stories.tsx'
import declarationMeta, { MissionStructuredFallback, ScenarioMissionDeclarations } from './DeclarationPanel.stories.tsx'
import composerMeta, { ReadingColumn, ReadingColumnLight, HostWidth } from './TranscriptComposer.stories.tsx'
import * as pressModule from './TranscriptPress.stories.tsx'

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

for (const [name, story] of Object.entries({ Scenario, LocalFailure, UnknownFailure, ServerFailure })) {
  test(`SyncLine ${name} consumes its declared sync slice`, () => {
    const result = scenarioStoryCheck(story, syncMeta)
    expect(result.failures).toEqual([])
    expect(result._tag).toBe('Passed')
    expect([...result.reads]).toEqual(['sync'])
  })
}

const { default: systemMeta, ...systemStories } = systemModule
for (const [name, story] of Object.entries(systemStories)) {
  test(`SystemState ${name} consumes its declared slices`, () => {
    const result = scenarioStoryCheck(story, systemMeta)
    expect(result.failures).toEqual([])
    expect(result._tag).toBe('Passed')
  })
}

for (const [name, story] of Object.entries({ MissionStructuredFallback, ScenarioMissionDeclarations })) {
  test(`DeclarationPanel ${name} consumes its declared details slice`, () => {
    const result = scenarioStoryCheck(story, declarationMeta)
    expect(result.failures).toEqual([])
    expect(result._tag).toBe('Passed')
    expect([...result.reads]).toEqual(['details'])
  })
}

for (const [name, story] of Object.entries({ ReadingColumn, ReadingColumnLight, HostWidth })) {
  test(`TranscriptComposer ${name} consumes its declared conversation slice`, () => {
    const result = scenarioStoryCheck(story, composerMeta)
    expect(result.failures).toEqual([])
    expect(result._tag).toBe('Passed')
  })
}

const { default: pressMeta, ...pressStories } = pressModule
for (const [name, story] of Object.entries(pressStories)) {
  if (!('parameters' in story) || story.parameters?.scenario === undefined) continue
  test(`TranscriptPress ${name} consumes its declared conversation slice`, () => {
    const result = scenarioStoryCheck(story, pressMeta)
    expect(result.failures).toEqual([])
    expect(result._tag).toBe('Passed')
  })
}
