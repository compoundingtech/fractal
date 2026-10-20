import { expect, test } from 'vitest'
import ts from 'typescript'
import { checkFixtureGraph, createTypeScriptGraph, requiresDomainMarker, type ExemptStoryMode } from './scenario-fixture-graph.ts'

// Virtual sources exercise the production TypeScript emitter/resolver, not a mock import parser.
const policy = {
  kitSource: '/fixture-graph/kit/src',
  uiSource: '/fixture-graph/ui/src',
  projectionHarness: '/fixture-graph/ui/src/scenario-transcript.story-harness.ts',
}
const source = (name: string): string => `${policy.uiSource}/${name}`
const sources: Readonly<Record<string, string>> = {
  [source('catalog.stories.ts')]: "import { value } from './catalog-bridge'; export const story = value",
  [source('catalog-bridge.ts')]: "export { value } from '../../kit/src/index'",
  [`${policy.kitSource}/index.ts`]: 'export const value = 1',
  [source('harness.stories.ts')]: "import { value } from './scenario-transcript.story-harness'; export const story = value",
  [policy.projectionHarness]: 'export const value = 1',
  [source('marked.stories.ts')]: "import { value } from './marked-data'; export const story = value",
  [source('marked-data.ts')]: 'export const scenarioDomainFixtureMarker = 1; export const value = 2',
  [source('missing.stories.ts')]: "import { value } from '../../kit/src/worlds/missing'; export const story = value",
  [`${policy.kitSource}/worlds/missing.ts`]: 'export const value = 1',
  [`${policy.kitSource}/domain-fixture.ts`]: "export const scenarioDomainFixture: unique symbol = Symbol('scenario-domain-fixture')",
  [`${policy.kitSource}/worlds/marked.ts`]: "import { scenarioDomainFixture } from '../domain-fixture'; export const scenarioDomainFixtureMarker: typeof scenarioDomainFixture = scenarioDomainFixture",
  [source('contract.stories.ts')]: "export { value } from './bridge'",
  [source('bridge.ts')]: "export { value } from './local-fixture'",
  [source('local-fixture.ts')]: 'export const value = 1',
  [source('types.stories.ts')]: "import type { Shape } from '../../kit/src/worlds/types'; import { OtherShape } from '../../kit/src/worlds/types'; export const story: Shape | OtherShape = { value: 1 }",
  [`${policy.kitSource}/worlds/types.ts`]: 'export interface Shape { value: number }; export interface OtherShape { value: number }',
  [source('loader.stories.ts')]: "export const load = () => import('../../kit/src/index'); if (globalThis) { void import('../../kit/src/index') }; export const conditional = globalThis ? import('../../kit/src/index') : undefined",
  [source('eager.stories.ts')]: "export const loaded = import('../../kit/src/index')",
  [source('unresolved.stories.ts')]: "import './not-present'",
  [source('recording.stories.ts')]: "import data from '../../kit/recordings/data.json'; export const story = data",
  '/fixture-graph/kit/recordings/data.json': '{}',
}
const graph = createTypeScriptGraph(Object.keys(sources).filter(file => file.endsWith('.ts')), {
  target: ts.ScriptTarget.ESNext,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  noLib: true,
  types: [],
}, sources)
const check = (file: string, mode: ExemptStoryMode) => checkFixtureGraph(new Map([[source(file), new Set([mode])]]), graph, policy)

test('primitive transitive catalog imports fail with the full path', () => {
  expect(check('catalog.stories.ts', 'primitive').violations).toEqual([
    `primitive: ${source('catalog.stories.ts')} -> ${source('catalog-bridge.ts')} -> ${policy.kitSource}/index.ts (forbidden domain/fixture module)`,
  ])
})
test('contract fixtures cannot import the projection harness or a marked module', () => {
  expect(check('harness.stories.ts', 'contract-fixture').violations.join('\n')).toContain(policy.projectionHarness)
  expect(check('marked.stories.ts', 'contract-fixture').violations.join('\n')).toContain(source('marked-data.ts'))
})
test('unmarked worlds fail both reachability and required marker coverage', () => {
  const world = `${policy.kitSource}/worlds/missing.ts`
  expect(check('missing.stories.ts', 'contract-fixture').violations.join('\n')).toContain(world)
  expect(requiresDomainMarker(world, policy)).toBe(true)
  expect(graph.hasTypedMarker(world)).toBe(false)
  expect(graph.hasTypedMarker(`${policy.kitSource}/worlds/marked.ts`)).toBe(true)
  expect(graph.hasTypedMarker(source('marked-data.ts'))).toBe(false)
})
test('local contract-data is allowed only for contract fixtures and reports transitive importers', () => {
  const result = check('contract.stories.ts', 'contract-fixture')
  expect(result.violations).toEqual([])
  expect([...result.contractData]).toEqual([[source('local-fixture.ts'), [
    `${source('contract.stories.ts')} -> ${source('bridge.ts')} -> ${source('local-fixture.ts')}`,
  ]]])
  expect(requiresDomainMarker(source('local-fixture.ts'), policy)).toBe(false)
  expect(check('contract.stories.ts', 'primitive').violations.join('\n')).toContain(source('local-fixture.ts'))
})
test('explicit and inferred erased type imports do not create runtime edges', () => {
  expect(check('types.stories.ts', 'primitive').violations).toEqual([])
})
test('conditional dynamic loaders are excluded but eager dynamic imports are checked', () => {
  expect(check('loader.stories.ts', 'primitive').violations).toEqual([])
  expect(check('eager.stories.ts', 'primitive').violations.join('\n')).toContain(`${policy.kitSource}/index.ts`)
})
test('unresolved runtime edges fail closed instead of disappearing from the graph', () => {
  expect(check('unresolved.stories.ts', 'primitive').violations).toEqual([
    `${source('unresolved.stories.ts')} -> ./not-present (unresolved runtime import)`,
  ])
})
test('raw recording JSON cannot bypass domain ownership checks', () => {
  expect(check('recording.stories.ts', 'contract-fixture').violations.join('\n')).toContain('/fixture-graph/kit/recordings/data.json')
})
