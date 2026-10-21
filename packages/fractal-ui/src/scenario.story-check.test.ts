/** @vitest-environment jsdom */
/// <reference types="vite/client" />
import { scenarioStoryCheck, withScenario } from '@smalltalk/st3-scenarios/storybook/check'
import { loadWorld } from '@smalltalk/st3-scenarios'
import { useScenarioSlice } from '@smalltalk/st3-scenarios/react'
import { scenarioSections } from './scenario-transcript.story-harness'
import { scenarioExemptionIds } from './scenario-exemption-ids.ts'
import { afterAll, beforeAll, expect, test, vi } from 'vitest'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { composeStory, type Meta, type StoryObj } from '@storybook/react'
import { isExportStory } from 'storybook/internal/csf'
import meta, { AllStates } from './Markdown.scenario.stories.tsx'
import * as systemModule from './SystemState.stories.tsx'
import * as nodePath from 'node:path'
import ts from 'typescript'
import { checkFixtureGraph, createTypeScriptGraph, requiresDomainMarker, type ExemptStoryMode } from './scenario-fixture-graph.ts'
import './scenario-fixture-graph.unit.test.ts'

import { transcriptInitialTailCount, transcriptMountedWindow } from './assistant-ui/composition/Transcript'
import { historyBatchSize } from './assistant-ui/EmbraceScrollViewport'

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

