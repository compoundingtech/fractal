import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from './assistant-ui/composition/Controls'
import { Workbench, type WorkbenchProps } from './assistant-ui/workbench/Workbench'
import { group } from './assistant-ui/workbench/workbench-model'
import { detailsWorkbenchResources as workbenchResources } from './assistant-ui/details/story-fixtures'
import { DetailsPane, type DetailsVariants, type DetailsLayoutIntent } from './assistant-ui/details/DetailsPane'
import { DetailsLabelContext, traceHistorySection } from './assistant-ui/details/DetailsResources'
import { ResourceCardV1, ResourceChipV1 } from './assistant-ui/resources/ResourceCardV1'
import { resources as sharedResources } from './assistant-ui/details/resource-fixtures'
import { accumulationParts, hasDetailsChartData } from './assistant-ui/details/DetailsCharts'
import type { DetailsState } from './assistant-ui/details/model'
import { detailsStoryInputs, paintedText } from './assistant-ui/details/story-fixtures'
import { detailsTraceFixtures, traceForDetailsState } from './assistant-ui/details/trace/fixture-provider'
import { adaptTrace } from './assistant-ui/details/trace/adapter'
import { sessionTraceFixture } from './assistant-ui/details/trace/fixture'
import type { TraceSeries } from './assistant-ui/details/trace-series'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { surfaceVars as surface } from './assistant-ui/composition-tokens.stylex'

