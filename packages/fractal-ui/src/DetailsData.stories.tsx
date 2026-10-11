import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, waitFor, within } from 'storybook/test'
import { DetailsPane, type DetailsPaneProps } from './assistant-ui/details/DetailsPane'
import { detailsStoryInputs } from './assistant-ui/details/story-fixtures'
import { fullTraceFixture } from './assistant-ui/details/trace/fixture-provider'
import type { TraceFact, TraceSeries } from './assistant-ui/details/trace-series'
import { Workbench } from './assistant-ui/workbench/Workbench'
import { group } from './assistant-ui/workbench/workbench-model'
import { detailsWorkbenchResources as workbenchResources } from './assistant-ui/details/story-fixtures'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { surfaceVars as surface } from './assistant-ui/composition-tokens.stylex'

const now = 1800000000000
const known = <A,>(value: A): TraceFact<A> => ({ _tag: 'Known', value, source: 'st', asOfMs: now })
const unavailable = { _tag: 'Unknown', reason: 'not-attributed' } as const
const window = known({ _tag: 'Interval', fromMs: now - 3_600_000, toMs: now } as const)
const native: TraceSeries = {
  ...fullTraceFixture,
  coverage: { _tag: 'Complete' },
  meters: {
    ...fullTraceFixture.meters,
    spend: {
      ...fullTraceFixture.meters.spend,
      usdScope: { _tag: 'Self' }, usdWindow: window,
      totalUsdCoverage:{_tag:'Complete'},mainUsdScope:{_tag:'Self'},mainUsdWindow:window,mainUsdCoverage:{_tag:'Complete'},
      subagentUsdScope:{_tag:'IncludingSubagents'},subagentUsdWindow:window,subagentUsdCoverage:{_tag:'Unavailable',reason:'not-attributed'},
      tokensScope: { _tag: 'AgentIncarnations' }, tokensWindow: unavailable,
      totalUsd: known(12.34), mainUsd: known(12.34), subagentUsd: unavailable,
      tokens: known(480035214), split: unavailable,
    },
  },
  series: unavailable, turns: unavailable, subagents: unavailable, subagentsWindow: unavailable, subagentCount: unavailable,
}
const inclusive: TraceSeries = {
  ...native,
  meters: { ...native.meters, spend: { ...native.meters.spend,
    usdScope: { _tag: 'IncludingSubagents' }, totalUsd: known(15.55), subagentUsd: known(3.21),
    subagentUsdCoverage:{_tag:'Complete'},
    tokensScope: { _tag: 'Self' }, tokensWindow: window, tokens: known(120000),
  } },
  subagentCount: known(1),
  subagentsWindow:window,
  subagents: known([{ sessionId: 'native-child', parentSessionId: known('native-session'), label: known('Research delegate'), model: known('gpt-5.4'), usd: 3.21, requests: 4, firstMs: known(now - 1_800_000), lastMs: known(now - 60_000) }]),
}
const partial: TraceSeries = { ...inclusive, coverage: { _tag: 'Partial', attributedFraction: 0.62 }, subagentCount: known(4),
  meters:{...inclusive.meters,spend:{...inclusive.meters.spend,totalUsdCoverage:{_tag:'Partial',attributedFraction:0.62},subagentUsdCoverage:{_tag:'Partial',attributedFraction:0.62}}},
}
const withAttributionFraction=(fraction:number):TraceSeries=>({...partial,coverage:{_tag:'Partial',attributedFraction:fraction},
  meters:{...partial.meters,spend:{...partial.meters.spend,totalUsdCoverage:{_tag:'Partial',attributedFraction:fraction},subagentUsdCoverage:{_tag:'Partial',attributedFraction:fraction}}},
})
const boundary=withAttributionFraction(0.9949)
const boundary995=withAttributionFraction(0.995)
const zeroCoverage=withAttributionFraction(0)
const fullPartial=withAttributionFraction(1)
const notCovered: TraceSeries = { ...native, coverage: { _tag: 'NotCovered', harnessName: 'Claude Code' } }
const percentOnly: TraceSeries = { ...native, meters: { ...native.meters, context: {
  reportedPercent: known(37.4), usedTokens: unavailable, windowTokens: unavailable,
  compactions: unavailable, lastCompactionMs: unavailable,
} } }
const partialFacts: TraceSeries = { ...partial, subagents: known([
  { sessionId: 'native-child', parentSessionId: unavailable, label: unavailable, model: unavailable, usd: 3.21, requests: 4, firstMs: unavailable, lastMs: known(now - 60_000) },
]) }
const percentSamples: TraceSeries = { ...percentOnly, turns: known({
  points: [
    { turnIndex: 1, tMs: now - 120_000, contextTokens: null, contextPercent: 18.2, estimated: false },
    { turnIndex: 2, tMs: now - 60_000, contextTokens: null, contextPercent: 37.4, estimated: false },
  ], compactions: [],
}) }
const mixedWindows:TraceSeries={...partial,subagentsWindow:known({_tag:'Reported',label:'7d'}),meters:{...partial.meters,spend:{...partial.meters.spend,
 totalUsd:{_tag:'Known',value:100,source:'series'},usdWindow:known({_tag:'Reported',label:'7d'}),usdScope:{_tag:'IncludingSubagents'},
 mainUsd:known(12.34),mainUsdScope:{_tag:'Self'},mainUsdWindow:window,mainUsdCoverage:{_tag:'Complete'},
 subagentUsd:known(22),subagentUsdWindow:known({_tag:'Reported',label:'7d'}),
}}}
const unknownTotalScope:TraceSeries={...mixedWindows,meters:{...mixedWindows.meters,spend:{...mixedWindows.meters.spend,usdScope:{_tag:'Unknown',reason:'not-attributed'}}}}
const invalidCoverage=(fraction:number):TraceSeries=>({...inclusive,
  meters:{...inclusive.meters,spend:{...inclusive.meters.spend,totalUsdCoverage:{_tag:'Partial',attributedFraction:fraction}}},
})
const invalidNaN=invalidCoverage(Number.NaN),invalidNegative=invalidCoverage(-0.1),invalidOverflow=invalidCoverage(1.1)
const scenarios = { partial, boundary, boundary995, zeroCoverage, fullPartial, notCovered, independent: native, session: inclusive, partialFacts, percentOnly, percentSamples, mixedWindows, unknownTotalScope, invalidNaN, invalidNegative, invalidOverflow } as const
interface Args extends DetailsPaneProps { scenario: keyof typeof scenarios; scheme: 'dark' | 'light'; width: 320 | 400 | 520 }
const styles = stylex.create({
  frame: { height: 720, minHeight: 0, overflow: 'hidden', backgroundColor: surface.canvas },
  width: (width: number) => ({ width }),
  comparison: { display: 'flex', flexWrap: 'wrap', gap: 16, alignItems: 'flex-start' },
})
const meta = {
  title: 'Fractal UI/Details Data',
  component: DetailsPane,
  parameters: { layout: 'fullscreen' },
  args: {
    ...detailsStoryInputs('busy', native),
    variants: { overview: 'O2', context: 'C1', spend: 'S1', resources: 'R1', empty: 'E3' },
    state: 'busy', intent: { hiddenSections: [] }, onIntentChange: () => {}, onOpen: () => {}, onRetry: () => {},
    scenario: 'independent', scheme: 'dark', width: 400,
  },
  argTypes: {
    scenario: { control: 'select', options: Object.keys(scenarios) },
    scheme: { control: 'inline-radio', options: ['dark', 'light'] },
    width: { control: 'inline-radio', options: [320, 400, 520] },
    trace: { control: false }, overview: { control: false }, sections: { control: false },
  },
  render: ({ scenario, scheme, width, ...props }) => <section aria-label={`Data case ${scenario}`} {...stylex.props(styles.frame, styles.width(width), ...baselineTheme, ...(scheme === 'light' ? lightTheme : []))}>
    <ThemePortal><Workbench resources={workbenchResources} layout={group([{ uri: 'agent:worker-1', view: 'details' }])} workspaceId={`details-data-${scenario}`} scheme={scheme}
      renderPane={() => <DetailsPane {...props} trace={scenarios[scenario]} pendingTrace={scenarios[scenario]} />}/></ThemePortal>
  </section>,
} satisfies Meta<Args>
export default meta
type Story = StoryObj<typeof meta>

