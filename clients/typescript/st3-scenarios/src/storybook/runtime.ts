import * as React from 'react'
import type { Decorator } from '@storybook/react'
import { DEFAULT_WORLD, loadWorld, type Clock, type World, type WorldOverrides } from '../index.ts'
import { ScenarioProvider, type ReadTracker, type ServedSlices } from '../react/index.ts'
import type { ScenarioParameter } from './index.ts'

export const resolveScenarioWorld = (parameter: ScenarioParameter, globals: Readonly<Record<string, unknown>>, args: Readonly<Record<string, unknown>>): World => {
  const selected = parameter.world ?? globals.scenario ?? DEFAULT_WORLD
  if (typeof selected !== 'string') throw new Error('scenario global must name a world')
  const now = globals.scenarioNow
  if (now !== undefined && typeof now !== 'number' && typeof now !== 'string' && !(now instanceof Date)) throw new Error('scenarioNow must be an instant')
  const world = loadWorld(selected, now === undefined ? {} : { now })
  const overrides: WorldOverrides = Object.fromEntries(parameter.slices.flatMap((kind) => {
    const variant = args[kind]
    return typeof variant === 'string' && variant !== 'default' ? [[kind, variant]] : []
  }))
  return world.with(overrides)
}

export const withScenario: Decorator = (Story, context) => {
  const parameter: ScenarioParameter | undefined = context.parameters.scenario
  if (parameter === undefined) return React.createElement(Story)
  const world = resolveScenarioWorld(parameter, context.globals, context.args)
  const tracker: ReadTracker | ServedSlices | undefined = context.parameters.scenarioReadTracker ?? parameter.tracker
  const clock: Clock | undefined = context.parameters.scenarioClock
  return React.createElement(ScenarioProvider, { world, tracker, clock },
    React.createElement('div', {
      'data-scenario': world.id,
      'data-scenario-slices': parameter.slices.map((kind) => `${kind}:${world.variants[kind]}`).join(','),
    }, React.createElement(Story)))
}

