import * as React from 'react'
import { composeStory } from '@storybook/react'
import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { scenarioGlobalTypes, scenarioLoaders, withScenario } from '../src/storybook/index.ts'
import { useScenarioSlice } from '../src/react/index.ts'

const annotations = { globalTypes: scenarioGlobalTypes, loaders: scenarioLoaders, decorators: [withScenario] }

it('renders an exempt story without loading scenario runtime even with an invalid scenario global', async () => {
  const Story = composeStory({ render: () => React.createElement('p', {}, 'ordinary story'), parameters: {
    scenario: { _tag: 'primitive', reason: 'Plain typography without scenario data' },
  } }, {}, {
    ...annotations, initialGlobals: { scenario: 'not-a-world' },
  })
  await Story.load()
  expect(renderToStaticMarkup(React.createElement(Story))).toBe('<p>ordinary story</p>')
})

it('awaits the catalog before rendering and honors pinned worlds, time and slice args', async () => {
  const Consumer = () => React.createElement('p', {}, useScenarioSlice('roster').state.agents.length)
  const Story = composeStory({
    render: Consumer,
    parameters: { scenario: { _tag: 'slices', slices: ['roster'], world: 'fleet-mid-refactor' } },
    args: { roster: 'one-agent' },
  }, {}, { ...annotations, initialGlobals: { scenario: 'empty', scenarioNow: 1_800_000_000_000 } })
  await Story.load()
  const markup = renderToStaticMarkup(React.createElement(Story))
  expect(markup).toContain('data-scenario="fleet-mid-refactor"')
  expect(markup).toContain('data-scenario-slices="roster:one-agent"')
  expect(markup).toContain('<p>1</p>')
})

it('propagates invalid world errors after loading rather than silently rendering', async () => {
  const Story = composeStory({ render: () => React.createElement('p'), parameters: { scenario: { _tag: 'slices', slices: [] } } }, {}, {
    ...annotations, initialGlobals: { scenario: 'not-a-world' },
  })
  await Story.load()
  expect(() => renderToStaticMarkup(React.createElement(Story))).toThrow('unknown scenario world not-a-world')
})

it.each(['contract-fixture', 'primitive'] as const)('rejects useScenarioSlice under a %s exemption at render', async (_tag) => {
  const Consumer = () => React.createElement('p', {}, useScenarioSlice('roster').state.agents.length)
  const Story = composeStory({
    render: Consumer,
    parameters: { scenario: { _tag, reason: 'A deliberately invalid hook consumer' } },
  }, {}, annotations)
  await Story.load()
  expect(() => renderToStaticMarkup(React.createElement(Story))).toThrow(`useScenarioSlice is forbidden under scenario ${_tag} exemption`)
})

it('rejects useScenarioSlice without a declaration at render', async () => {
  const Consumer = () => React.createElement('p', {}, useScenarioSlice('roster').state.agents.length)
  const Story = composeStory({ render: Consumer }, {}, annotations)
  await Story.load()
  expect(() => renderToStaticMarkup(React.createElement(Story))).toThrow('no parameters.scenario declaration')
})