function requireContract(condition: unknown, cause: string): asserts condition { if (!condition) throw new Error(cause) }
async function rejectsControl(cause: string, check: () => void | Promise<void>) {
  let rejected = false
  try { await check() } catch (error) { requireContract(error instanceof Error && error.message === cause, `Unexpected control failure: ${String(error)}`); rejected = true }
  requireContract(rejected, `Negative control did not reject: ${cause}`)
}
function pane(canvas: HTMLElement): HTMLElement {
  const element = canvas.querySelector<HTMLElement>('[data-testid="details-pane"]')
  requireContract(element, 'Missing production DetailsPane')
  requireContract(!/\bUnknown\b/.test(element.textContent ?? ''), 'Literal Unknown escaped the fact boundary')
  return element
}
function spend(pane: HTMLElement): HTMLElement {
  const element = pane.querySelector<HTMLElement>('[data-details-chart="S1"]')
  requireContract(element, 'Missing production spend section')
  return element
}
function assertPartial(root: HTMLElement, expected: string) {
  const labels = Array.from(root.querySelectorAll('[data-details-coverage]')).map(item => item.textContent)
  requireContract(labels.length > 0 && labels.every(label => label === expected), 'Coverage rounding or attribution copy differs')
  requireContract(!Array.from(spend(root).querySelectorAll('dt')).some(item => /^Total/.test(item.textContent ?? '')), 'Partial attribution was labelled a complete total')
}
function assertIndependent(root: HTMLElement, tokenLabel = 'Cumulative tokens · observed agent incarnations') {
  const section = spend(root)
  const tokenRow = section.querySelector('[data-details-token-label]')?.closest('div')
  requireContract(tokenRow?.querySelector('[data-details-token-label]')?.textContent === tokenLabel, 'Token scope inherited monetary scope')
  requireContract(!tokenRow?.querySelector('[data-details-token-period]')&&!tokenRow?.textContent?.includes('all time')&&section.querySelector('[data-details-partial-note]')?.textContent==="Accumulation window isn't reported by this source.", 'Token accumulation window was fabricated')
  const monetaryRow = Array.from(section.querySelectorAll('dt')).find(item => item.textContent?.startsWith('Total · self'))?.closest('div')
  const monetaryText = monetaryRow?.textContent
  requireContract(typeof monetaryText === 'string', 'Monetary accumulation window inherited token window')
  requireContract(monetaryText?.includes('UTC') && !monetaryText.includes('all time'), 'Monetary accumulation window inherited token window')
  requireContract(monetaryText.includes('$12.34') && tokenRow?.textContent?.includes('480'), 'Native metrics disappeared without descendants')
}
function assertPercentOnly(root: HTMLElement) {
  const section = root.querySelector('[data-details-chart^="C"]')
  requireContract(section, 'Missing context section')
  requireContract(!/\b\d[\d.,kKmM ]*\s+tokens\b|\bCapacity\b/.test(section.textContent ?? ''), 'Context occupancy rendered token counts')
  const meter = within(section as HTMLElement).getByRole('meter', { name: 'Context occupancy' })
  requireContract(meter.getAttribute('aria-valuemax') === '100' && meter.getAttribute('aria-valuenow') === '37.4', 'Reported-percent-only context was discarded')
}
function falsifyText(root:HTMLElement,selector:string,value:string):HTMLElement {
 const wrong=root.cloneNode(true) as HTMLElement
 const nodes=wrong.querySelectorAll(selector)
 requireContract(nodes.length>0,`Missing negative control target: ${selector}`)
 nodes.forEach(node=>{node.textContent=value})
 return wrong
}

