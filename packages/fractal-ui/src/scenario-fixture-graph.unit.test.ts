import * as path from 'node:path'
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
  [source('kit-loader.stories.ts')]: "export { scenarioLoaders } from '../../kit/src/storybook/index'",
  [`${policy.kitSource}/storybook/index.ts`]: "export const scenarioLoaders = [async context => { const runtime = await import('./runtime'); return runtime }]",
  [`${policy.kitSource}/storybook/runtime.ts`]: "export { value } from '../index'",
  [source('iife.stories.ts')]: "export const value = (() => import('../../kit/src/index'))()",
  [source('condition.stories.ts')]: "if (await import('../../kit/src/index')) { }",
  [source('switch.stories.ts')]: "switch (await import('../../kit/src/index')) { default: break }",
  [source('logical.stories.ts')]: "void (import('../../kit/src/index') && true)",
  [source('unresolved.stories.ts')]: "import './not-present'",
  [source('recording.stories.ts')]: "import data from '../../kit/recordings/data.json'; export const story = data",
  '/fixture-graph/kit/recordings/data.json': '{}',
  [source('html.stories.ts')]: "import text from './page.html?raw'; export const story = text",
  [source('page.html')]: '<html>Example</html>',
  [source('font.stories.ts')]: "import url from './font.ttf?url'; export const story = url",
  [source('font.ttf')]: 'example-font',
  [source('missing-asset.stories.ts')]: "import text from './missing.html?raw'; export const story = text",
  [source('raw-domain.stories.ts')]: "import text from '../../kit/src/index.ts?raw'; export const story = text",
  [source('raw-fixture.stories.ts')]: "import text from './local-fixture.ts?raw'; export const story = text",
  [source('raw-marked.stories.ts')]: "import text from './marked-data.ts?raw'; export const story = text",
  [source('unknown-query.stories.ts')]: "import text from './page.html?unknown'; export const story = text",
  [source('raw-code.stories.ts')]: "import text from './catalog-bridge.ts?raw'; export const story = text",
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
test('only the exact resolved kit loader edge is excluded', () => {
  expect(check('kit-loader.stories.ts', 'primitive').violations).toEqual([])
  expect(check('loader.stories.ts', 'primitive').violations.join('\n')).toContain(`${policy.kitSource}/index.ts`)
  expect(check('eager.stories.ts', 'primitive').violations.join('\n')).toContain(`${policy.kitSource}/index.ts`)
})
// These dynamic imports intentionally exercise the loader's module-loading boundary.
test.each([
  ['static', "import './runtime'; export const scenarioLoaders = []"],
  ['eager', "const runtime = await import('./runtime'); export const scenarioLoaders = [runtime]"],
  ['other function', "export const load = async () => { const runtime = await import('./runtime'); return runtime }"],
  ['nested IIFE', "export const scenarioLoaders = [async () => (() => import('./runtime'))()]"],
])('the loader path cannot exempt a %s import', (_kind, loaderSource) => {
  const seededSources = { ...sources, [`${policy.kitSource}/storybook/index.ts`]: loaderSource }
  const seededGraph = createTypeScriptGraph(Object.keys(seededSources).filter(file => file.endsWith('.ts')), {
    target: ts.ScriptTarget.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    noLib: true,
    types: [],
  }, seededSources)
  const result = checkFixtureGraph(new Map([[source('kit-loader.stories.ts'), new Set<ExemptStoryMode>(['primitive'])]]), seededGraph, policy)
  expect(result.violations.join('\n')).toContain(`${policy.kitSource}/storybook/runtime.ts`)
  expect(result.violations.join('\n')).toContain(`${policy.kitSource}/index.ts`)
})
test.each(['iife', 'condition', 'switch', 'logical'])('dynamic imports in %s expressions cannot bypass the graph', name => {
  expect(check(`${name}.stories.ts`, 'contract-fixture').violations.join('\n')).toContain(`${policy.kitSource}/index.ts`)
})
test('unresolved runtime edges fail closed instead of disappearing from the graph', () => {
  expect(check('unresolved.stories.ts', 'primitive').violations).toEqual([
    `${source('unresolved.stories.ts')} -> ./not-present (unresolved runtime import)`,
  ])
})
test('raw recording JSON cannot bypass domain ownership checks', () => {
  expect(check('recording.stories.ts', 'contract-fixture').violations.join('\n')).toContain('/fixture-graph/kit/recordings/data.json')
})