const variants: DetailsVariants = { overview: 'O2', context: 'C2', spend: 'S1', empty: 'E3' }
const widths = [320, 400, 520]
const full = detailsTraceFixtures.full
const missing = { _tag: 'Unknown', reason: 'not-covered' } as const
const zero = { _tag: 'Known', value: 0, source: 'series', asOfMs: 1799999999000 } as const
const zeroCost = { uncachedInputUsd: 0, cacheReadUsd: 0, cacheWriteUsd: 0, outputUsd: 0 }
interface Case { name: string; width: number; state: DetailsState; variants: DetailsVariants; trace?: TraceSeries }
function example(name: string, width = 400, state: DetailsState = 'busy', changes: Partial<DetailsVariants> = {}, trace?: TraceSeries): Case {
  return { name, width, state, variants: { ...variants, ...changes }, ...(trace === undefined ? {} : { trace }) }
}
const sourceCases = (['turn-only', 'bucket-only'] as const).flatMap(source => (['unknown', 'positive', 'zero'] as const).flatMap(amounts => (['S1', 'S2', 'S3'] as const).map(spend => {
  const meter = amounts === 'unknown' ? { ...full.meters.spend, totalUsd: missing, mainUsd: missing, subagentUsd: missing, tokens: missing, split: missing }
    : amounts === 'zero' ? { ...full.meters.spend, totalUsd: zero, mainUsd: zero, subagentUsd: zero, tokens: zero, split: { ...zero, value: zeroCost } } : full.meters.spend
  const trace: TraceSeries = { ...full, meters: { ...full.meters, spend: meter }, subagents: missing,
    series: source === 'turn-only' ? missing : amounts === 'zero' && full.series._tag === 'Known' ? { ...full.series, value: { ...full.series.value, points: full.series.value.points.map(point => ({ ...point, cost: zeroCost, subagentUsd: 0 })) } } : full.series,
    turns: source === 'bucket-only' ? missing : full.turns }
  return example(`${source}/${amounts}/${spend}`, 400, 'busy', { spend, empty: 'E2' }, trace)
})))
const presentationCases: Case[] = [
  ...(['new', 'busy', 'partial', 'unknown', 'idle', 'absent'] as const).flatMap(state => (['C1', 'C2', 'C3'] as const).map((context, index) => example(`${state}/${context}`, widths[index]!, state, { context, spend: (['S1', 'S2', 'S3'] as const)[index]!}))),
  ...widths.map(width => example(`new/${width}`, width, 'new')),
  ...(['overview', 'context', 'spend', 'empty'] as const).flatMap(dimension => ['1', '2', '3'].map(index => example(`${dimension}/${index}`, 400, 'busy', { [dimension]: `${dimension[0]!.toUpperCase()}${index}` } as Partial<DetailsVariants>))),
]
const resourceStates = ['busy', 'partial', 'new', 'unknown', 'idle', 'absent'] as const
const resourceStateCases = resourceStates.map(state => example(`${state}/400`, 400, state))
interface Args extends WorkbenchProps { cases: readonly Case[]; comparison: boolean }
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', minHeight: 0, height: 560, overflow: 'hidden', backgroundColor: surface.canvas },
  controls: { display: 'flex', flexWrap: 'wrap', gap: 4, backgroundColor: surface.canvas },
  comparison: { display: 'flex', flexWrap: 'wrap', gap: 16, backgroundColor: surface.canvas },
})
const meta = {
  excludeStories: ['scenarioExemptions'],
  id: 'fractal-ui-details-resources', title: 'Surfaces/Details/Resources',
  component: Workbench,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'Production DetailsPane mounted in Workbench. Cases preserve inherited chart, resource, scope and geometry gates without the former ExploreDetails or DetailsWorkbench harness. Host counters observe Details callbacks only; terminal drawers, split-tree wiring and sidebar selection are intentionally not rendered.' } } },
  args: { resources: workbenchResources, layout: group([{ uri: 'agent:worker-1', view: 'details' }]), workspaceId: 'details-resources', scheme: 'dark', cases: [example('busy')], comparison: false },
  render: function Render({ cases, comparison, ...args }) {
    const [selected, setSelected] = React.useState(0)
    const [stateOverride, setStateOverride] = React.useState<DetailsState>()
    const [intent, setIntent] = React.useState<DetailsLayoutIntent>({ hiddenSections: [] })
    const [intentWrites, setIntentWrites] = React.useState(0)
    const [opens, setOpens] = React.useState<readonly { key: string; split: boolean }[]>([])
    const mounted = comparison ? cases : [cases[selected]!]
    return <div data-testid="resource-gates" data-selected-case={selected} data-intent-writes={intentWrites} data-open-events={JSON.stringify(opens)}>
      <header {...stylex.props(styles.controls, ...baselineTheme, ...(args.scheme === 'light' ? lightTheme : []))}>{!comparison && cases.map((item, index) => <Button key={item.name} size="sm" onPress={() => { setSelected(index); setStateOverride(undefined) }}>{item.name}</Button>)}
        {(['idle', 'new', 'absent'] as const).map(state => <Button key={state} size="sm" onPress={() => setStateOverride(state)}>Set {state}</Button>)}
      </header>
      <div {...stylex.props(comparison && styles.comparison)}>{mounted.map(item => <section key={item.name} aria-label={item.name} data-testid="resource-stage" style={{ width: item.width }} {...stylex.props(styles.root, ...baselineTheme, ...(args.scheme === 'light' ? lightTheme : []))}>
        <ThemePortal><Workbench {...args} workspaceId={`${args.workspaceId}-${item.name}`} renderPane={() => <DetailsLabelContext.Provider value={comparison ? `${item.name} ` : ''}><DetailsPane {...detailsStoryInputs(stateOverride ?? item.state,item.trace)} variants={item.variants} state={stateOverride ?? item.state} intent={intent}
          onIntentChange={next => { setIntent(next); setIntentWrites(count => count + 1) }} onOpen={(key, split) => setOpens(current => [...current, { key, split }])} onRetry={() => setStateOverride('busy')} /></DetailsLabelContext.Provider>}/></ThemePortal>
      </section>)}</div>
    </div>
  },
} satisfies Meta<Args>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'E3ChartMatrix:contract-fixture',
  'Interactions:contract-fixture',
  'OrientationAnchor:contract-fixture',
  'PresentationMatrix:contract-fixture',
  'ResourceGeometry:contract-fixture',
  'ResourceStates:contract-fixture',
  'SharedResourceActionNames:contract-fixture',
  'SpendScope:contract-fixture',
  'TraceContracts:contract-fixture',
]
type Story = StoryObj<typeof meta>
function requireContract(value: unknown, reason: string): asserts value { if (!value) throw new Error(reason) }
async function rejectsControl(reason: string, assertion: () => void | Promise<void>) {
  let rejected = false
  try { await assertion() } catch (error) { requireContract(error instanceof Error && error.message === reason, `Unexpected control failure: ${String(error)}`); rejected = true }
  requireContract(rejected, `Negative control did not reject: ${reason}`)
}
async function frames() { await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))) }
async function ready(canvas: HTMLElement) { await canvas.ownerDocument.fonts.ready; await frames() }
function paneFor(canvas: HTMLElement) { const pane = canvas.querySelector<HTMLElement>('[data-testid="details-pane"]'); requireContract(pane, 'Missing production Details pane'); return pane }
async function eachCase(canvas: HTMLElement, cases: readonly Case[], assertion: (pane: HTMLElement, item: Case) => void | Promise<void>) {
  await ready(canvas)
  for (const [index, item] of cases.entries()) {
    await userEvent.click(within(canvas).getByRole('button', { name: item.name }))
    await waitFor(() => requireContract(canvas.querySelector('[data-testid="resource-gates"]')?.getAttribute('data-selected-case') === String(index), 'Host did not select case'))
    await frames()
    const pane = paneFor(canvas)
    requireContract(Math.abs(pane.getBoundingClientRect().width - item.width) <= 2, 'Requested Details width differs from mounted width')
    await assertion(pane, item)
  }
}
function shown(element: HTMLElement) { return !element.closest('[inert],[hidden]') && getComputedStyle(element).visibility !== 'hidden' && getComputedStyle(element).display !== 'none' && element.getBoundingClientRect().width > 0 }
function assertHuman(pane: HTMLElement) {
  requireContract(!/\bUnknown\b|\b(no-provider|not-covered|not-attributed|out-of-retention)\b|Some values (?:aren't|aren’t) reported|Some details (?:aren't|aren’t) available|Not observed/i.test(pane.innerText), 'Details exposes a tag or generic missing-data value')
  requireContract(!/(?:\+|−)0\b|\b1 compactions\b|claude-opus|claude-sonnet|gpt-\d/i.test(pane.innerText), 'Details exposes zero diff counts, wrong plural or unsanitized model')
  for (const row of pane.querySelectorAll<HTMLElement>('p,[data-details-unavailable]')) if (shown(row)) {
    requireContract(!/^(?:—|Unavailable(?:\s+—)?|Partial)$/.test(row.textContent?.trim() ?? '') && (!row.hasAttribute('data-details-unavailable') || !!row.getAttribute('data-details-unavailable-subject')?.trim()), 'Details renders an unlabeled unavailable row')
  }
  for (const section of pane.querySelectorAll('section')) {
    const notes = [...section.querySelectorAll<HTMLElement>('[data-details-unavailable]')].filter(shown)
    requireContract(notes.length <= 1, 'Section repeats its missing-data explanation')
    requireContract([...section.querySelectorAll('[data-details-missing-value]')].every(value=>value.textContent==='—'&&value.getAttribute('aria-label')?.endsWith(': unavailable')), 'Partial explanation does not identify its missing field')
    const disclosure = section.querySelector('button[aria-expanded]')
    requireContract(!disclosure || disclosure.querySelector('svg'), 'Section disclosure lacks a kit chevron')
    requireContract(![...section.querySelectorAll('button')].some(button => /^(Hide|×)/.test(button.textContent ?? '')), 'Section duplicates its disclosure affordance')
  }
  for (const element of pane.querySelectorAll<HTMLElement | SVGElement>('*')) {
    if (!shown(element as HTMLElement)) continue
    const css = getComputedStyle(element)
    for (const property of ['color', 'backgroundColor', 'borderTopColor', 'fill', 'stroke'] as const) {
      const channels = css[property].match(/rgba?\(([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/)
      requireContract(!channels || Number(channels[2]) <= Math.max(Number(channels[1]), Number(channels[3])) + 20, 'Details paints green status or metric ink')
    }
  }
  assertMetricGroups(pane)
}
/** The approved whole-group summary, identical on desktop and phone. */
const groupSummaries: Readonly<Record<string, string>> = { Context: 'This session does not report context usage.', Spend: 'This session does not report spend.' }
/** Context and Spend name every missing field once: `Spend · not reported` with its summary sentence and one collapsed Why?, or reported fields plus one `Not reported:` summary whose reasons wait behind one Why?; never per-field dashes or tooltip triggers. */
function assertMetricGroups(pane: HTMLElement) {
  for (const name of ['Context', 'Spend']) {
    const section = pane.querySelector<HTMLElement>(`section[aria-label$="${name}"]`)
    if (!section) continue
    const lines = section.querySelectorAll<HTMLElement>('[data-details-not-reported]')
    requireContract(lines.length <= 1 && !section.querySelector('[data-details-missing-value]') && !paintedText(section).includes('—'), 'Metric group paints per-field missing values')
    const line = lines[0]
    if (line?.dataset.detailsNotReported === 'group') {
      const head = [...line.children].slice(0, 3).map(node => node.textContent).join(''), summaries = line.querySelectorAll('[data-details-group-summary]'), whys = line.querySelectorAll<HTMLElement>('button[data-details-why]')
      requireContract(line.parentElement?.childElementCount === 1 && head === `${name}·not reported` && summaries.length === 1 && summaries[0]!.textContent === groupSummaries[name] && whys.length === 1 && whys[0]!.getAttribute('aria-expanded') === 'false' && (whys[0]!.textContent ?? '').replace(/\s+/g, ' ').trim() === `Why? ${name} not reported` && !line.querySelector('button:not([data-details-why]),[aria-describedby]'), 'Unreported metric group is not its summary behind one collapsed Why?')
    }
    if (line?.dataset.detailsNotReported === 'fields') {
      const names = (line.dataset.detailsNotReportedNames ?? '').split(', ').filter(Boolean), summary = line.querySelector('[data-details-not-reported-summary]')?.textContent ?? ''
      requireContract((names.length > 0 ? summary.startsWith('Not reported: ') : summary === `Some ${name.toLowerCase()} details aren’t reported.`) && line.querySelectorAll('[data-details-not-reported-field]').length === names.length && line.querySelectorAll('button[data-details-why]').length === 1 && !line.querySelector('button:not([data-details-why])'), 'Partial metric group lacks one Not reported summary behind Why?')
    }
  }
}
function rgb(value: string): number[] | undefined {
  const values = value.match(/[\d.]+/g)?.map(Number)
  if (!values || values.length < 3) return undefined
  const scale = value.startsWith('color(') ? 255 : value.includes('%') ? 2.55 : 1
  return [...values.slice(0, 3).map(number => number * scale), values[3] ?? 1]
}
function contrast(element: Element) {
  let background = [255, 255, 255, 1]
  for (let current: Element | null = element; current; current = current.parentElement) { const color = rgb(getComputedStyle(current).backgroundColor); if (color?.[3] === 1) { background = color; break } }
  const color = rgb(getComputedStyle(element).color)
  requireContract(color, 'Unparseable Details text color')
  const luminance = (channels: number[]) => channels.slice(0, 3).map(number => number / 255).map(number => number <= 0.04045 ? number / 12.92 : ((number + 0.055) / 1.055) ** 2.4).reduce((sum, number, index) => sum + number * [0.2126, 0.7152, 0.0722][index]!, 0)
  const foreground = luminance(color.slice(0, 3).map((number, index) => number * color[3]! + background[index]! * (1 - color[3]!)))
  const back = luminance(background)
  return (Math.max(foreground, back) + 0.05) / (Math.min(foreground, back) + 0.05)
}
function assertCraft(pane: HTMLElement, item: Case) {
  assertHuman(pane)
  for (const reason of pane.querySelectorAll<HTMLElement>('[data-details-unavailable]')) if (shown(reason)) requireContract(contrast(reason) >= 4.5, 'Unavailable explanation contrast below 4.5:1')
  for (const count of pane.querySelectorAll<HTMLElement>('[data-details-section-count]')) if (shown(count)) requireContract(contrast(count) >= 4.5, 'Section count contrast below 4.5:1')
  for (const name of ['Workspace', 'Pull requests', 'CI runs']) {
    const section = pane.querySelector(`section[aria-label$="${name}"]`), heading = section?.querySelector('[data-details-heading]'), meta = section?.querySelector('[data-thing-meta-item]')
    if (heading && meta) requireContract(contrast(heading) > contrast(meta), 'Resource heading is no stronger than metadata')
  }
  if (item.state === 'new' && item.variants.empty === 'E3') {
    const orientation = pane.querySelectorAll('[data-details-orientation]')
    requireContract(orientation.length === 1 && /context.*spend.*resources/i.test(orientation[0]?.textContent ?? ''), 'New agent lacks one calm Details orientation')
    requireContract(!pane.querySelector('[data-details-source-note]'), 'New agent repeats source unavailability')
    for (const section of pane.querySelectorAll('section')) requireContract(section.querySelector('strong,dd,[data-details-data-row],[data-thing-row],svg[role="img"]'), 'New agent offers an empty disclosure')
  }
  if (item.state === 'unknown') for (const name of ['Context', 'Spend']) requireContract(pane.querySelector(`section[aria-label$="${name}"] [data-details-not-reported="group"]`), 'Unknown source omits its single not-reported line')
  const history = pane.querySelector('section[aria-label$="History"]')
  if (item.state === 'busy' || item.state === 'idle') requireContract(history&&[...history.querySelectorAll('[data-thing-title]')].some(title=>title.textContent?.startsWith('Session · ')), 'Known trace history is absent')
  const changes = pane.querySelector('[aria-label="Compaction context changes"]')
  // Compactions paint context occupancy: 897,170 → 132,952 of a 1M window, excluding the straddling bucket.
  if (changes) requireContract(/90% → 13%/.test(changes.textContent ?? ''), 'Compaction includes the straddling bucket')
  if (item.state === 'busy' && item.variants.empty === 'E3' && item.variants.spend !== 'S1') {
    const chart = pane.querySelector(`[data-details-chart="${item.variants.spend}"]`), meter = chart?.querySelector('dl[aria-label="Trace spend meters"]')
    requireContract(meter?.textContent?.includes('Cumulative tokens') && meter.querySelector('[data-details-metric="subagent-usd"] dd') && chart?.querySelector('[aria-label="Subagent spend breakdown"]')?.textContent?.includes('Worker 1'), 'E3 chart omits known spend meter rows')
    requireContract(chart?.firstElementChild?.contains(meter!), 'E3 meter is not above its chart')
  }
}
async function humanControl(pane: HTMLElement) {
  const target = pane.querySelector<HTMLElement>('strong')!
  requireContract(target, 'Human-copy negative control lacks known identity')
  const original = target.textContent
  try { target.textContent = 'Unknown'; await rejectsControl('Details exposes a tag or generic missing-data value', () => assertHuman(pane)) } finally { target.textContent = original }
  const orphan = document.createElement('p')
  orphan.textContent = '—'; target.closest('section')!.append(orphan)
  try { await rejectsControl('Details renders an unlabeled unavailable row', () => assertHuman(pane)) } finally { orphan.remove() }
  assertHuman(pane)
}
async function contrastControl(pane: HTMLElement, item: Case) {
  const count = [...pane.querySelectorAll<HTMLElement>('[data-details-section-count]')].find(shown)
  requireContract(count, 'Contrast control lacks a shown section count')
  // Ink equal to the pane surface is the exact low-contrast cause in either scheme.
  const color = count.style.color
  try { count.style.color = getComputedStyle(pane).backgroundColor; await rejectsControl('Section count contrast below 4.5:1', () => assertCraft(pane, item)) } finally { count.style.color = color }
  assertCraft(pane, item)
}
/** Kit rows: one line each, title first, metadata whole or absent; the pane never overflows its drop surface. */
function assertRows(pane: HTMLElement) {
  const rows = [...pane.querySelectorAll<HTMLElement>('[data-thing-row]')].filter(row => !row.closest('[inert]'))
  requireContract(rows.length > 0, 'Resource row assertions have no measured rows')
  for (const row of rows) {
    const title = row.querySelector<HTMLElement>('[data-thing-title]'), box = row.getBoundingClientRect()
    requireContract(title, 'Resource row lacks its title')
    requireContract(box.height <= parseFloat(getComputedStyle(row).minHeight) + 0.5, 'Resource row wraps beyond one line')
    requireContract(title.getBoundingClientRect().right <= box.right + 1, 'Resource title escapes its row')
    const meta = row.querySelector<HTMLElement>('[data-thing-meta]')
    if (!meta) continue
    const line = meta.getBoundingClientRect(), items = [...meta.querySelectorAll<HTMLElement>('[data-thing-meta-item]')]
    const visible = items.filter(item => { const rect = item.getBoundingClientRect(); return rect.width > 0 && rect.top < line.bottom - 0.5 })
    for (const item of visible) { const rect = item.getBoundingClientRect(); requireContract(rect.left >= line.left - 0.5 && rect.right <= line.right + 0.5 && rect.bottom <= line.bottom + 0.5, 'Metadata is partially shown') }
    // Meta gives way whole, status word first, then id, then age: what shows is always a prefix of age, id, status.
    requireContract(visible.every((item, index) => item === items[index]), 'Metadata drops out of order')
    // Title first: a cut title never sits beside shown metadata.
    if (title.scrollWidth > title.clientWidth + 0.5) requireContract(visible.length === 0, 'Title truncates beside shown metadata')
  }
  const surface = pane.closest<HTMLElement>('[data-drop-group]')
  requireContract(surface, 'Details has no production AgentDropSurface')
  requireContract(surface.scrollHeight === surface.clientHeight, 'AgentDropSurface overflows vertically')
}
function assertChevron(pane: HTMLElement) {
  const columns = [...pane.querySelectorAll('section')].map(section => ({ name: section.getAttribute('aria-label'), svg: section.querySelector('button[aria-expanded] svg') }))
  for (const name of ['Overview', 'Context', 'Spend', 'Workspace', 'Files touched', 'Subagents']) requireContract(columns.some(column => column.name === name && column.svg), 'Required section chevron is missing')
  const reference = columns.find(column => column.name === 'Overview')!.svg!.getBoundingClientRect().x
  requireContract(columns.filter(column => column.svg).every(column => Math.abs(column.svg!.getBoundingClientRect().x - reference) <= 0.5), 'Section chevrons do not share the Overview column')
}
function assertScope(pane: HTMLElement, trace: TraceSeries) {
  const spend = pane.querySelector<HTMLElement>('section[aria-label="Spend"]')!, label = spend.querySelector('[data-details-token-label]')?.textContent ?? '', period = spend.querySelector('[data-details-token-period]')?.textContent ?? '', meter = trace.meters.spend
  requireContract(label.includes('Cumulative tokens'), 'Known cumulative token meter is missing')
  const scopeNote = spend.querySelector<HTMLElement>('[data-details-scope-note]')
  if (scopeNote) requireContract(scopeNote.getBoundingClientRect().height <= parseFloat(getComputedStyle(scopeNote).lineHeight) + 0.5, 'Unreported scope note is not one quiet line')
  if (meter.tokensScope._tag === 'Unknown') {
    requireContract(!/incl\. subagents|all agent sessions|including subagents|excludes subagents|this agent|observed agent sessions/i.test(label), 'Unreported total makes an unproven scope claim')
    if (meter.usdScope._tag === 'Unknown') requireContract(!spend.querySelector('[aria-label="Subagent spend breakdown"],[aria-label="Session and subagent spend"]'), 'Unreported monetary total makes an unproven breakdown claim')
    requireContract([...spend.querySelectorAll('[data-details-not-reported] [data-details-reason-note] [data-details-reason-text]')].some(note => note.textContent === 'Cumulative tokens: which sessions are included isn’t reported.'), 'Unreported token total lacks its exact scope explanation')
  } else {
    requireContract(label === `Cumulative tokens${meter.tokensScope._tag === 'Self' ? '' : meter.tokensScope._tag === 'AgentIncarnations' ? ' · all agent sessions' : ' incl. subagents'}`, 'Reported token scope is absent from its inline label')
    if (meter.tokensWindow._tag === 'Known') {
      const window = meter.tokensWindow.value
      if (window._tag === 'AllTime') requireContract(period === 'all time', 'Reported all-time window is absent')
      else if (window._tag === 'Interval') { const from = new Date(window.fromMs).toISOString(), to = new Date(window.toMs).toISOString(); requireContract(period.includes(from.slice(0, 10)) && period.includes(from.slice(11, 16)) && period.includes(to.slice(11, 16)) && period.includes('UTC'), 'Reported accumulation interval is absent from its separate date line') }
      else requireContract(period === accumulationParts(meter.tokensWindow)?.join(''), 'Reported accumulation window label is absent')
    } else requireContract(spend.querySelector('[data-details-not-reported] [data-details-not-reported-field="token window"]')&&!paintedText(spend).includes('all time'), 'Unreported accumulation window claims all-time scope')
  }
}
function assertContext(pane: HTMLElement, trace: TraceSeries) {
  const projection = adaptTrace(trace), chart = pane.querySelector<HTMLElement>('[data-chart-samples]')
  if (projection === undefined) {
    requireContract(chart === null, 'Context plot fabricates unreported source samples')
    return
  }
  const compact = pane.querySelector('[data-details-chart="C2"]') !== null
  const samples = projection.lane.turnSamples !== undefined && (compact || projection.lane.samples.length === 0) ? projection.lane.turnSamples : projection.lane.samples
  if (!pane.querySelector('[data-details-chart="C1"]') && samples.some(sample => sample.contextPercent !== null)) requireContract(chart, 'Context plot does not match retained source samples')
  if (!chart) return
  const points = [...chart.querySelectorAll('circle')]
  requireContract(points.length > 0 && points.length === samples.filter(sample => sample.contextPercent !== null).length && Number(chart.dataset.chartSourceCount) === samples.length, 'Context plot does not match retained source samples')
  const kind = projection.lane.turnSamples !== undefined && (compact || projection.lane.samples.length === 0) ? 'turn' : 'bucket'
  requireContract(chart.dataset.chartSampleKind === kind, 'Context plot mislabels source granularity')
  if (compact) {
    const axis = chart.querySelector('[data-chart-axis-label]')?.textContent ?? ''
    requireContract(kind === 'turn' ? axis.includes('per turn') : axis.includes('per minute') && !/per turn/i.test(pane.innerText), 'Context axis renames buckets as turns')
    if (kind === 'turn' && points.length > 2 && samples.every(sample => sample.contextPercent !== null)) { const xs = points.map(point => Number(point.getAttribute('cx'))), step = (xs.at(-1)! - xs[0]!) / (xs.length - 1); requireContract(xs.slice(1).every((x, index) => Math.abs(x - xs[index]! - step) <= 0.001), 'Per-turn context samples are not spaced by turn index') }
  }
}
function assertFixtureFacts() {
  requireContract(full.nativeSessionId._tag === 'Known' && full.turns._tag === 'Known' && full.series._tag === 'Known', 'Full trace fixture lacks retained sources')
  const nativeSessionId = full.nativeSessionId.value
  const requests = sessionTraceFixture.requests.filter(request => request.sessionId === nativeSessionId).toSorted((a, b) => a.ts - b.ts)
  requireContract(full.meters.context.usedTokens._tag === 'Known' && full.meters.context.usedTokens.source === 'st' && full.meters.context.usedTokens.value === 61206 && full.subagents._tag === 'Known' && full.meters.spend.usdScope._tag === 'IncludingSubagents', 'Full trace fixture loses snapshot provenance or attribution')
  requireContract(requests.length === 17 && full.turns.value.points.length === requests.length && full.turns.value.points.every((point, index) => point.turnIndex === index + 1 && point.tMs === requests[index]?.ts) && requests.every(request => request.requests === 1), 'Trace turns are not the actual source requests')
  const partial = detailsTraceFixtures.partial, unknown = detailsTraceFixtures.unknown
  requireContract(partial.series._tag === 'Unknown' && partial.meters.spend.mainUsd._tag === 'Unknown' && partial.meters.spend.subagentUsd._tag === 'Unknown' && partial.meters.spend.usdScope._tag === 'Unknown' && partial.meters.spend.usdScope.reason === 'not-attributed' && partial.meters.spend.tokens._tag === 'Known' && partial.meters.spend.tokens.source === 'st', 'Partial trace fixture invents attribution')
  requireContract(unknown.nativeSessionId._tag === 'Unknown' && unknown.meters.spend.totalUsd._tag === 'Unknown', 'Unknown trace fixture invents identity or spend')
  const projection = adaptTrace(full), history = traceHistorySection(full)
  requireContract(projection !== undefined, 'Full trace lacks a chart projection')
  const compaction = projection.lane.compactions[0]
  requireContract(compaction?.before === 897170 / 1000000 * 100 && compaction.after === 132952 / 1000000 * 100, 'Compaction includes the straddling bucket')
  requireContract(full.history._tag === 'Known' && history.items._tag === 'Known' && history.items.value.length === full.history.value.length && history.items.value[0]?.uri === `session:${full.history.value[0]?.nativeSessionId}` && traceHistorySection(partial).items._tag === 'Unknown', 'History projection loses source identity or unavailability')
  const turnOnly = adaptTrace({ ...full, series: missing })
  requireContract(turnOnly !== undefined, 'Retained turns lack a chart projection')
  requireContract(turnOnly.lane.samples.length === 0 && turnOnly.lane.turnSamples?.length === requests.length && turnOnly.lane.cumulativeValues.length === 0, 'Turn-only trace fabricates bucket spend')
  const nullTurn = adaptTrace({ ...full, turns: { ...full.turns, value: { ...full.turns.value, points: full.turns.value.points.map((point, index) => index === 0 ? { ...point, contextTokens: null, contextPercent: null } : point) } } })
  requireContract(nullTurn !== undefined, 'Retained context observations lack a chart projection')
  requireContract(nullTurn.lane.turnSamples?.[0]?.contextPercent === null, 'Null context observation is fabricated')
  for (const context of ['C1', 'C2', 'C3'] as const) for (const spend of ['S1', 'S2', 'S3'] as const) for (const section of ['context', 'spend'] as const) {
    requireContract(!hasDetailsChartData({ trace: unknown, context, spend, section }) && hasDetailsChartData({ trace: full, context, spend, section }), 'Chart availability confuses missing and known data')
  }
}

export const Interactions: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed Session terminal and src/ resource rows prove painted accessible names/descriptions and exact terminal:sample/1 and diff:sample/ callback identities, alongside controlled visibility-intent write counts.' } }, play: async ({ canvasElement }) => {
  await ready(canvasElement)
  const pane = paneFor(canvasElement), root = canvasElement.querySelector<HTMLElement>('[data-testid="resource-gates"]')!, canvas = within(canvasElement)
  const writes = () => Number(root.dataset.intentWrites), before = writes(), overview = within(pane).getByRole('button', { name: 'Overview' })
  await userEvent.click(overview); requireContract(overview.getAttribute('aria-expanded') === 'false' && writes() === before, 'Local collapse writes synced visibility intent')
  await userEvent.click(overview); requireContract(overview.getAttribute('aria-expanded') === 'true' && writes() === before, 'Local expansion writes synced visibility intent')
  const workspace = within(pane).getByRole('button', { name: /^Workspace/ }); await userEvent.click(workspace); requireContract(workspace.getAttribute('aria-expanded') === 'false' && writes() === before, 'Resource collapse writes synced visibility intent'); await userEvent.click(workspace)
  await userEvent.click(canvas.getByRole('button', { name: 'Visible sections' })); requireContract(writes() === before, 'Visibility disclosure writes synced intent')
  const context = within(canvasElement.ownerDocument.body).getByRole('checkbox', { name: 'Context' }) as HTMLInputElement
  await userEvent.click(context.closest('label')!); await waitFor(() => requireContract(!context.checked && writes() === before + 1 && !pane.querySelector('section[aria-label="Context"]'), 'Explicit hide does not commit exactly one visibility intent'))
  context.focus(); await userEvent.keyboard(' '); await waitFor(() => requireContract(context.checked && writes() === before + 2 && pane.querySelector('section[aria-label="Context"]'), 'Keyboard restore does not commit exactly one visibility intent'))
  await userEvent.keyboard('{Escape}')
  const events = () => JSON.parse(root.dataset.openEvents!) as { key: string; split: boolean }[]
  // Openable rows are links named by everything they paint: kind, title, then status or ask, whatever the width drops.
  const terminal = within(pane).getByRole('link', { name: /^Terminal Session terminal\b/ })
  const painted = () => [terminal.querySelector('[data-thing-title]')?.textContent, ...[...terminal.querySelectorAll('[data-thing-meta-item],[data-thing-act]')].map(node => node.textContent)].filter((text): text is string => Boolean(text))
  const assertName = () => requireContract(within(pane).queryByRole('link', { name: name => name.startsWith('Terminal ') && painted().every(text => name.includes(text)) }) === terminal, 'Resource name differs from its painted text')
  assertName()
  try { terminal.setAttribute('aria-label', 'Open'); await rejectsControl('Resource name differs from its painted text', assertName) } finally { terminal.removeAttribute('aria-label') }
  assertName()
  await userEvent.click(terminal); await waitFor(() => requireContract(events().at(-1)?.key === 'terminal:sample/1' && !events().at(-1)?.split, 'Details terminal callback loses exact session identity'))
  await userEvent.click(terminal); requireContract(events().length === 2 && writes() === before + 2, 'Repeated terminal request alters layout intent or fails to reattach')
  const file = within(pane).getAllByRole('link', { name: /^File src\// })[0]!
  await userEvent.click(file); requireContract(events().at(-1)?.key.startsWith('diff:sample/') && !events().at(-1)?.split, 'Details resource open callback loses pane identity')
  // One user-event instance carries the held modifier into the click; direct APIs reset keyboard state per call.
  const user = userEvent.setup(); await user.keyboard('{Control>}'); try { await user.click(file) } finally { await user.keyboard('{/Control}') }
  requireContract(events().at(-1)?.split, 'Details modified resource open loses split intent')
  file.focus(); await user.keyboard('{Alt>}{Enter}{/Alt}')
  requireContract(events().at(-1)?.split && writes() === before + 2, 'Alt+Enter alters synced visibility or loses split intent')
  await user.keyboard('{Enter}')
  await rejectsControl('Details modified resource open loses split intent', () => requireContract(events().at(-1)?.split, 'Details modified resource open loses split intent'))
  await humanControl(pane)
} }
export const PresentationMatrix: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'The authored trace/resource availability matrix exercises all Details presentation variants, exact 90% → 13% compaction occupancy, known spend rows and missing-field copy with human-copy and contrast negative controls.' } }, args: { cases: presentationCases }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, (pane, item) => assertCraft(pane, item)); await humanControl(paneFor(canvasElement)); await contrastControl(paneFor(canvasElement), args.cases.at(-1)!)
} }
const traceCases = [...sourceCases, example('turn-only context', 400, 'busy', {}, { ...full, series: missing }), example('null-turn context', 400, 'busy', {}, full.turns._tag === 'Known' ? { ...full, turns: { ...full.turns, value: { ...full.turns.value, points: full.turns.value.points.map((point, index) => index === 0 ? { ...point, contextTokens: null, contextPercent: null } : point) } } } : full)]
const bucketCases = widths.map(width => example(`bucket/${width}`, width, 'busy', {}, detailsTraceFixtures.bucketOnly))
export const TraceContracts: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Exact local trace buckets, turns, null context samples and unknown/positive/zero spend meters prove numeric projection and retention contracts; catalog details exposes missions/work, not these trace-series inputs.' } }, args: { cases: [...traceCases, ...bucketCases] }, play: async ({ canvasElement, args }) => {
  assertFixtureFacts()
  const bucketProjection = adaptTrace(detailsTraceFixtures.bucketOnly)
  requireContract(bucketProjection !== undefined, 'Retained buckets lack a chart projection')
  requireContract(detailsTraceFixtures.bucketOnly.turns._tag === 'Unknown' && detailsTraceFixtures.bucketOnly.turns.reason === 'not-covered' && JSON.stringify(detailsTraceFixtures.bucketOnly.series) === JSON.stringify(full.series) && bucketProjection.lane.turnSamples === undefined, 'Bucket-only provider fabricates turns or changes source buckets')
  await eachCase(canvasElement, args.cases, async (pane, item) => {
    const trace = item.trace!
    if (item.name.startsWith('bucket/')) {
      assertContext(pane, trace)
      if (item.name !== bucketCases.at(-1)!.name) return
      const axis = pane.querySelector<HTMLElement>('[data-chart-axis-label]')!, text = axis.textContent
      try { axis.textContent = 'per turn'; await rejectsControl('Context axis renames buckets as turns', () => assertContext(pane, trace)) } finally { axis.textContent = text }
      assertContext(pane, trace)
      return
    }
    const projected = adaptTrace(trace), meter = trace.meters.spend
    requireContract(projected !== undefined, 'Retained sources lack a chart projection')
    requireContract(projected.lane.totalUsd === meter.mainUsd && projected.descendantSpend.totalUsd === meter.subagentUsd, 'Projection loses tagged source totals')
    assertHuman(pane); assertContext(pane, trace)
    if (item.name === traceCases.at(-1)!.name) {
      const chart = pane.querySelector<HTMLElement>('[data-chart-samples]')!, circle = chart.querySelector('circle')!
      circle.remove(); await rejectsControl('Context plot does not match retained source samples', () => assertContext(pane, trace)); chart.querySelector('svg')!.append(circle)
    }
    if (!item.name.includes('/')) return
    const spend = pane.querySelector('section[aria-label="Spend"]')!, text = paintedText(spend), amounts = text.match(/\$\d[\d,.]*/g) ?? []
    const line = spend.querySelector<HTMLElement>('[data-details-not-reported]')
    if (item.name.includes('/unknown/')) requireContract(amounts.length === 0 && line && (line.dataset.detailsNotReported === 'group' || line.querySelector('[data-details-not-reported-field="total spend"]')), 'Unknown spend source renders inferred currency')
    else if (item.name.includes('/zero/')) requireContract(amounts.length > 0 && amounts.every(amount => amount === '$0.00'), 'Known zero spend is absent or fabricated')
    else requireContract(meter.mainUsd._tag === 'Known' && text.includes(`$${meter.mainUsd.value.toFixed(2)}`), 'Known source spend total is omitted')
  })
} }
const scopeCases = widths.flatMap(width => [example(`including/${width}`, width, 'busy', {}, full), example(`self-all-time/${width}`, width, 'busy', {}, { ...full, meters: { ...full.meters, spend: { ...full.meters.spend, usdScope: { _tag: 'Self' }, tokensScope: { _tag: 'Self' }, tokensWindow: { ...zero, value: { _tag: 'AllTime' } } } } }), example(`unreported-window/${width}`, width, 'busy', {}, { ...full, meters: { ...full.meters, spend: { ...full.meters.spend, tokensWindow: missing } } })])
const unreportedScopeCases = widths.map(width => example(`unreported/${width}`, width, 'partial', {}, detailsTraceFixtures.partial))
/** Reported and unreported token scopes each falsify their inline label on their widest pane. */
const scopeControlCases = [scopeCases.at(-1)!.name, unreportedScopeCases.at(-1)!.name]
export const SpendScope: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Authored attributed and unreported trace spend scopes prove exact local main/subagent/session meter relationships and prevent presenting an unattributed total as an attributed value.' } }, args: { cases: [...scopeCases, ...unreportedScopeCases] }, play: ({ canvasElement, args }) => eachCase(canvasElement, args.cases, async (pane, item) => {
  const trace = item.trace!
  assertCraft(pane, item); assertScope(pane, trace)
  if (!scopeControlCases.includes(item.name)) return
  const label = pane.querySelector<HTMLElement>('[data-details-token-label]')!, text = label.textContent
  try { label.textContent = 'Cumulative tokens · invented session scope'; const reason = trace.meters.spend.tokensScope._tag === 'Unknown' ? 'Unreported total makes an unproven scope claim' : 'Reported token scope is absent from its inline label'; if (reason.startsWith('Unreported')) label.textContent = 'Cumulative tokens incl. subagents'; await rejectsControl(reason, () => assertScope(pane, trace)) } finally { label.textContent = text }
  assertScope(pane, trace)
}) }
/** One mount per state and width owns one-line rows, whole-or-absent metadata, title-first truncation and chevron columns. */
const geometryCases = (['busy', 'partial'] as const).flatMap(state => [240, ...widths].map(width => example(`${state}/${width}`, width, state)))
export const ResourceGeometry: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Exact local file diff counts, CI labels and PR statuses probe one-line kit rows, whole-or-absent metadata and title-first truncation from 240px to 520px; replacing labels would change the measured geometry contract.' } }, args: { cases: geometryCases }, play: ({ canvasElement, args }) => eachCase(canvasElement, args.cases, async (pane, item) => {
  assertRows(pane)
  const fitted = widths.includes(item.width)
  if (fitted) assertCraft(pane, item)
  if (fitted && item.state === 'busy') assertChevron(pane)
  if (item.name === 'busy/320') {
    const row = [...pane.querySelectorAll<HTMLElement>('[data-thing-openable]')].find(row => [...row.querySelectorAll<HTMLElement>('[data-thing-meta-item]')].some(meta => meta.getBoundingClientRect().width > 0 && meta.getBoundingClientRect().top < meta.parentElement!.getBoundingClientRect().bottom - 0.5))
    requireContract(row, 'Row negative controls have no row with shown metadata')
    const title = row.querySelector<HTMLElement>('[data-thing-title]')!, maxWidth = title.style.maxWidth, children = [...title.childNodes]
    // Flatten path segments for this injected overflow: their own clipping otherwise hides the outer title's scrollWidth.
    try { title.replaceChildren(document.createTextNode(title.textContent!)); title.style.maxWidth = '8px'; await rejectsControl('Title truncates beside shown metadata', () => assertRows(pane)) } finally { title.replaceChildren(...children); title.style.maxWidth = maxWidth }
    const padding = row.style.paddingBlock
    try { row.style.paddingBlock = '12px'; await rejectsControl('Resource row wraps beyond one line', () => assertRows(pane)) } finally { row.style.paddingBlock = padding }
    assertRows(pane)
  }
  if (item.name === 'busy/520') {
    const svg = pane.querySelector<SVGElement>('section[aria-label="Workspace"] button[aria-expanded] svg')!, original = svg.style.transform
    try { svg.style.transform = 'translateX(2px)'; await rejectsControl('Section chevrons do not share the Overview column', () => assertChevron(pane)) } finally { svg.style.transform = original }
    assertChevron(pane)
  }
}) }
export const OrientationAnchor: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Host-controlled idle/new/absent Details fixtures isolate first-section anchoring and placement of new-agent orientation inside Overview, with a 4px margin negative control; catalog details has no equivalent presentation-state lifecycle.' } }, args: { cases: widths.map(width => example(`orientation/${width}`, width, 'idle')) }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, async pane => {
    const top = () => { pane.scrollTop = 0; return pane.querySelector('section')!.getBoundingClientRect().top - pane.getBoundingClientRect().top }
    const assertAnchor = () => requireContract(Math.abs(top()) <= 0.1, 'Orientation displaces the first Details section')
    for (const state of ['idle', 'new', 'absent']) { await userEvent.click(within(canvasElement).getByRole('button', { name: `Set ${state}` })); await frames(); assertAnchor(); if (state === 'new') requireContract(pane.querySelector('section[aria-label="Overview"] [data-details-orientation]'), 'New orientation is outside Overview') }
    const section = pane.querySelector<HTMLElement>('section')!, original = section.style.marginTop
    try { section.style.marginTop = '4px'; await rejectsControl('Orientation displaces the first Details section', assertAnchor) } finally { section.style.marginTop = original }
    assertAnchor()
  })
} }
export const ResourceStates: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'The fixed busy/partial/new/unknown/idle/absent comparison retains exact trace/resource availability and compaction/spend assertions plus copy, contrast and one-line row negative controls.' } }, args: { cases: resourceStateCases, comparison: true }, play: async ({ canvasElement, args }) => {
  await ready(canvasElement)
  const panes = [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="details-pane"]')]
  requireContract(panes.length === args.cases.length, 'Resource-state comparison omits a production Details state')
  for (const [index, pane] of panes.entries()) { assertCraft(pane, args.cases[index]!); if (args.cases[index]!.state === 'busy') assertRows(pane) }
  await humanControl(panes[0]!); await contrastControl(panes[0]!, args.cases[0]!)
} }

