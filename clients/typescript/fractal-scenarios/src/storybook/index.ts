import * as React from 'react'
import type { ArgTypes, Decorator, Loader, Parameters } from '@storybook/react'

import type { SliceKind, World } from '../index.ts'
import type { ReadTracker, ServedSlices } from '../react/index.ts'
import { DEFAULT_WORLD, defaultVariants, worldNames } from './manifest.ts'
import { ScenarioReadBoundary } from '../react/index.ts'

/** Per-thread conversation mount policy; other slices retain their full marker contract. */
export type ScenarioMountedWindow =
  | { readonly _tag: 'all' }
  /** The final `rows` source rows (text content and tool calls) plus the latest user prompt; at most `rows` mount whole. */
  | { readonly _tag: 'sourceRows'; readonly rows: number }

export interface ScenarioParameter {
  readonly _tag: 'slices'
  readonly slices: readonly SliceKind[]
  readonly world?: string
  readonly invariant?: Partial<Record<SliceKind, string>>
  readonly tracker?: ReadTracker | ServedSlices
  readonly window?: ScenarioMountedWindow
}

export type ScenarioStoryExemption = {
  readonly _tag: 'contract-fixture' | 'primitive'
  readonly reason: string
}

export type ScenarioStoryParameter = ScenarioParameter | ScenarioStoryExemption

/** Exemptions cannot claim scenario wiring; arbitrary fixture reads are not observable here. */
export const validateScenarioDeclaration = (
  parameter: ScenarioStoryParameter | undefined,
  args: Readonly<Record<string, unknown>>,
): void => {
  if (parameter === undefined) throw new Error('Story requires parameters.scenario declaration')
  if (parameter._tag === 'slices') {
    if (parameter.slices.length === 0) throw new Error('scenario slices requires at least one slice')
    return
  }
  if (parameter._tag !== 'contract-fixture' && parameter._tag !== 'primitive') throw new Error('parameters.scenario requires a valid _tag')
  if (typeof parameter.reason !== 'string' || parameter.reason.trim().length === 0) throw new Error(`scenario ${parameter._tag} requires a reason`)
  if ('slices' in parameter || 'world' in parameter || 'invariant' in parameter || 'tracker' in parameter || 'window' in parameter) {
    throw new Error(`scenario ${parameter._tag} cannot declare slices or scenario configuration; a story must not change scenario mode from meta; declare the mode per story or split the meta`)
  }
  const scenarioArgs = Object.keys(defaultVariants).filter((key) => key in args)
  if (scenarioArgs.length > 0) throw new Error(`scenario ${parameter._tag} cannot supply scenario args: ${scenarioArgs.join(', ')}`)
}

export interface ScenarioParameters extends Parameters {
  readonly scenario?: ScenarioStoryParameter
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
  const parameter: ScenarioStoryParameter | undefined = context.parameters.scenario
  if (parameter === undefined) return {}
  validateScenarioDeclaration(parameter, context.args)
  if (parameter._tag !== 'slices') return {}
  // Static imports would make every story pay for the recording catalog.
  const runtime = await import('./runtime.ts')
  return { scenarioRender: runtime.withScenario }
}]

const isDecorator = (value: unknown): value is Decorator => typeof value === 'function'

export const withScenario: Decorator = (Story, context) => {
  const parameter: ScenarioStoryParameter | undefined = context.parameters.scenario
  if (parameter === undefined) return React.createElement(ScenarioReadBoundary, { reason: 'no parameters.scenario declaration' }, React.createElement(Story))
  validateScenarioDeclaration(parameter, context.args)
  if (parameter._tag !== 'slices') return React.createElement(ScenarioReadBoundary, { reason: `scenario ${parameter._tag} exemption` }, React.createElement(Story))
  const render: unknown = context.loaded.scenarioRender
  if (!isDecorator(render)) throw new Error('withScenario requires scenarioLoaders in preview.loaders')
  return render(Story, context)
}