test.each(['html', 'font', 'raw-code'])('Vite asset imports are leaves: %s', name => {
  expect(check(`${name}.stories.ts`, 'contract-fixture').violations).toEqual([])
})
test.each(['inline', 'no-inline'])('documented Vite %s assets are leaves', query => {
  expect(graph.resolve(`./font.ttf?${query}`, source('font.stories.ts'))).toBe(`${source('font.ttf')}?${query}`)
})
test.each(['missing-asset', 'unknown-query'])('asset imports fail closed: %s', name => {
  expect(check(`${name}.stories.ts`, 'contract-fixture').violations.join('\n')).toContain('unresolved runtime import')
})
test.each(['raw-domain', 'raw-fixture', 'raw-marked'])('raw source cannot hide forbidden fixture data: %s', name => {
  expect(check(`${name}.stories.ts`, 'primitive').violations.join('\n')).toContain('forbidden domain/fixture module')
})

test.each([
  ['allowed', "export { value } from './value'", []],
  ['static', `export { value } from '${policy.kitSource}/index'`, [policy.kitSource + '/index.ts']],
  ['dynamic', `export const value = () => import('${policy.kitSource}/index')`, [policy.kitSource + '/index.ts']],
  ['require', `export const value = require('${policy.kitSource}/index')`, [policy.kitSource + '/index.ts']],
])('workspace package aliases are emitted and traversed: %s', (_kind, workspaceSource, forbidden) => {
  const repository = path.resolve(import.meta.dirname, '../../..')
  const entry = `${repository}/packages/graph-example/contract.stories.ts`
  const workspace = `${repository}/clients/graph-example/index.ts`
  const value = `${repository}/clients/graph-example/value.ts`
  const thirdParty = `${repository}/node_modules/graph-third-party/index.ts`
  const seededSources = {
    ...sources,
    [entry]: "export { value } from '@example/graph-workspace'",
    [`${repository}/node_modules/@example/graph-workspace/package.json`]: JSON.stringify({
      name: '@example/graph-workspace', main: '../../../clients/graph-example/index.ts',
    }),
    [workspace]: `import type { Shape } from '${policy.kitSource}/worlds/types'; import { OtherShape } from '${policy.kitSource}/worlds/types'; import 'graph-third-party'; ${workspaceSource}; export const shape: Shape | OtherShape = { value: 1 }`,
    [value]: 'export const value = 1',
    [`${repository}/node_modules/graph-third-party/package.json`]: JSON.stringify({
      name: 'graph-third-party', main: './index.ts',
    }),
    [thirdParty]: `export { value } from '${policy.kitSource}/index'`,
  }
  // Only the CSF is a root: adding the workspace to rootNames hides TS's external-library emit skip.
  const seededGraph = createTypeScriptGraph([entry], {
    target: ts.ScriptTarget.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    noLib: true,
    types: [],
  }, seededSources)
  expect(seededGraph.resolve('@example/graph-workspace', entry)).toBe(workspace)
  expect(seededGraph.isExternal(thirdParty)).toBe(true)
  expect(seededGraph.read(workspace).imports.map(edge => edge.specifier)).not.toContain(`${policy.kitSource}/worlds/types`)
  const result = checkFixtureGraph(new Map([[entry, new Set<ExemptStoryMode>(['contract-fixture'])]]), seededGraph, policy)
  expect(result.violations).toEqual(forbidden.map(file =>
    `contract-fixture: ${entry} -> ${workspace} -> ${file} (forbidden domain/fixture module)`))
  expect(result.visited).toBe(3)
})
