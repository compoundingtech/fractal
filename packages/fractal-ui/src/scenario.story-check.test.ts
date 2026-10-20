import { scenarioStoryCheck, withScenario } from '@smalltalk/st3-scenarios/storybook/check'
import { loadWorld } from '@smalltalk/st3-scenarios'
import { scenarioSections } from './scenario-transcript.story-harness'
import { expect, test } from 'vitest'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { composeStory } from '@storybook/react'
import meta, { AllStates } from './Markdown.stories.tsx'
import syncMeta, { Scenario, LocalFailure, UnknownFailure, ServerFailure } from './SyncLine.stories.tsx'
import * as systemModule from './SystemState.stories.tsx'
import declarationMeta, { MissionStructuredFallback, ScenarioMissionDeclarations } from './DeclarationPanel.stories.tsx'
import composerMeta, { ReadingColumn, ReadingColumnLight, HostWidth, ScenarioConversation as ComposerScenarioConversation, ScenarioConversationLight as ComposerScenarioConversationLight } from './TranscriptComposer.stories.tsx'
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

for (const [name, story] of Object.entries({ ReadingColumn, ReadingColumnLight, HostWidth, ComposerScenarioConversation, ComposerScenarioConversationLight })) {
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

test('scenario transcript preserves the active migrator turn without a terminal status', () => {
  const world = loadWorld('fleet-mid-refactor')
  const section = scenarioSections(world.slices.conversation).sections.find(({ title }) => title.includes('/migrator'))
  expect(section).toBeDefined()
  expect(section?.turns.at(-1)?.work.running).toBe(true)
})

test('scenario transcript marks only the first huge section of each incomplete thread as having older history', () => {
  const world = loadWorld('huge')
  const sections = scenarioSections(world.slices.conversation).sections
  expect(sections.some(({ history }) => history?._tag === 'HasOlder')).toBe(true)
  for (const section of sections) {
    const first = section.id.endsWith('/0')
    expect(section.history?._tag).toBe(first ? 'HasOlder' : 'Complete')
    expect(section.turns.every(({ work }) => work.commands === undefined && work.changedFiles === undefined)).toBe(true)
  }
})

test('scenario transcript preserves tool operations and failed tool lifecycle', () => {
  const world = loadWorld('fleet-mid-refactor')
  const toolTurns = scenarioSections(world.with({ conversation: 'tool-heavy' }).slices.conversation).sections.flatMap(({ turns }) => turns)
  expect(toolTurns.some(({ work }) => work.running && work.commands === 5 && work.changedFiles === 1)).toBe(true)
  expect(toolTurns.flatMap(({ work }) => work.calls).some(call => call.kind === 'run' && call.command !== undefined)).toBe(true)
  expect(toolTurns.flatMap(({ work }) => work.calls).some(call => call.kind === 'edit' && call.changedPath === 'src/session.ts')).toBe(true)
  const failedTurns = scenarioSections(world.with({ conversation: 'failed-tools' }).slices.conversation).sections.flatMap(({ turns }) => turns)
  expect(failedTurns.some(({ work }) => work.failed && work.running)).toBe(true)
})

test('connection geometry header and bounded history belong to the same thread', () => {
  const Composed = composeStory({ ...systemStories.BandKeepsFollow, parameters: { scenario: { slices: ['conversation'], world: 'fleet-mid-refactor' } } }, systemMeta, { decorators: [withScenario] })
  const markup = renderToStaticMarkup(React.createElement(Composed))
  const world = loadWorld('fleet-mid-refactor').with({ conversation: 'long' })
  const agent = world.slices.conversation.state.threads[0]!.agent
  const rowIds = [...markup.matchAll(/data-item-id="([^"]+)"/g)].map(match => match[1]!)
  expect(rowIds).toHaveLength(48)
  expect(rowIds.every(id => id.startsWith(`${agent}/`))).toBe(true)
  expect(markup).toContain(`>${agent}</`)
})
