/** @vitest-environment jsdom */
/// <reference types="vite/client" />
import { scenarioStoryCheck, withScenario } from '@fractal/scenarios/storybook/check'
import { foldSlice, loadWorld } from '@fractal/scenarios'
import { useScenarioSlice } from '@fractal/scenarios/react'
import { scenarioSections, type TranscriptSection } from './scenario-transcript.story-harness'
import { afterAll, beforeAll, expect, test, vi } from 'vitest'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { composeStory, type Meta, type StoryObj } from '@storybook/react'
import { isExportStory } from 'storybook/internal/csf'
import meta, { AllStates } from './Markdown.scenario.stories.tsx'
import * as systemModule from './SystemState.stories.tsx'
import * as nodePath from 'node:path'
import { globSync, lstatSync, mkdirSync, mkdtempSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import ts from 'typescript'
import { checkFixtureGraph, createTypeScriptGraph, requiresDomainMarker, type ExemptStoryMode } from './scenario-fixture-graph.ts'
import './scenario-fixture-graph.unit.test.ts'
import { storyGlobs } from '../.storybook/stories.ts'
import storybookConfig from '../.storybook/main.ts'

import { transcriptInitialSourceRowCount, transcriptMountedWindow, type TranscriptTurn } from './assistant-ui/composition/Transcript'

// jsdom has no layout engine. This test observes domain reads, not resize geometry;
// the real browser plays remain the proof for observer-driven layout behavior.
class ReadCheckResizeObserver {
  readonly targets = new Set<Element>()
  observe(target: Element): void { this.targets.add(target) }
  unobserve(target: Element): void { this.targets.delete(target) }
  disconnect(): void { this.targets.clear() }
}
const originalFonts = Object.getOwnPropertyDescriptor(document, 'fonts')
beforeAll(() => {
  vi.stubGlobal('ResizeObserver', ReadCheckResizeObserver)
  vi.stubGlobal('matchMedia', (media: string) => ({
    matches: false, media, onchange: null,
    addEventListener() {}, removeEventListener() {},
    addListener() {}, removeListener() {}, dispatchEvent: () => true,
  }))
  Object.defineProperty(document, 'fonts', { configurable: true, value: Object.assign(new EventTarget(), { ready: Promise.resolve() }) })
})
afterAll(() => {
  vi.unstubAllGlobals()
  if (originalFonts === undefined) Reflect.deleteProperty(document, 'fonts')
  else Object.defineProperty(document, 'fonts', originalFonts)
})

test('browser exemption checks propagate forbidden slice reads from a client root', () => {
  const ReadsDomain = () => {
    useScenarioSlice('roster')
    return React.createElement('span')
  }
  expect(() => scenarioStoryCheck({
    render: ReadsDomain,
    parameters: { scenario: { _tag: 'primitive', reason: 'Planted forbidden domain read' } },
  }, {})).toThrow(/exempt|exemption/)
})

test('an error boundary cannot hide an exempt story domain read', () => {
  class CatchesRenderErrors extends React.Component<React.PropsWithChildren, { failed: boolean }> {
    state = { failed: false }
    static getDerivedStateFromError(): { failed: boolean } { return { failed: true } }
    render(): React.ReactNode {
      return this.state.failed ? React.createElement('span', {}, 'Caught render error') : this.props.children
    }
  }
  const ReadsDomain = () => {
    useScenarioSlice('roster')
    return React.createElement('span')
  }
  expect(() => scenarioStoryCheck({
    render: () => React.createElement(CatchesRenderErrors, {}, React.createElement(ReadsDomain)),
    parameters: { scenario: { _tag: 'primitive', reason: 'Planted swallowed domain read' } },
  }, {})).toThrow(/exempt|forbidden/)
})

test('the story mounted window uses the real Transcript initial source-row bound', () => {
  expect(transcriptMountedWindow).toEqual({ _tag: 'sourceRows', rows: transcriptInitialSourceRowCount })
})
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

// Exact export IDs remain undeclared while their owning PRs block these files.
// Removing a story or declaring its mode must remove its entry; new debt fails.
const deferredStoryModes: Readonly<Record<string, number>> = {
}

// Reviewed shrink-only pin owned by the app, separate from the blocked-PR ratchet.
const deferredAppStoryModes: Readonly<Record<string, 'fractal-web-app'>> = {
}

interface StoryModule {
  readonly default: Meta
  readonly [exportName: string]: unknown
}
// A lazy repository-wide loader pool is not the inventory: only configured CSFs
// are imported. Configured files outside this supported pool fail explicitly.
const storyLoaders = import.meta.glob<StoryModule>('../../../**/*.stories.{ts,tsx,js,jsx,mjs,mts,cjs,cts}')
const uiSource = nodePath.resolve(process.cwd(), 'src')
const loadersByFile = Object.fromEntries(Object.entries(storyLoaders).map(([file, loader]) => [nodePath.resolve(uiSource, file), loader]))
const storybookDirectory = nodePath.resolve(uiSource, '../.storybook')
// Do not filter by extension here: every configured file needs a loader or an
// explicit failure, including newly configured CSF extensions and glob braces.
const configuredStoryFiles = (globs: string[], directory = storybookDirectory) => {
  const unsupported = (glob: string): never => {
    throw new Error(`Cannot derive static Storybook inventory roots for configured pattern: ${glob}`)
  }
  const expandBraces = (glob: string): string[] => {
    const start = glob.indexOf('{')
    if (start === -1) return glob.includes('}') ? unsupported(glob) : [glob]
    const alternatives: string[] = []
    let depth = 0
    let part = start + 1
    for (let index = start; index < glob.length; index++) {
      const character = glob[index]
      if (character === '{') depth++
      if (character === '}') {
        depth--
        if (depth === 0) {
          if (alternatives.length === 0) return unsupported(glob)
          alternatives.push(glob.slice(part, index))
          return alternatives.flatMap(alternative => expandBraces(glob.slice(0, start) + alternative + glob.slice(index + 1)))
        }
      }
      if (character === ',' && depth === 1) {
        alternatives.push(glob.slice(part, index))
        part = index + 1
      }
    }
    return unsupported(glob)
  }
  const rejectSymlink = (path: string) => {
    const stat = lstatSync(path)
    if (stat.isSymbolicLink()) throw new Error(`Symlink in configured Storybook inventory: ${path}`)
    return stat
  }
  const walk = (path: string) => {
    if (!rejectSymlink(path).isDirectory()) return
    for (const name of readdirSync(path)) {
      if (name !== 'node_modules') walk(nodePath.join(path, name))
    }
  }
  const files = new Set<string>()
  for (const glob of globs) {
    const expandedGlobs = expandBraces(glob)
    if (glob.includes('node_modules') || expandedGlobs.some(expanded => expanded.includes('node_modules'))) {
      throw new Error(`Unsupported node_modules segment in configured Storybook pattern: ${glob}; extend the scenario inventory check before loading dependency stories`)
    }
    const roots = new Set(expandedGlobs.map(expanded => {
      if (/[()!\\]/.test(expanded)) return unsupported(glob)
      const segments = expanded.split('/')
      const magic = segments.findIndex(segment => /[*?[]/.test(segment))
      if (magic === 0 || (magic !== -1 && segments.slice(magic).includes('..'))) return unsupported(glob)
      return nodePath.resolve(directory, magic === -1 ? nodePath.dirname(expanded) : segments.slice(0, magic).join('/'))
    }))
    for (const root of roots) {
      // Inspect static ancestors first, without following earlier path components.
      const ancestors: string[] = []
      for (let parent = nodePath.dirname(root); ; parent = nodePath.dirname(parent)) {
        ancestors.push(parent)
        if (parent === nodePath.dirname(parent)) break
      }
      for (const ancestor of ancestors.reverse()) rejectSymlink(ancestor)
      walk(root)
    }
    for (const file of globSync(glob, { cwd: directory })) {
      // Glob exclusions miss leading ../ patterns; match dependency segments like the walk.
      if (!file.split(nodePath.sep).includes('node_modules')) files.add(nodePath.resolve(directory, file))
    }
  }
  return [...files]
}
const configuredFiles = configuredStoryFiles(storyGlobs)
const storyModules: Record<string, StoryModule> = {}
for (const file of configuredFiles) {
  const path = `./${nodePath.relative(uiSource, file)}`
  const loader = loadersByFile[file]
  if (loader === undefined) throw new Error(`No CSF loader for configured Storybook file: ${file}`)
  storyModules[path] = await loader()
}

test('the scenario inventory covers exactly the CSFs loaded by kit Storybook', () => {
  expect(storybookConfig.stories).toEqual(storyGlobs)
  expect(Object.keys(storyModules).map(file => nodePath.resolve(uiSource, file)).sort()).toEqual(configuredFiles.sort())
  expect(configuredFiles.length).toBeGreaterThan(0)
})

test('configured inventory rejects directory and file symlinks without following them', () => {
  const directory = mkdtempSync(nodePath.join(tmpdir(), 'scenario-inventory-'))
  try {
    const source = nodePath.join(directory, 'src')
    mkdirSync(source)
    const story = nodePath.join(source, 'Example.stories.tsx')
    writeFileSync(story, '')
    const globs = ['src/**/*.stories.tsx']
    expect(configuredStoryFiles(globs, directory)).toEqual([story])
    const directoryLink = nodePath.join(source, 'linked')
    symlinkSync(directory, directoryLink, 'dir')
    expect(() => configuredStoryFiles(globs, directory)).toThrow(`Symlink in configured Storybook inventory: ${directoryLink}`)
    expect(() => configuredStoryFiles(['src/linked/**/*.stories.tsx'], directory)).toThrow(`Symlink in configured Storybook inventory: ${directoryLink}`)
    expect(() => configuredStoryFiles(['src/linked/src/**/*.stories.tsx'], directory)).toThrow(`Symlink in configured Storybook inventory: ${directoryLink}`)
    rmSync(directoryLink)
    const fileLink = nodePath.join(source, 'Linked.stories.tsx')
    symlinkSync(story, fileLink, 'file')
    expect(() => configuredStoryFiles(globs, directory)).toThrow(`Symlink in configured Storybook inventory: ${fileLink}`)
    expect(() => configuredStoryFiles(['src/Linked.stories.tsx'], directory)).toThrow(`Symlink in configured Storybook inventory: ${fileLink}`)
    rmSync(fileLink)
    const dependencies = nodePath.join(source, 'node_modules')
    mkdirSync(dependencies)
    symlinkSync(directory, nodePath.join(dependencies, 'ignored'), 'dir')
    expect(configuredStoryFiles(globs, directory)).toEqual([story])
    expect(() => configuredStoryFiles(['src/node_modules/**/*.stories.tsx'], directory))
      .toThrow('Unsupported node_modules segment in configured Storybook pattern: src/node_modules/**/*.stories.tsx; extend the scenario inventory check')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('configured inventory excludes dependency stories with leading relative prefixes', () => {
  const directory = mkdtempSync(nodePath.join(tmpdir(), 'scenario-inventory-dependencies-'))
  try {
    const config = nodePath.join(directory, '.storybook')
    const source = nodePath.join(directory, 'src')
    mkdirSync(config)
    mkdirSync(nodePath.join(source, 'a/node_modules/b'), { recursive: true })
    mkdirSync(nodePath.join(source, 'a/not_node_modules/b'), { recursive: true })
    const story = nodePath.join(source, 'a/Example.stories.tsx')
    const similarNameStory = nodePath.join(source, 'a/not_node_modules/b/Example.stories.tsx')
    writeFileSync(story, '')
    writeFileSync(similarNameStory, '')
    writeFileSync(nodePath.join(source, 'a/node_modules/b/Dependency.stories.tsx'), '')
    for (const glob of [
      '../src/**/*.stories.tsx',
      './../src/**/*.stories.tsx',
      '../.storybook/../src/**/*.stories.{ts,tsx}',
      '../src/{a,./a}/**/*.stories.tsx',
    ]) {
      expect(configuredStoryFiles([glob], config).sort(), glob).toEqual([story, similarNameStory].sort())
    }
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('configured inventory expands nested brace roots before inspecting symlinks', () => {
  const directory = mkdtempSync(nodePath.join(tmpdir(), 'scenario-inventory-braces-'))
  try {
    const config = nodePath.join(directory, 'config/.storybook')
    mkdirSync(config, { recursive: true })
    mkdirSync(nodePath.join(directory, 'config/src/nested'), { recursive: true })
    const outside = nodePath.join(directory, 'outside')
    mkdirSync(outside)
    const story = nodePath.join(outside, 'Example.stories.tsx')
    writeFileSync(story, '')
    const globs = ['../src/{nested,{../../outside,nested}}/**/*.stories.tsx']
    expect(configuredStoryFiles(globs, config)).toEqual([story])
    const link = nodePath.join(outside, 'linked')
    symlinkSync(directory, link, 'dir')
    expect(() => configuredStoryFiles(globs, config)).toThrow(`Symlink in configured Storybook inventory: ${link}`)
    expect(() => configuredStoryFiles(['{../src,../../outside}/**/*.stories.tsx'], config))
      .toThrow(`Symlink in configured Storybook inventory: ${link}`)
    for (const glob of ['@(src|outside)/**/*.stories.tsx', '*/**/*.stories.tsx', '../src/{1..3}/**/*.stories.tsx']) {
      expect(() => configuredStoryFiles([glob], config)).toThrow(`Cannot derive static Storybook inventory roots for configured pattern: ${glob}`)
    }
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('configured .stories.ts CSFs are discovered and checked, including brace globs', async () => {
  const files = configuredStoryFiles(['../src/scenario-story-check-fixtures/*.stories.{ts,tsx}'])
  expect(files).toEqual([nodePath.resolve(uiSource, 'scenario-story-check-fixtures/Primitive.stories.ts')])
  const loader = loadersByFile[files[0]!]
  expect(loader).toBeDefined()
  if (loader === undefined) throw new Error('Configured TypeScript CSF has no loader')
  const module = await loader()
  const story = module['AllStates']
  expect(isStory(story)).toBe(true)
  if (!isStory(story)) throw new Error('TypeScript CSF fixture has no object story')
  expect(scenarioStoryCheck(story, module.default)._tag).toBe('Passed')
})
const isStory = (value: unknown): value is StoryObj => typeof value === 'object' && value !== null
const undeclared: string[] = []
const undeclaredApp: string[] = []
const exemptions: string[] = []
const exemptFiles = new Map<string, Set<ExemptStoryMode>>()
const sliceFiles = new Set<string>()

// react-stately renders every row in jsdom (no layout); SidebarTree's observed
// CI maximum is 7.1 s, so 15 s allows about 2x. Other stories take at most ~1.9 s;
// a global increase would hide regressions outside this expanded-tree fixture.
const sidebarTreeValidationTimeoutMs = 15_000

for (const [path, module] of Object.entries(storyModules)) {
  const { default: storyMeta, ...exports } = module
  for (const [name, story] of Object.entries(exports)) {
    if (!isExportStory(name, storyMeta)) continue
    const id = `${path}#${name}`
    if (!isStory(story)) {
      test(`${id} uses an object story so its scenario declaration can be checked`, () => {
        expect(isStory(story)).toBe(true)
      })
      continue
    }
    const declaration: unknown = story.parameters?.scenario ?? storyMeta.parameters?.scenario
    const tag = typeof declaration === 'object' && declaration !== null && '_tag' in declaration ? declaration._tag : undefined
    if (tag !== 'slices' && tag !== 'contract-fixture' && tag !== 'primitive') {
      const appStory = path.startsWith('./../../../apps/')
      if (appStory) undeclaredApp.push(id)
      else undeclared.push(id)
      test(`${id} has pinned, owned undeclared mode debt`, () => {
        expect(appStory ? deferredAppStoryModes[id] : deferredStoryModes[id], 'Declare a scenario mode instead of adding unowned debt').toBeDefined()
        if (appStory) expect(declaration, 'Deferred app stories must have no scenario declaration').toBeUndefined()
      })
      continue
    }
    if (tag !== 'slices') {
      exemptions.push(`${id}:${tag}`)
      const modes = exemptFiles.get(path) ?? new Set<ExemptStoryMode>()
      modes.add(tag)
      exemptFiles.set(path, modes)
    } else sliceFiles.add(path)
    test(`${id} validates its scenario mode`, () => {
      const result = scenarioStoryCheck(story, storyMeta)
      expect(result.failures).toEqual([])
      expect(result._tag).toBe('Passed')
      for (const exemption of result.exemptions) process.stdout.write(`${id}: ${exemption._tag} — ${exemption.reason}\n`)
      expect(result.exemptions).toHaveLength(tag === 'slices' ? 0 : 1)
    }, path === './assistant-ui/sidebar/SidebarTree.stories.tsx' ? sidebarTreeValidationTimeoutMs : undefined)
  }
}

const fixtureGraphEntries = (
  exempt: ReadonlyMap<string, ReadonlySet<ExemptStoryMode>>,
  deferredAppIds: readonly string[],
) => {
  const entries = new Map([...exempt].map(([file, modes]) => [nodePath.resolve(uiSource, file), modes]))
  for (const id of deferredAppIds) {
    const file = id.slice(0, id.lastIndexOf('#'))
    if (!exempt.has(file)) entries.set(nodePath.resolve(uiSource, file), new Set<ExemptStoryMode>())
  }
  return entries
}

test('declared slices in an app-like CSF pass normal rules without the forbidden-domain graph', () => {
  const appPath = './../../../apps/example/src/Slices.stories.tsx'
  expect(scenarioStoryCheck(AllStates, meta)._tag).toBe('Passed')
  expect(fixtureGraphEntries(new Map(), []).has(nodePath.resolve(uiSource, appPath))).toBe(false)
  expect(fixtureGraphEntries(new Map(), [`${appPath}#Deferred`]).has(nodePath.resolve(uiSource, appPath))).toBe(true)
  expect(fixtureGraphEntries(new Map([[appPath, new Set<ExemptStoryMode>(['primitive'])]]), []).has(nodePath.resolve(uiSource, appPath))).toBe(true)
})

test('exempt CSF runtime import graphs cannot reach domain fixture roots', () => {
  const started = performance.now()
  expect([...exemptFiles.keys()].filter(file => sliceFiles.has(file)), 'Slice and exempt stories must use separate CSF files').toEqual([])
  const uiSource = nodePath.resolve(process.cwd(), 'src')
  const kitRoot = nodePath.resolve(uiSource, '../../../clients/typescript/fractal-scenarios')
  const contractDataModules = [
    ...ts.sys.readDirectory(uiSource, ['.ts', '.tsx'], ['**/*.test.ts', '**/*.test.tsx', '**/*.stories.tsx'])
      .filter(file => /fixture/i.test(nodePath.basename(file)) && nodePath.basename(file) !== 'scenario-fixture-graph.ts'),
    ...['Families.tsx', 'markdown.story-canvas.tsx'].map(file => nodePath.resolve(uiSource, file)),
  ]
  const policy = {
    uiSource: uiSource.replace(/\/$/, ''),
    kitSource: `${kitRoot}/src`,
    projectionHarness: nodePath.resolve(uiSource, 'scenario-transcript.story-harness.ts'),
    contractDataModules,
  }
  const allKitSources = ts.sys.readDirectory(kitRoot, ['.ts', '.tsx'], ['**/node_modules/**', '**/*.test.ts', '**/*.test.tsx'])
  const markerRoots = [...allKitSources, policy.projectionHarness].filter(file => requiresDomainMarker(file, policy))
  // Only exemptions and actual undeclared app debt forbid domain fixture roots.
  // Declared slice CSFs consume the kit normally, regardless of their location.
  const entries = fixtureGraphEntries(exemptFiles, undeclaredApp)
  const configPath = nodePath.resolve(uiSource, '../tsconfig.json')
  const config = ts.readConfigFile(configPath, ts.sys.readFile)
  if (config.error !== undefined) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, '\n'))
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, nodePath.dirname(configPath))
  expect(parsed.errors.map(error => ts.flattenDiagnosticMessageText(error.messageText, '\n'))).toEqual([])
  const graph = createTypeScriptGraph([...entries.keys(), ...markerRoots], parsed.options)
  expect(markerRoots.filter(file => !graph.hasTypedMarker(file)), 'Every domain root exports the public unique-symbol marker').toEqual([])
  const result = checkFixtureGraph(entries, graph, policy)
  process.stdout.write('Allowed fractal-ui contract-data modules and exempt importers (transitive):\n')
  for (const [file, importers] of [...result.contractData].sort(([left], [right]) => left.localeCompare(right))) {
    process.stdout.write(`${file}\n${importers.length === 0 ? '  (no exempt importers)' : importers.map(importer => `  ${importer}`).join('\n')}\n`)
  }
  process.stdout.write(`Fixture graph: ${entries.size} exempt CSF files, ${result.visited} runtime modules, ${markerRoots.length} marker roots, ${Math.round(performance.now() - started)}ms\n`)
  expect(result.violations, result.violations.join('\n')).toEqual([])
}, 60_000)

test('undeclared story modes match the exact blocked-PR ratchet (no growth or stale entries)', () => {
  expect(undeclared.sort()).toEqual(Object.keys(deferredStoryModes).sort())
  for (const id of undeclared) process.stdout.write(`${id}: undeclared — blocked by PR #${deferredStoryModes[id]}\n`)
})

test('app mode debt matches its reviewed shrink-only pin', () => {
  expect(Object.keys(deferredAppStoryModes)).toHaveLength(0)
  expect(undeclaredApp.sort()).toEqual(Object.keys(deferredAppStoryModes).sort())
  for (const id of undeclaredApp) process.stdout.write(`${id}: undeclared — owned by ${deferredAppStoryModes[id]}\n`)
})

const isExemptionList = (value: unknown): value is readonly string[] => Array.isArray(value) && value.every(entry => typeof entry === 'string')

// Each stories file reviews its own exemptions, so parallel PRs only meet when they edit the same file.
test('scenario exemptions match the reviewed list exported by each stories file and their reasons are printed by each check', () => {
  const listed: string[] = []
  for (const [path, module] of Object.entries(storyModules)) {
    const { default: storyMeta, scenarioExemptions } = module
    if (scenarioExemptions === undefined) continue
    expect(isExemptionList(scenarioExemptions), `${path} scenarioExemptions must be a list of export:mode ids`).toBe(true)
    if (!isExemptionList(scenarioExemptions)) continue
    expect(isExportStory('scenarioExemptions', storyMeta), `${path} must list scenarioExemptions in its meta excludeStories`).toBe(false)
    expect(scenarioExemptions.length, `${path} exports an empty scenarioExemptions list`).toBeGreaterThan(0)
    expect(scenarioExemptions, `${path} scenarioExemptions must be sorted with no id listed twice`).toEqual([...new Set(scenarioExemptions)].sort())
    for (const entry of scenarioExemptions) {
      const name = entry.slice(0, entry.lastIndexOf(':'))
      expect(isExportStory(name, storyMeta) && isStory(module[name]), `${path}#${entry} names no story`).toBe(true)
      listed.push(`${path}#${entry}`)
    }
  }
  expect(exemptions.sort()).toEqual(listed.sort())
})

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

test('scenario transcript projects native reasoning, generic arguments and structured tool results', () => {
  const turns = scenarioSections(loadWorld('transcript-families').slices.conversation).sections.flatMap(section => section.turns)
  const items = turns.flatMap(turn => turn.items)
  expect(items).toContainEqual(expect.objectContaining({
    _tag: 'Reasoning', id: 'timeline-entry/scenario-transcript-families-builder-3',
    text: 'I will compare the release scope and report totals before exporting.', streaming: false,
  }))
  const call = items.find(item => item._tag === 'ToolCall' && item.callId === 'inspect-report')
  expect(call).toMatchObject({
    _tag: 'ToolCall', name: 'catalog.compare_release', status: 'success',
    input: { releases: ['atlas-7', 'atlas-8'], include: { totals: true, sections: ['runtime', 'client'] }, limit: 3 },
    result: { mediaType: 'application/json', content: { release: 'atlas-8', totals: { passed: 18, failed: 0 }, sections: [{ name: 'runtime', ready: true }, { name: 'client', ready: true }] } },
  })
  expect(turns.flatMap(turn => turn.work.calls)).toContainEqual(expect.objectContaining({ kind: 'read', title: 'catalog.compare_release', outputLanguage: 'json' }))
})

test('scenario transcript retains semantic failure evidence and interrupts unresolved terminal calls', () => {
  const turns = scenarioSections(loadWorld('transcript-families').slices.conversation).sections.flatMap(section => section.turns)
  for (const status of ['failed', 'cancelled']) {
    const turn = turns.find(turn => turn.items.some(item => item._tag === 'Status' && item.status === status))
    expect(turn).toBeDefined()
    expect(turn?.work).toMatchObject({ running: false, failed: status === 'failed', interrupted: status === 'cancelled', calls: [expect.objectContaining({ status: 'interrupted' })] })
    expect(turn?.items).toContainEqual(expect.objectContaining({ _tag: 'ToolCall', status: 'interrupted' }))
    expect(turn?.items).toContainEqual(expect.objectContaining({ _tag: 'Status', status, detail: expect.any(String) }))
  }
  const failed = turns.find(turn => turn.work.failed)
  expect(failed?.work.failureNote).toBe('Export worker lost; report totals retained, but no export result was received.')
  expect(failed?.items).toContainEqual(expect.objectContaining({ _tag: 'Notice', kind: 'error', text: 'The export worker disconnected before returning a result.' }))
})

test('scenario transcript replaces a streaming revision with its final native content', () => {
  const slice = loadWorld('transcript-families').slices.conversation
  const before = scenarioSections(slice).sections.flatMap(section => section.turns).flatMap(turn => turn.items)
  const after = scenarioSections({ ...slice, state: foldSlice(slice, 2_000).state }).sections.flatMap(section => section.turns).flatMap(turn => turn.items)
  const partial = before.find(item => item._tag === 'Text' && item.streaming)
  expect(partial).toMatchObject({ _tag: 'Text', streaming: true, text: 'The report totals are retained; checking the handoff summary.' })
  expect(after.filter(item => item.id === partial?.id)).toHaveLength(1)
  expect(after.find(item => item.id === partial?.id)).toMatchObject({ _tag: 'Text', streaming: false, text: 'All 18 checks passed. The export failed and its retry was cancelled; hand off the report totals, not an export artifact.' })
  expect(after).toContainEqual(expect.objectContaining({ _tag: 'Status', status: 'completed', detail: 'Handoff summary complete; no export claimed.' }))
})

test('phone whole-thread projection preserves source rows while leaving mounting to Transcript', () => {
  const slice = loadWorld('huge').slices.conversation
  const sourceIds = (sections: readonly TranscriptSection[]) => sections.flatMap(section => section.turns.flatMap(turn => [...(turn.prompt === undefined ? [] : [turn.prompt.id]), ...turn.items.map(item => item.id)]))
  const sourceRowCount = (turns: readonly TranscriptTurn[]) => turns.reduce((rows, turn) => rows + turn.items.length + (turn.prompt === undefined ? 0 : 1), 0)
  const bounded = scenarioSections(slice).sections
  const whole = scenarioSections(slice, 'whole-thread').sections
  expect(whole).toHaveLength(slice.state.threads.length)
  expect(sourceIds(whole)).toEqual(sourceIds(bounded))
  // Bounded sections mount whole; whole threads exceed the initial bound, so Transcript defers their history.
  expect(bounded.every(section => sourceRowCount(section.turns) <= transcriptInitialSourceRowCount)).toBe(true)
  expect(whole.some(section => sourceRowCount(section.turns) > transcriptInitialSourceRowCount)).toBe(true)
})

test('connection geometry header and bounded history belong to the same thread', () => {
  const { default: systemMeta, ...systemStories } = systemModule
  const Composed = composeStory({ ...systemStories.BandKeepsFollow, parameters: { scenario: { _tag: 'slices', slices: ['conversation'], world: 'fleet-mid-refactor' } } }, systemMeta, { decorators: [withScenario] })
  const markup = renderToStaticMarkup(React.createElement(Composed))
  const world = loadWorld('fleet-mid-refactor').with({ conversation: 'long' })
  const agent = world.slices.conversation.state.threads[0]!.agent
  const rowIds = [...markup.matchAll(/data-item-id="([^"]+)"/g)].map(match => match[1]!)
  expect(rowIds).toHaveLength(48)
  expect(rowIds.every(id => id.startsWith(`${agent}/`))).toBe(true)
  expect(markup).toContain(`>${agent}</`)
})