test('the story mounted window uses the real Transcript and viewport mount constants', () => {
  expect(transcriptMountedWindow).toEqual({
    _tag: 'tail', turns: transcriptInitialTailCount, historyBatch: historyBatchSize,
  })
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
  './Transcript.stories.tsx#RawInputDefaultDetailLight': 2936,
  './Transcript.stories.tsx#RawInputWithoutOutput': 2936,
  './Transcript.stories.tsx#RawInputWithoutOutputLight': 2936,
  './Transcript.stories.tsx#NativeToolOutput': 2936,
  './Transcript.stories.tsx#NativeToolOutputLight': 2936,
  './Transcript.stories.tsx#Streaming': 2936,
  './Transcript.stories.tsx#StreamingLight': 2936,
  './Transcript.stories.tsx#Failed': 2936,
  './Transcript.stories.tsx#FailedLight': 2936,
  './Transcript.stories.tsx#Interrupted': 2936,
  './Transcript.stories.tsx#InterruptedLight': 2936,
  './Transcript.stories.tsx#CompletionUnknown': 2936,
  './Transcript.stories.tsx#Loading': 2936,
  './Transcript.stories.tsx#LoadingLight': 2936,
  './Transcript.stories.tsx#CatchingUp': 2936,
  './Transcript.stories.tsx#CatchingUpLight': 2936,
  './Transcript.stories.tsx#Reconnecting': 2936,
  './Transcript.stories.tsx#SyncFailed': 2936,
  './Transcript.stories.tsx#SyncFailedLight': 2936,
  './Transcript.stories.tsx#AllStates': 2936,
  './Transcript.stories.tsx#AllStatesLight': 2936,
  './Transcript.stories.tsx#SettledAnswerMeta': 2936,
  './Transcript.stories.tsx#Unavailable': 2936,
  './Transcript.stories.tsx#UnavailableLight': 2936,
  './Transcript.stories.tsx#UnavailableWithAction': 2936,
  './Transcript.stories.tsx#UnavailableWithActionLight': 2936,
  './Transcript.stories.tsx#UnavailableWithComposer': 2936,
  './Transcript.stories.tsx#UnavailableWithComposerLight': 2936,
  './Transcript.stories.tsx#HasOlderWithLoad': 2936,
  './Transcript.stories.tsx#HasOlderWithLoadLight': 2936,
  './Transcript.stories.tsx#HasOlderWithoutLoad': 2936,
  './Transcript.stories.tsx#HasOlderLight': 2936,
  './Transcript.stories.tsx#PendingSend': 2936,
  './Transcript.stories.tsx#PendingSendLight': 2936,
  './Transcript.stories.tsx#FailedSend': 2936,
  './Transcript.stories.tsx#FailedSendLight': 2936,
  './Transcript.stories.tsx#UnconfirmedSend': 2936,
  './Transcript.stories.tsx#UnconfirmedSendLight': 2936,
  './Transcript.stories.tsx#AdjacentReasoning': 2936,
  './Transcript.stories.tsx#AdjacentReasoningLight': 2936,
  './Transcript.stories.tsx#Settled': 2936,
  './Transcript.stories.tsx#SettledLight': 2936,
  './Transcript.stories.tsx#Expanded': 2936,
  './Transcript.stories.tsx#ExpandedLight': 2936,
  './Transcript.stories.tsx#LongCommandRow': 2936,
  './Transcript.stories.tsx#LongCommandRowLight': 2936,
  './Transcript.stories.tsx#RawInputOnExpand': 2936,
  './Transcript.stories.tsx#RawInputOnExpandLight': 2936,
  './Transcript.stories.tsx#RawInputDefaultDetail': 2936,
  './Transcript.stories.tsx#SendIdentity': 2936,
  './Transcript.stories.tsx#SendIdentityLight': 2936,
  './Transcript.stories.tsx#AppendedItems': 2936,
  './Transcript.stories.tsx#AppendedItemsLight': 2936,
  './Transcript.stories.tsx#StrandedItem': 2936,
  './Transcript.stories.tsx#StrandedItemLight': 2936,
  './Transcript.stories.tsx#FailedRetry': 2936,
  './Transcript.stories.tsx#FailedRetryLight': 2936,
  './Transcript.stories.tsx#MidTurnHistory': 2936,
  './Transcript.stories.tsx#MidTurnHistoryLight': 2936,
  './Transcript.stories.tsx#ReadOnlyTools': 2936,
  './Transcript.stories.tsx#ReadOnlyToolsLight': 2936,
  './Transcript.stories.tsx#ProseOnly': 2936,
  './Transcript.stories.tsx#ProseOnlyLight': 2936,
  './Transcript.stories.tsx#SendRejected': 2936,
  './Transcript.stories.tsx#SendUngranted': 2936,
  './Transcript.stories.tsx#SendInvalid': 2936,
  './Transcript.stories.tsx#SendGenericFailure': 2936,
  './Transcript.stories.tsx#SendStaleFence': 2936,
  './Transcript.stories.tsx#SendSnapshotUnavailable': 2936,
  './Transcript.stories.tsx#SemanticItems': 2936,
  './Transcript.stories.tsx#SemanticItemsLight': 2936,
  './Transcript.stories.tsx#ReadableEmpty': 2936,
  './Transcript.stories.tsx#ReadableEmptyLight': 2936,
  './Transcript.stories.tsx#DefaultEmpty': 2936,
  './Transcript.stories.tsx#SettledAnswerMetaLight': 2936,
  './Transcript.stories.tsx#NoGreenAllStates': 2936,
  './Transcript.stories.tsx#NoGreenAllStatesLight': 2936,
  './Transcript.stories.tsx#MixedContent': 2936,
  './Composer.stories.tsx#Geometry': 2937,
  './Composer.stories.tsx#Idle': 2937,
  './Composer.stories.tsx#IdleLight': 2937,
  './Composer.stories.tsx#TypingMultiLine': 2937,
  './Composer.stories.tsx#Running': 2937,
  './Composer.stories.tsx#QueuedTwice': 2937,
  './Composer.stories.tsx#FailedSendRestored': 2937,
  './Composer.stories.tsx#OfflineDraft': 2937,
  './Composer.stories.tsx#ImageAttached': 2937,
  './Composer.stories.tsx#AllStates': 2937,
  './Composer.stories.tsx#AllStatesLight': 2937,
  './Composer.stories.tsx#TargetTooltips': 2937,
  './Composer.stories.tsx#TargetTooltipsLight': 2937,
  './Composer.stories.tsx#MentionPopup': 2937,
  './Composer.stories.tsx#MentionPopupLight': 2937,
  './Composer.stories.tsx#SlashCommands': 2937,
  './Composer.stories.tsx#ImeComposition': 2937,
  './Composer.stories.tsx#ShiftEnterNewline': 2937,
  './Composer.stories.tsx#DurableDraft': 2937,
  './Composer.stories.tsx#EffortSupported': 2937,
  './Composer.stories.tsx#EffortUnsupported': 2937,
  './Composer.stories.tsx#EffortUnknown': 2937,
  './Composer.stories.tsx#EffortAllStates': 2937,
  './Composer.stories.tsx#FooterWidthBudget': 2937,
  './Composer.stories.tsx#FooterWidthBudgetLight': 2937,
  './Composer.stories.tsx#LayoutShapes': 2937,
  './Composer.stories.tsx#KitLabelResize': 2937,
  './Composer.stories.tsx#PlainTextAdapter': 2937,
  './Workbench.stories.tsx#Dark': 2936,
  './Workbench.stories.tsx#Light': 2936,
  './Workbench.stories.tsx#OnePane': 2936,
  './Workbench.stories.tsx#IdleSample': 2936,
  './Workbench.stories.tsx#RunningQueueAndSteer': 2936,
  './Workbench.stories.tsx#TerminalOpen': 2936,
  './Workbench.stories.tsx#Dragging': 2936,
  './Workbench.stories.tsx#NestedRatios': 2936,
  './Workbench.stories.tsx#ThreadTabs': 2936,
  './Workbench.stories.tsx#Empty': 2936,
  './Workbench.stories.tsx#Loading': 2936,
  './Workbench.stories.tsx#Stale': 2936,
  './Workbench.stories.tsx#Failed': 2936,
  './Workbench.stories.tsx#Unavailable': 2936,
  './Workbench.stories.tsx#SplitBelow': 2936,
  './Workbench.stories.tsx#DragToSplit': 2936,
  './Workbench.stories.tsx#DragToSplitSingleGroup': 2936,
  './Workbench.stories.tsx#RendererOverride': 2936,
  './Workbench.stories.tsx#OptionalSnapshotFields': 2936,
  './Workbench.stories.tsx#KeyboardCloseAndSnapshot': 2936,
  './Workbench.stories.tsx#KeyboardTabsAndTerminal': 2936,
  './Workbench.stories.tsx#KeyboardSplitPreservesHost': 2936,
  './Workbench.stories.tsx#HostToolReveal': 2936,
  './Workbench.stories.tsx#AllStates': 2936,
  './Workbench.stories.tsx#AllStatesLight': 2936,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#StickyChain': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#PushOutBoundary': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#DepthCap': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#AccessibleRows': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#PinnedClick': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#FocusAndReveal': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#VisibleFocusDoesNotJump': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#CollapseFilterReorder': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#ColdRevealBeforeMeasurement': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#RevealUnvisitedAncestors': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#Light440': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#Dark1440': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#Light1440': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#Modal240': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#Modal240Light': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#ChevronIcons': 3355,
  './assistant-ui/sidebar/SidebarTree.stories.tsx#AllStates': 3355,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#HoverCardKnown': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#HoverCardUnknown': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#TimeCohort': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#FrozenWorkingVersusIdle': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#QuickOpenAction': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#AllStates': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#DensitiesAndChildren': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#PhoneTouchTargets': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#PhoneFocusNoHover': 2906,
  './assistant-ui/sidebar/SidebarAgentRow.stories.tsx#ThemedOverlaysAndFocus': 2906,
  './stories/milestone/RedesignMilestone.stories.tsx#Slice': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#SliceLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#Loading': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#LoadingLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#EmptyRoster': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#EmptyRosterLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#EmptyThread': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#EmptyThreadLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#NoMatch': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#NoMatchLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#RosterError': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#RosterErrorLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#ThreadUnavailable': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#ThreadUnavailableLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#Stale': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#StaleLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#Navigation': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#NavigationLight': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#HideChanges': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#NavigationKeyboard': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#ErrorRecovery': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#ThreadRecovery': 3355,
  './stories/milestone/RedesignMilestone.stories.tsx#Decisions': 3355,
}

interface StoryModule {
  readonly default: Meta
  readonly [exportName: string]: unknown
}
const storyModules = import.meta.glob<StoryModule>('./**/*.stories.tsx', { eager: true })
const isStory = (value: unknown): value is StoryObj => typeof value === 'object' && value !== null
const undeclared: string[] = []
const exemptions: string[] = []
const exemptFiles = new Map<string, Set<ExemptStoryMode>>()
const sliceFiles = new Set<string>()

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
      undeclared.push(id)
      test(`${id} has pinned, PR-owned undeclared mode debt`, () => {
        expect(deferredStoryModes[id], 'Declare a scenario mode instead of adding unowned debt').toBeDefined()
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
    })
  }
}