const e3Traces: readonly { name: string; trace: TraceSeries }[] = [
  { name: 'full', trace: full }, { name: 'partial', trace: detailsTraceFixtures.partial }, { name: 'unknown', trace: detailsTraceFixtures.unknown },
  { name: 'bucket-only', trace: detailsTraceFixtures.bucketOnly },
  { name: 'unattributed', trace: { ...full, subagents: missing, meters: { ...full.meters, spend: { ...full.meters.spend, usdScope: { _tag: 'Unknown', reason: 'not-attributed' }, mainUsd: missing, subagentUsd: missing } } } },
]
const e3Cases = e3Traces.flatMap(({ name, trace }) => (['C1', 'C2', 'C3'] as const).flatMap((context, index) => (['S1', 'S2', 'S3'] as const).map(spend => example(`${name}/${context}/${spend}`, widths[index]!, 'busy', { context, spend }, trace))))
export const E3ChartMatrix: Story = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Exact full, partial, unknown, bucket-only and unattributed local traces cross C1/C2/C3 and S1/S2/S3 to prove known chart rows survive and unavailable fields retain explanations; these trace availability/scope inputs are absent from catalog details.' } }, args: { cases: e3Cases }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, (pane, item) => {
    assertHuman(pane)
    assertContext(pane, item.trace!)
    for (const name of ['Context', 'Spend']) {
      const section = pane.querySelector<HTMLElement>(`section[aria-label="${name}"]`)!
      requireContract(section, 'E3 omits its chart section')
      const body = section.querySelector(`[data-details-chart="${name === 'Context' ? item.variants.context : item.variants.spend}"]`)!
      const data = hasDetailsChartData({ trace: item.trace!, context: item.variants.context, spend: item.variants.spend, section: name === 'Context' ? 'context' : 'spend', empty: 'E3' })
      if (!data) requireContract(body.querySelector('[data-details-not-reported="group"]'), 'E3 hides unavailable field explanations')
      else requireContract(body.querySelector('[data-details-data-row],dl,svg[role="img"]'), 'E3 drops known chart rows')
    }
    if (item.trace!.meters.spend.tokens._tag === 'Known') assertScope(pane, item.trace!)
  })
  await humanControl(paneFor(canvasElement))
} }