export const PartialCoverage: Story = {
  args: { scenario: 'partial' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    assertPartial(root, 'Partial · 62% attributed')
    await rejectsControl('Coverage rounding or attribution copy differs', () => assertPartial(falsifyText(root,'[data-details-coverage]','Complete'), 'Partial · 62% attributed'))
    requireContract(root.textContent?.includes('Showing 1 of 4 traced sessions'), 'Retained descendants masqueraded as the full list')
  },
}
export const PartialRoundingBoundary: Story = {
  args: { scenario: 'boundary' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    assertPartial(root, 'Partial · 99% attributed')
    await rejectsControl('Coverage rounding or attribution copy differs', () => assertPartial(falsifyText(root,'[data-details-coverage]','Partial · 100% attributed'), 'Partial · 99% attributed'))
  },
}
export const NotCovered: Story = {
  args: { scenario: 'notCovered' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const trigger = within(spend(root)).getByRole('button', { name: 'Not tracked for Claude Code' })
    trigger.focus()
    requireContract(canvasElement.ownerDocument.activeElement === trigger, 'Coverage explanation is not keyboard reachable')
    await userEvent.keyboard('{Tab}{Shift>}{Tab}{/Shift}')
    await waitFor(() => requireContract(canvasElement.ownerDocument.activeElement === trigger, 'Keyboard navigation skipped the coverage trigger'))
    await waitFor(() => requireContract(within(canvasElement.ownerDocument.body).getByRole('tooltip').textContent === 'Spend is recorded only for OMP sessions today.', 'Not-covered explanation differs'))
    const wrong=falsifyText(root,'[data-details-coverage]','Not tracked for Codex')
    await rejectsControl('Not-covered harness display name differs', () => requireContract(wrong.querySelector('[data-details-coverage]')?.textContent === 'Not tracked for Claude Code', 'Not-covered harness display name differs'))
    requireContract(spend(root).textContent?.includes('$12.34') && spend(root).textContent?.includes('480'), 'Not-covered descendants erased native metrics')
    await userEvent.keyboard('{Escape}')
  },
}
export const PartialRoundingUpperBoundary:Story={
 args:{scenario:'boundary995'},
 play:async({canvasElement})=>{
  const root=pane(canvasElement)
  assertPartial(root,'Partial · 99% attributed')
  await rejectsControl('Coverage rounding or attribution copy differs',()=>assertPartial(falsifyText(root,'[data-details-coverage]','Complete'),'Partial · 99% attributed'))
  await rejectsControl('Coverage rounding or attribution copy differs',()=>assertPartial(falsifyText(root,'[data-details-coverage]','Partial · 100% attributed'),'Partial · 99% attributed'))
 },
}
export const PartialZeroCoverage:Story={
 args:{scenario:'zeroCoverage'},
 play:async({canvasElement})=>{
  const root=pane(canvasElement)
  assertPartial(root,'Partial · 0% attributed')
  await rejectsControl('Coverage rounding or attribution copy differs',()=>assertPartial(falsifyText(root,'[data-details-coverage]','Partial · 1% attributed'),'Partial · 0% attributed'))
 },
}
export const PartialCappedCoverage:Story={
 args:{scenario:'fullPartial'},
 play:async({canvasElement})=>{
  const root=pane(canvasElement)
  assertPartial(root,'Partial · 99% attributed')
  await rejectsControl('Coverage rounding or attribution copy differs',()=>assertPartial(falsifyText(root,'[data-details-coverage]','Partial · 100% attributed'),'Partial · 99% attributed'))
 },
}
function assertDescendantListWindow(root:HTMLElement) {
 const label=root.querySelector('[data-details-subagent-window]')?.textContent
 requireContract(label==='Traced sessions · reported window · 7d','Descendant list inherited the native session window')
 const spendLabel=root.querySelector('[data-details-subagent-spend-window]')?.textContent
 requireContract(spendLabel===label,'Descendant spend breakdown lost its list window')
 requireContract(root.querySelector('[data-details-metric="main-usd"]')?.textContent?.includes('UTC'),'Native session lost its independent exact window')
}
export const DescendantListWindow:Story={
 args:{scenario:'mixedWindows'},
 play:async({canvasElement})=>{
  const root=pane(canvasElement)
  assertDescendantListWindow(root)
  const nativePeriod=root.querySelector('[data-details-metric="main-usd"] [data-details-metric-period]')?.textContent
  requireContract(nativePeriod,'Missing native-window negative control')
  const wrong=falsifyText(root,'[data-details-subagent-window]',nativePeriod)
  await rejectsControl('Descendant list inherited the native session window',()=>assertDescendantListWindow(wrong))
 },
}
export const IndependentMetricScopes: Story = {
  args: { scenario: 'independent' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    assertIndependent(root)
    await rejectsControl('Token scope inherited monetary scope', () => assertIndependent(falsifyText(root,'[data-details-token-label]','Cumulative tokens · this agent')))
    const monetaryWindow=spend(root).querySelector('[data-details-metric="total-usd"] [data-details-metric-period]')?.textContent
    requireContract(monetaryWindow,'Missing monetary-window negative control target')
    for(const inventedWindow of ['all time',monetaryWindow]) {
      const wrong=root.cloneNode(true) as HTMLElement
      const tokenLabel=wrong.querySelector('[data-details-token-label]')
      requireContract(tokenLabel?.parentElement,'Missing token-window negative control target')
      const period=wrong.ownerDocument.createElement('span')
      period.setAttribute('data-details-token-period','true')
      period.textContent=inventedWindow
      tokenLabel.parentElement.append(period)
      await rejectsControl('Token accumulation window was fabricated',()=>assertIndependent(wrong))
    }
  },
}
function assertNativeWindow(root:HTMLElement) {
 const section=spend(root)
 const mainText=section.querySelector('[data-details-metric="main-usd"]')?.textContent
 requireContract(typeof mainText === 'string', 'Native monetary metadata inherited inclusive metadata')
 requireContract(mainText?.includes('Main session · self')&&!mainText.includes('reported window')&&mainText.includes('UTC')&&!mainText.includes('Partial'), 'Native monetary metadata inherited inclusive metadata')
 const totalText=section.querySelector('[data-details-metric="total-usd"]')?.textContent
 requireContract(typeof totalText === 'string', 'Producer window fabricated exact boundaries')
 requireContract(totalText?.includes('reported window · 7d')&&!totalText.includes('UTC'), 'Producer window fabricated exact boundaries')
 requireContract(mainText.includes('$12.34')&&totalText.includes('$100.00'), 'Independent monetary amounts were derived from one another')
}
export const NativeAndInclusiveWindows:Story={
 args:{scenario:'mixedWindows'},
 play:async({canvasElement})=>{
  const root=pane(canvasElement)
  assertNativeWindow(root)
  const wrong=root.cloneNode(true) as HTMLElement
  const main=wrong.querySelector('[data-details-metric="main-usd"] dt')
  const total=wrong.querySelector('[data-details-metric="total-usd"] dt')
  requireContract(main&&total,'Missing monetary metadata control rows')
  main.textContent=total.textContent
  await rejectsControl('Native monetary metadata inherited inclusive metadata',()=>assertNativeWindow(wrong))
 },
}
export const NativeWithUnattributedTotal:Story={
 args:{scenario:'unknownTotalScope'},
 play:async({canvasElement})=>{
  const root=pane(canvasElement)
  assertNativeWindow(root)
  const wrong=root.cloneNode(true) as HTMLElement
  wrong.querySelector('[data-details-metric="main-usd"]')?.remove()
  await rejectsControl('Native monetary metadata inherited inclusive metadata',()=>assertNativeWindow(wrong))
 },
}
function assertSession(root:HTMLElement) {
 const section=spend(root)
 const total=section.querySelector('[data-details-metric="total-usd"]')
 const main=section.querySelector('[data-details-metric="main-usd"]')
 const descendants=section.querySelector('[data-details-metric="subagent-usd"]')
 requireContract(total?.textContent?.includes('Total · including subagents')&&total.querySelector('dd')?.textContent==='$15.55', 'Inclusive monetary total lost its scope')
 requireContract(main?.textContent?.includes('Main session')&&main.querySelector('dd')?.textContent==='$12.34'&&descendants?.textContent?.includes('Subagents')&&descendants.querySelector('dd')?.textContent==='$3.21', 'Session and descendants were conflated')
 requireContract(section.querySelector('[data-details-token-label]')?.textContent === 'Cumulative tokens · this agent', 'Self tokens inherited inclusive monetary scope')
}
function assertPartialFacts(root:HTMLElement) {
 const card=Array.from(root.querySelectorAll('[data-resource-card]')).find(item=>item.textContent?.includes('native-child'))
 requireContract(card, 'Unavailable label hid the known descendant identity')
 requireContract(!card.textContent?.includes('Invalid Date')&&!card.textContent?.includes('NaN')&&!card.textContent?.includes('First observed')&&!card.textContent?.includes('Parent ·')&&card.textContent?.includes('Last observed'), 'Unavailable timestamps or parent fabricated lifecycle facts')
 requireContract(root.textContent?.includes('Showing 1 of 4 traced sessions'), 'Missing descendant facts changed the reported descendant count')
 requireContract(!card.textContent?.includes('gpt-5.4'), 'Unavailable model was fabricated')
}
function assertContextSamples(root:HTMLElement) {
 assertPercentOnly(root)
 const section=root.querySelector('[data-details-chart="C3"]')
 requireContract(section !== null, 'Percent-only retained observations were discarded')
 requireContract(section?.textContent?.includes('Peak 37%'), 'Percent-only retained observations were discarded')
 const titles=Array.from(section.querySelectorAll('circle title')).map(title=>title.textContent??'')
 requireContract(titles.length===2&&titles.every(title=>/\d+%/.test(title)&&!title.includes('tokens')), 'Historical occupancy escaped percentage units')
}
function assertAllStates(root:HTMLElement) {
 const names=Object.keys(scenarios)
 const mounted=root.querySelectorAll('[data-testid="details-pane"]')
 requireContract(mounted.length===names.length, 'AllStates omitted a production pane')
 for(const name of names)requireContract(root.querySelectorAll(`section[aria-label="${name}"] [data-testid="details-pane"]`).length===1, 'AllStates omitted a named scenario')
 mounted.forEach(element=>requireContract(!/\bUnknown\b/.test(element.textContent??''), 'Literal Unknown escaped the fact boundary'))
}
export const SessionVersusDescendants: Story = {
  args: { scenario: 'session' },
  play: async ({ canvasElement }) => {
    const root=pane(canvasElement)
    assertSession(root)
    const wrong=falsifyText(root,'[data-details-metric="main-usd"] dd','$15.55')
    await rejectsControl('Session and descendants were conflated',()=>assertSession(wrong))
  },
}
export const PartialDescendantFacts: Story = {
  args: { scenario: 'partialFacts' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    assertPartialFacts(root)
    const wrong=root.cloneNode(true) as HTMLElement
    const card=Array.from(wrong.querySelectorAll('[data-resource-card]')).find(item=>item.textContent?.includes('native-child'))
    requireContract(card,'Missing descendant-model negative control target')
    const invented=wrong.ownerDocument.createElement('span')
    invented.textContent='gpt-5.4'
    card.append(invented)
    await rejectsControl('Unavailable model was fabricated',()=>assertPartialFacts(wrong))
  },
}
export const ContextPercentOnly: Story = {
  args: { scenario: 'percentOnly' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    assertPercentOnly(root)
    const wrong=root.cloneNode(true) as HTMLElement
    within(wrong).getByRole('meter',{name:'Context occupancy'}).setAttribute('aria-valuemax','1000000')
    await rejectsControl('Reported-percent-only context was discarded', () => assertPercentOnly(wrong))
  },
}
export const ContextSamplesPercentOnly: Story = {
  args: { scenario: 'percentSamples', variants: { overview: 'O2', context: 'C3', spend: 'S1', resources: 'R1', empty: 'E3' } },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    assertContextSamples(root)
    const wrong=falsifyText(root,'[data-details-chart="C3"] circle title','18 tokens · measured')
    await rejectsControl('Historical occupancy escaped percentage units',()=>assertContextSamples(wrong))
  },
}
function assertInvalidCoverageSurvives(canvas:HTMLElement) {
 const root=canvas.querySelector<HTMLElement>('[data-testid="details-pane"]')
 requireContract(root,'Invalid row blanked the Details pane')
 requireContract(root.querySelector('[data-details-metric="total-usd"] [data-details-coverage]')?.textContent==='Partial','Invalid fraction fabricated an attribution percentage')
 requireContract(root.querySelector('[data-details-metric="main-usd"]')?.textContent?.includes('$12.34')&&root.querySelector('[data-details-metric="subagent-usd"]')?.textContent?.includes('$3.21'),'Invalid coverage erased valid monetary rows')
 requireContract(root.querySelector('section[aria-label="Overview"]')&&root.querySelector('[data-details-chart^="C"]')&&root.querySelector('section[aria-label="Workspace"]'),'Invalid coverage erased sibling sections')
}
const invalidPartialPlay=async({canvasElement}:{canvasElement:HTMLElement})=>{
 assertInvalidCoverageSurvives(canvasElement)
 const blank=canvasElement.cloneNode(true) as HTMLElement
 blank.querySelector('[data-testid="details-pane"]')?.remove()
 await rejectsControl('Invalid row blanked the Details pane',()=>assertInvalidCoverageSurvives(blank))
 const percentage=falsifyText(canvasElement,'[data-details-metric="total-usd"] [data-details-coverage]','Partial · NaN% attributed')
 await rejectsControl('Invalid fraction fabricated an attribution percentage',()=>assertInvalidCoverageSurvives(percentage))
}
export const InvalidPartialNaN:Story={args:{scenario:'invalidNaN'},play:invalidPartialPlay}
export const InvalidPartialNegative:Story={args:{scenario:'invalidNegative'},play:invalidPartialPlay}
export const InvalidPartialOverflow:Story={args:{scenario:'invalidOverflow'},play:invalidPartialPlay}
export const AllStates: Story = {
  render: ({ scheme, width, ...props }) => <div {...stylex.props(styles.comparison)}>{Object.entries(scenarios).map(([name, trace]) => <section key={name} aria-label={name} {...stylex.props(styles.frame, styles.width(width), ...baselineTheme, ...(scheme === 'light' ? lightTheme : []))}>
    <ThemePortal><Workbench resources={workbenchResources} layout={group([{ uri: 'agent:worker-1', view: 'details' }])} workspaceId={`details-data-all-${name}`} scheme={scheme}
      renderPane={() => <DetailsPane {...props} trace={trace} pendingTrace={trace} />}/></ThemePortal>
  </section>)}</div>,
  play:async({canvasElement})=>{
    assertAllStates(canvasElement)
    const wrong=canvasElement.cloneNode(true) as HTMLElement
    const missing=wrong.querySelector('[data-testid="details-pane"]')
    requireContract(missing,'Missing AllStates negative control target')
    missing.remove()
    await rejectsControl('AllStates omitted a production pane',()=>assertAllStates(wrong))
  },
}