test('exempt CSF runtime import graphs cannot reach domain fixture roots', () => {
  const started = performance.now()
  expect([...exemptFiles.keys()].filter(file => sliceFiles.has(file)), 'Slice and exempt stories must use separate CSF files').toEqual([])
  const uiSource = nodePath.resolve(process.cwd(), 'src')
  const kitRoot = nodePath.resolve(uiSource, '../../../clients/typescript/st3-scenarios')
  const contractDataModules = [
    ...ts.sys.readDirectory(uiSource, ['.ts', '.tsx'], ['**/*.test.ts', '**/*.test.tsx', '**/*.stories.tsx'])
      .filter(file => /fixture/i.test(nodePath.basename(file)) && nodePath.basename(file) !== 'scenario-fixture-graph.ts'),
    ...['Families.tsx', 'Workshop.tsx', 'markdown.story-canvas.tsx'].map(file => nodePath.resolve(uiSource, file)),
  ]
  const policy = {
    uiSource: uiSource.replace(/\/$/, ''),
    kitSource: `${kitRoot}/src`,
    projectionHarness: nodePath.resolve(uiSource, 'scenario-transcript.story-harness.ts'),
    contractDataModules,
  }
  const allKitSources = ts.sys.readDirectory(kitRoot, ['.ts', '.tsx'], ['**/node_modules/**', '**/*.test.ts', '**/*.test.tsx'])
  const markerRoots = [...allKitSources, policy.projectionHarness].filter(file => requiresDomainMarker(file, policy))
  const entries = new Map([...exemptFiles].map(([file, modes]) => [nodePath.resolve(uiSource, file), modes]))
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

test('scenario exemptions stay pinned and their reasons are printed by each check', () => {
  expect(exemptions).toHaveLength(326)
  expect(exemptions.sort()).toEqual(scenarioExemptionIds)
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

test('phone whole-thread projection preserves turns while leaving mounting to Transcript', () => {
  const slice = loadWorld('huge').slices.conversation
  const bounded = scenarioSections(slice).sections
  const whole = scenarioSections(slice, 'whole-thread').sections
  expect(whole).toHaveLength(slice.state.threads.length)
  expect(whole.flatMap(section => section.turns.map(turn => turn.id))).toEqual(bounded.flatMap(section => section.turns.map(turn => turn.id)))
  expect(whole.some(section => section.turns.length > transcriptInitialTailCount + historyBatchSize)).toBe(true)
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