export const SharedResourceActionNames:Story={
  parameters:{scenario:{_tag:'contract-fixture',reason:'Fixed PR #42, +10/−2 totals and src/rows.ts +1/−1 file data prove painted accessible-name order, basename/full-path descriptions and exact file-open identity across resource cards and chips.'}},
  render:function Render({scheme}){
    const resource=sharedResources[1];requireContract(resource,'Missing shared resource fixture')
    const file=resource.files[0];requireContract(file,'Missing shared file fixture')
    const [opened,setOpened]=React.useState<string>()
    return <main aria-label="Shared resource actions" {...stylex.props(styles.root,...baselineTheme,...(scheme==='light'?lightTheme:[]))} data-shared-opened={opened}><ThemePortal>{(['RC-1','RC-2','RC-3'] as const).map(variant=><section key={variant} aria-label={`Resource ${variant}`}><ResourceCardV1 resource={resource} variant={variant} onOpen={(_,selected)=>setOpened(selected?.path??resource.label)}/></section>)}<section aria-label="Inline resource chip"><ResourceChipV1 resource={resource} file={file} onOpen={(_,selected)=>setOpened(selected?.path)}/></section></ThemePortal></main>
  },
  play:async({canvasElement})=>{
    const root=within(canvasElement).getByRole('main',{name:'Shared resource actions'})
    for(const variant of ['RC-1','RC-2']){
      const section=within(root).getByRole('region',{name:`Resource ${variant}`})
      const action=within(section).getByRole('button',{name:name=>name.startsWith('PR #42 · Keep selection stable')&&name.endsWith('Open')})
      const assertName=()=>requireContract(within(section).queryByRole('button',{name:name=>name.startsWith('PR #42 · Keep selection stable')&&name.includes('+10')&&name.includes('−2')&&name.endsWith('Open')})===action,'Resource card name omits visible text or changes its order')
      assertName()
      action.setAttribute('aria-label','Open PR #42 · Keep selection stable')
      try{await rejectsControl('Resource card name omits visible text or changes its order',assertName)}finally{action.removeAttribute('aria-label')}
      assertName()
    }
    for(const name of ['Resource RC-3','Inline resource chip']){
      const section=within(root).getByRole('region',{name})
      const chip=within(section).getByRole('button',{name:label=>label.startsWith('rows.ts')&&label.includes('+1')&&label.includes('−1')})
      const assertChip=()=>requireContract(within(section).queryByRole('button',{name:label=>label.startsWith('rows.ts')&&!label.includes('src/')&&label.includes('+1')&&label.includes('−1')})===chip&&chip.getAttribute('aria-describedby')?.split(/\s+/).some(id=>chip.ownerDocument.getElementById(id)?.textContent==='src/rows.ts'),'File chip name or full-path description differs from painted basename')
      assertChip();expect(chip).toHaveAccessibleDescription('src/rows.ts')
      chip.setAttribute('aria-label','Open src/rows.ts')
      try{await rejectsControl('File chip name or full-path description differs from painted basename',assertChip)}finally{chip.removeAttribute('aria-label')}
      assertChip()
      const describedBy=chip.getAttribute('aria-describedby');requireContract(describedBy,'Missing chip description reference')
      chip.removeAttribute('aria-describedby')
      try{await rejectsControl('File chip name or full-path description differs from painted basename',assertChip)}finally{chip.setAttribute('aria-describedby',describedBy)}
      assertChip()
      await userEvent.click(chip);await waitFor(()=>requireContract(root.dataset.sharedOpened==='src/rows.ts','Shared chip open callback lost full file identity'))
      await userEvent.unhover(chip);chip.blur()
    }
  },
}
