import * as React from 'react'
import type { ArgTypes, Decorator, Loader, Parameters } from '@storybook/react'

import type { SliceKind, World } from '../index.ts'
import type { ReadTracker, ServedSlices } from '../react/index.ts'
import { DEFAULT_WORLD, defaultVariants, worldNames } from './manifest.ts'

export interface ScenarioParameter {
  readonly slices: readonly SliceKind[]
  readonly world?: string
  readonly invariant?: Partial<Record<SliceKind, string>>
  readonly tracker?: ReadTracker | ServedSlices
}

export interface ScenarioParameters extends Parameters {
  readonly scenario?: ScenarioParameter
}

export const scenarioGlobalTypes = {
  scenario: {
    name: 'Scenario',
    description: 'The scenario world supplying story data',
    defaultValue: DEFAULT_WORLD,
    toolbar: { items: worldNames.map(({ id, title }) => ({ value: id, title })), dynamicTitle: true },
  },
  scenarioNow: {
    name: 'Scenario time',
    description: 'Pinned epoch milliseconds or RFC 3339 instant; unset uses load time',
  },
}

export const scenarioArgTypes = (slices: readonly SliceKind[], world?: World): ArgTypes =>
  Object.fromEntries(slices.map((kind) => [kind, {
    control: { type: 'select' },
    options: ['default', ...(world?.available ?? defaultVariants)[kind].filter((variant) => variant !== 'default')],
    defaultValue: 'default',
    table: { defaultValue: { summary: 'default' } },
  }]))

/** Storybook runs loaders before rendering, including after globals/args change. */
export const scenarioLoaders: Loader[] = [async (context) => {
  if (context.parameters.scenario === undefined) return {}
  // Static imports would make every story pay for the recording catalog.
  const runtime = await import('./runtime.ts')
  return { scenarioRender: runtime.withScenario }
}]

const isDecorator = (value: unknown): value is Decorator => typeof value === 'function'

export const withScenario: Decorator = (Story, context) => {
  if (context.parameters.scenario === undefined) return React.createElement(Story)
  const render: unknown = context.loaded.scenarioRender
  if (!isDecorator(render)) throw new Error('withScenario requires scenarioLoaders in preview.loaders')
  return render(Story, context)
}
