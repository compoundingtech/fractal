import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, waitFor, within } from 'storybook/test'
import { Button } from './kit'
import { Workbench, type WorkbenchProps } from './assistant-ui/workbench/Workbench'
import { group } from './assistant-ui/workbench/workbench-model'
import { workbenchResources } from './assistant-ui/workbench/workbench-fixtures'
import { DetailsPane, type DetailsVariants, type DetailsLayoutIntent } from './assistant-ui/details/DetailsPane'
import { traceHistorySection } from './assistant-ui/details/DetailsResources'
import { hasDetailsChartData } from './assistant-ui/details/DetailsCharts'
import { makeDetailsFixture, type DetailsState } from './assistant-ui/details/fixtures'
import { detailsTraceFixtures, traceForDetailsState } from './assistant-ui/details/session-trace-provider'
import { adaptTrace } from './assistant-ui/details/trace/adapter'
import { sessionTraceFixture } from './assistant-ui/details/trace/fixture'
import type { TraceSeries } from './assistant-ui/details/trace-series'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { surfaceVars as surface } from './assistant-ui/composition-tokens.stylex'

const variants: DetailsVariants = { overview: 'O2', context: 'C2', spend: 'S1', resources: 'R2', empty: 'E3' }
const widths = [320, 400, 520]
const full = detailsTraceFixtures.full
const missing = { _tag: 'Unknown', reason: 'not-covered' } as const
const zero = { _tag: 'Known', value: 0, source: 'series', asOfMs: 1799999999000 } as const
const zeroCost = { uncachedInputUsd: 0, cacheReadUsd: 0, cacheWriteUsd: 0, outputUsd: 0 }
interface Case { name: string; width: number; state: DetailsState; variants: DetailsVariants; trace?: TraceSeries }
function example(name: string, width = 400, state: DetailsState = 'busy', changes: Partial<DetailsVariants> = {}, trace?: TraceSeries): Case {
  return { name, width, state, variants: { ...variants, ...changes }, ...(trace === undefined ? {} : { trace }) }
}
const resourceCases = (forms: readonly DetailsVariants['resources'][], states: readonly DetailsState[], sizes: readonly number[]) => forms.flatMap(resources => states.flatMap(state => sizes.map(width => example(`${resources}/${state}/${width}`, width, state, { resources }))))
const sourceCases = (['turn-only', 'bucket-only'] as const).flatMap(source => (['unknown', 'positive', 'zero'] as const).flatMap(amounts => (['S1', 'S2', 'S3'] as const).map(spend => {
  const meter = amounts === 'unknown' ? { ...full.meters.spend, totalUsd: missing, mainUsd: missing, subagentUsd: missing, tokens: missing, split: missing }
    : amounts === 'zero' ? { ...full.meters.spend, totalUsd: zero, mainUsd: zero, subagentUsd: zero, tokens: zero, split: { ...zero, value: zeroCost } } : full.meters.spend
  const trace: TraceSeries = { ...full, meters: { ...full.meters, spend: meter }, subagents: missing,
    series: source === 'turn-only' ? missing : amounts === 'zero' && full.series._tag === 'Known' ? { ...full.series, value: { ...full.series.value, points: full.series.value.points.map(point => ({ ...point, cost: zeroCost, subagentUsd: 0 })) } } : full.series,
    turns: source === 'bucket-only' ? missing : full.turns }
  return example(`${source}/${amounts}/${spend}`, 400, 'busy', { spend, empty: 'E2' }, trace)
})))
const presentationCases: Case[] = [
  ...(['new', 'busy', 'partial', 'unknown', 'idle', 'absent'] as const).flatMap(state => (['C1', 'C2', 'C3'] as const).map((context, index) => example(`${state}/${context}`, widths[index]!, state, { context, spend: (['S1', 'S2', 'S3'] as const)[index]!, resources: (['R1', 'R2', 'R3'] as const)[index]! }))),
  ...resourceCases(['R1', 'R2', 'R3'], ['busy'], [320]),
  ...widths.map(width => example(`new/${width}`, width, 'new')),
  ...(['overview', 'context', 'spend', 'resources', 'empty'] as const).flatMap(dimension => ['1', '2', '3'].map(index => example(`${dimension}/${index}`, 400, 'busy', { [dimension]: `${dimension[0]!.toUpperCase()}${index}` } as Partial<DetailsVariants>))),
]
const allStatesCases = resourceCases(['R1', 'R2', 'R3'], ['busy', 'partial', 'new', 'unknown', 'idle', 'absent'], [400])
interface Args extends WorkbenchProps { cases: readonly Case[]; comparison: boolean }
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', minHeight: 0, height: 560, overflow: 'hidden', backgroundColor: surface.canvas },
  controls: { display: 'flex', flexWrap: 'wrap', gap: 4, backgroundColor: surface.canvas },
  comparison: { display: 'flex', flexWrap: 'wrap', gap: 16, backgroundColor: surface.canvas },
})
const meta = {
  title: 'Fractal UI/Details Resources',
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
      <header {...stylex.props(styles.controls)}>{!comparison && cases.map((item, index) => <Button key={item.name} size="sm" variant="quiet" onPress={() => { setSelected(index); setStateOverride(undefined) }}>{item.name}</Button>)}
        {(['idle', 'new', 'absent'] as const).map(state => <Button key={state} size="sm" variant="quiet" onPress={() => setStateOverride(state)}>Set {state}</Button>)}
      </header>
      <div {...stylex.props(comparison && styles.comparison)}>{mounted.map(item => <section key={item.name} aria-label={item.name} data-testid="resource-stage" style={{ width: item.width }} {...stylex.props(styles.root, ...baselineTheme, ...(args.scheme === 'light' ? lightTheme : []))}>
        <ThemePortal><Workbench {...args} workspaceId={`${args.workspaceId}-${item.name}`} renderPane={() => <DetailsPane variants={item.variants} state={stateOverride ?? item.state} trace={item.trace} intent={intent}
          onIntentChange={next => { setIntent(next); setIntentWrites(count => count + 1) }} onOpen={(key, split) => setOpens(current => [...current, { key, split }])} onRetry={() => setStateOverride('busy')} />}/></ThemePortal>
      </section>)}</div>
    </div>
  },
} satisfies Meta<Args>
export default meta
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
    await userEvent.click(within(canvas).getByRole('button', { name: item.name, exact: true }))
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
  for (const section of pane.querySelectorAll('section')) {
    const notes = [...section.querySelectorAll<HTMLElement>('[data-details-partial-note]')].filter(shown)
    if (pane.querySelector('[data-details-empty="E3"]')) requireContract(notes.length <= 1 && [...section.querySelectorAll<HTMLElement>('[data-details-reason]')].filter(shown).length <= 1, 'Section repeats its missing-data explanation')
    requireContract(notes.every(note => !/Some values|Some details/i.test(note.textContent ?? '') && /Scope|Accumulation window|Line counts|Status|Turn samples|cost|tokens|counts|samples|window|context/i.test(note.textContent ?? '')), 'Partial explanation does not identify its missing field')
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
  for (const reason of pane.querySelectorAll<HTMLElement>('[data-details-reason]')) if (shown(reason)) requireContract(contrast(reason) >= 4.5, 'Unavailable explanation contrast below 4.5:1')
  for (const name of ['Workspace', 'Pull requests', 'CI runs']) {
    const section = pane.querySelector(`section[aria-label="${name}"]`), heading = section?.querySelector('[data-details-heading]'), meta = section?.querySelector('[data-resource-meta]')
    if (heading && meta) requireContract(contrast(heading) > contrast(meta), 'Resource heading is no stronger than metadata')
  }
  if (item.state === 'new' && item.variants.empty === 'E3') {
    const orientation = pane.querySelectorAll('[data-details-orientation]')
    requireContract(orientation.length === 1 && /context.*spend.*resources/i.test(orientation[0]?.textContent ?? ''), 'New agent lacks one calm Details orientation')
    requireContract(!pane.querySelector('[data-details-source-note]'), 'New agent repeats source unavailability')
    for (const section of pane.querySelectorAll('section')) requireContract(section.querySelector('strong,dd,[data-details-data-row],[data-details-resource-row],svg[role="img"]'), 'New agent offers an empty disclosure')
  }
  if (item.state === 'unknown') for (const name of ['Context', 'Spend']) requireContract(pane.querySelector(`section[aria-label="${name}"] [data-details-reason]`), 'Unknown source omits a field-specific explanation')
  const history = pane.querySelector('section[aria-label="History"]')
  if (item.state === 'busy' || item.state === 'idle') requireContract(history?.querySelector('button[aria-label^="Open Incarnation"]'), 'Known trace history is absent')
  const changes = pane.querySelector('[aria-label="Compaction context changes"]')
  if (changes) requireContract(/897\.2K → 133K/.test(changes.textContent ?? ''), 'Compaction includes the straddling bucket')
  if (item.state === 'busy' && item.variants.empty === 'E3' && item.variants.spend !== 'S1') {
    const chart = pane.querySelector(`[data-details-chart="${item.variants.spend}"]`), meter = chart?.querySelector('dl[aria-label="Trace spend meters"]')
    requireContract(meter?.textContent?.includes('Cumulative tokens') && meter.textContent.includes('Subagents') && chart?.querySelector('[aria-label="Subagent spend breakdown"]')?.textContent?.includes('Worker 1'), 'E3 chart omits known spend meter rows')
    requireContract(chart?.firstElementChild?.contains(meter!), 'E3 meter is not above its chart')
  }
}
async function humanControl(pane: HTMLElement) {
  const target = pane.querySelector<HTMLElement>('strong')!
  requireContract(target, 'Human-copy negative control lacks known identity')
  const original = target.textContent
  try { target.textContent = 'Unknown'; await rejectsControl('Details exposes a tag or generic missing-data value', () => assertHuman(pane)) } finally { target.textContent = original }
  assertHuman(pane)
}
function intrinsic(element: HTMLElement, pane: HTMLElement) {
  const copy = element.cloneNode(true) as HTMLElement, css = getComputedStyle(element)
  copy.className = ''; copy.removeAttribute('id')
  copy.style.cssText = `position:fixed;visibility:hidden;pointer-events:none;width:max-content;min-width:0;max-width:none;white-space:nowrap;display:${element.hasAttribute('data-resource-meta-value') ? 'inline-flex' : 'inline-block'};font:${css.font};letter-spacing:${css.letterSpacing};`
  pane.append(copy); const width = copy.getBoundingClientRect().width; copy.remove(); return width
}
function assertFit(pane: HTMLElement, item: Case) {
  const rows = pane.querySelectorAll<HTMLElement>('[data-details-resource-row],[data-resource-row]')
  requireContract(rows.length > 0, 'Resource fit assertions have no measured rows')
  for (const row of rows) {
    if (row.closest('[inert]')) continue
    const title = row.querySelector<HTMLElement>('[data-details-resource-title],[data-resource-title]')
    if (!title) continue
    const box = row.getBoundingClientRect(), css = getComputedStyle(row), gap = parseFloat(css.columnGap) || 0
    const width = row.clientWidth - parseFloat(css.paddingLeft) - parseFloat(css.paddingRight), titleWidth = intrinsic(title, pane), reservation = Math.min(titleWidth, width * 0.6 + gap)
    if (item.width <= 322) requireContract(title.getBoundingClientRect().width >= reservation - 0.5, 'Narrow title loses intrinsic/60% priority')
    requireContract(title.getBoundingClientRect().right <= box.right + 1, 'Resource title escapes its row')
    for (const slot of row.querySelectorAll<HTMLElement>('[data-resource-meta]')) {
      const value = slot.querySelector<HTMLElement>('[data-resource-meta-value]')
      requireContract(value, 'Metadata lacks a whole-value element')
      if (!value.textContent?.trim()) continue
      const slotBox = slot.getBoundingClientRect(), valueBox = value.getBoundingClientRect(), visible = shown(slot) && shown(value)
      const controls = [...row.children].filter(child => child !== title && child !== slot && getComputedStyle(child).position !== 'absolute')
      const available = width - reservation - controls.reduce((sum, child) => sum + child.getBoundingClientRect().width, 0) - gap * (controls.length + 1)
      requireContract(visible === (intrinsic(value, pane) <= available + 0.5), 'Metadata is not shown whenever it fits whole')
      requireContract(visible || slotBox.width <= 0.01, 'Absent metadata reserves horizontal space')
      if (!visible && title.dataset.resourceTitleLines !== 'wrap' && titleWidth > title.getBoundingClientRect().width + 0.5) requireContract(title.getBoundingClientRect().width >= width - controls.reduce((sum, child) => sum + child.getBoundingClientRect().width, 0) - gap * controls.length - 0.5, 'Title truncates beside free space')
      requireContract(!visible || (valueBox.left >= slotBox.left - 0.01 && valueBox.right <= slotBox.right + 0.01), 'Metadata is partially shown')
      requireContract(slot.dataset.resourceMetaFit === (visible ? 'whole' : 'absent'), 'Metadata is neither whole nor absent')
      requireContract(visible || value.getAttribute('aria-hidden') === 'true', 'Omitted metadata remains accessible')
      if (slot.dataset.resourceMeta === 'descriptive') requireContract(getComputedStyle(value).fontFamily === getComputedStyle(pane).fontFamily, 'Descriptive metadata lacks proportional UI font')
      if (slot.dataset.resourceMeta === 'numeric') for (const element of [slot, value, ...value.querySelectorAll('*')]) requireContract(getComputedStyle(element).textOverflow !== 'ellipsis', 'Numeric metadata is ellipsized')
    }
  }
  const metas = [...pane.querySelectorAll<HTMLElement>('[data-resource-meta]')]
  if (item.variants.resources === 'R1' && item.width === 320) for (const pair of ['+1-1', '+2-1', '+12-4']) requireContract(metas.some(meta => meta.textContent?.replace(/−/g, '-').replace(/\s/g, '') === pair && meta.dataset.resourceMetaFit === 'whole'), 'R1 320 hides fitting numeric counts')
  if (item.variants.resources === 'R2') {
    if (item.width === 320) requireContract(metas.some(meta => meta.textContent === 'CI run · succeeded' && meta.dataset.resourceMetaFit === 'whole'), 'R2 320 hides whole succeeded CI status')
    if (item.width === 520) for (const status of ['Open · sample-app · checks passed', 'Draft · sample-app · checks running']) requireContract(metas.some(meta => meta.textContent === status && meta.dataset.resourceMetaFit === 'whole'), 'R2 520 hides fitting PR status')
    for (const row of pane.querySelectorAll<HTMLElement>('section[aria-label="Subagents"] [data-details-resource-row]')) requireContract(row.getBoundingClientRect().height <= 24.5, 'R2 subagent row is not dense 24px')
  }
  for (const [label, icon] of [['Observation coverage', 'clock'], ['Projection checks', 'check']]) requireContract([...rows].some(row => row.textContent?.includes(label!) && row.querySelector('[data-details-resource-icon]')?.getAttribute('data-details-resource-icon') === icon), 'CI resource lacks its pending/succeeded kit icon')
}
function probesFor(pane: HTMLElement) { return [...pane.querySelectorAll<HTMLElement>('*')].filter(node => { const css = getComputedStyle(node); return css.position === 'absolute' && css.visibility === 'hidden' }) }
function assertContainment(pane: HTMLElement) {
  for (const probe of probesFor(pane)) { const card = probe.closest('[data-resource-card],[data-testid="taste-agent-row"]'); requireContract(card && probe.offsetParent && card.contains(probe.offsetParent), 'Intrinsic-fit probe offsetParent escapes its resource card') }
  const surface = pane.closest<HTMLElement>('[data-drop-group]')
  requireContract(surface, 'Details has no production AgentDropSurface')
  requireContract(surface.scrollHeight === surface.clientHeight, 'AgentDropSurface overflows vertically')
}
async function containmentGate(pane: HTMLElement) {
  assertContainment(pane)
  const probes = probesFor(pane)
  requireContract(probes.length > 0, 'Containment gate exercises no intrinsic-fit probes')
  const surface = pane.closest<HTMLElement>('[data-drop-group]')!, probe = probes.at(-1)!, card = probe.closest<HTMLElement>('[data-resource-card],[data-testid="taste-agent-row"]')!
  const scrollTop = pane.scrollTop, surfaceTop = surface.scrollTop, distance = probe.getBoundingClientRect().top - card.getBoundingClientRect().top
  pane.scrollTop = pane.scrollHeight; await frames()
  requireContract(Math.abs(probe.getBoundingClientRect().top - card.getBoundingClientRect().top - distance) < 0.01, 'Intrinsic-fit probe fails to follow its card while scrolling')
  probe.scrollIntoView({ block: 'nearest' }); await frames()
  requireContract(surface.scrollTop === surfaceTop, 'Hidden probe scrolls the outer AgentDropSurface')
  pane.scrollTop = scrollTop
  const cards = [...pane.querySelectorAll<HTMLElement>('[data-resource-card]')].map(element => ({ element, position: element.style.getPropertyValue('position'), priority: element.style.getPropertyPriority('position') }))
  try {
    for (const { element } of cards) element.style.setProperty('position', 'static', 'important')
    await rejectsControl('Intrinsic-fit probe offsetParent escapes its resource card', () => assertContainment(pane))
  } finally { for (const { element, position, priority } of cards) position ? element.style.setProperty('position', position, priority) : element.style.removeProperty('position') }
  assertContainment(pane)
}
function assertIdentity(pane: HTMLElement) {
  const titles = [...pane.querySelectorAll<HTMLElement>('[data-resource-title-lines]:not([data-resource-title-lines="1"])')]
  requireContract(titles.some(title => title.textContent === 'Review the source projection'), 'Real mission identity is missing')
  for (const title of titles) {
    const css = getComputedStyle(title), box = title.getBoundingClientRect(), row = title.closest('[data-details-resource-row]')!.getBoundingClientRect()
    requireContract(css.webkitLineClamp === 'none' && title.scrollHeight <= title.clientHeight + 0.5 && title.scrollWidth <= title.clientWidth + 0.5, 'Resource identity is clamped instead of wrapping in full')
    requireContract(box.bottom <= row.bottom + 0.5 && box.top >= row.top - 0.5, 'Wrapped identity overflows its resource row')
  }
}
function textLines(title: HTMLElement) {
  const text = title.firstChild
  requireContract(text?.nodeType === Node.TEXT_NODE, 'Wrapped identity lacks its text node')
  const node = text as Text, range = document.createRange(), lines: { top: number; text: string }[] = []
  for (let index = 0; index < node.length; index++) { range.setStart(node, index); range.setEnd(node, index + 1); const rect = range.getClientRects()[0]; if (!rect || rect.width === 0) continue; let line = lines.find(line => Math.abs(line.top - rect.top) < 4); if (!line) { line = { top: rect.top, text: '' }; lines.push(line) } line.text += node.data[index] }
  return lines.map(line => line.text)
}
function assertBranch(pane: HTMLElement) {
  const titles = [...pane.querySelectorAll<HTMLElement>('[data-resource-title-lines]:not([data-resource-title-lines="1"])')]
  requireContract(titles.some(title => title.closest('[data-details-resource-row]')?.querySelector('[data-resource-meta]')?.textContent?.includes('Declared branch')), 'Declared branch row is missing')
  for (const title of titles) {
    requireContract(title.closest('[data-details-resource-row]')?.getAttribute('aria-label')?.includes(title.textContent!), 'Wrapped identity lacks its full accessible name')
    const lines = textLines(title)
    requireContract(lines.length < 2 || lines.at(-1)!.trim().split(/[\s/_-]+/).filter(Boolean).length >= 2, 'Wrapped identity leaves an orphaned final word')
  }
}
function assertMissions(pane: HTMLElement) {
  const labels = makeDetailsFixture('busy').sections.flatMap(section => section.items._tag === 'Known' ? section.items.value.filter(resource => resource.uri.startsWith('mission:')).map(resource => resource.card.label) : [])
  requireContract(labels.length >= 2, 'Mission title fixture is incomplete')
  for (const label of labels) {
    const title = [...pane.querySelectorAll<HTMLElement>('[data-details-resource-title]')].find(title => title.textContent === label)
    requireContract(title, 'Real mission title is missing')
    const box = title.getBoundingClientRect(), range = document.createRange(); range.selectNodeContents(title)
    requireContract([...range.getClientRects()].filter(rect => rect.width > 0).every(rect => rect.left >= box.left - 0.5 && rect.right <= box.right + 0.5 && rect.top >= box.top - 0.5 && rect.bottom <= box.bottom + 0.5) && title.scrollWidth <= title.clientWidth + 0.5 && getComputedStyle(title).textOverflow !== 'ellipsis', 'Mission title is truncated')
  }
}
function assertChevron(pane: HTMLElement) {
  const columns = [...pane.querySelectorAll('section')].map(section => ({ name: section.getAttribute('aria-label'), svg: section.querySelector('button[aria-expanded] svg') }))
  for (const name of ['Overview', 'Context', 'Spend', 'Workspace', 'Reviews', 'Files touched', 'Subagents']) requireContract(columns.some(column => column.name === name && column.svg), 'Required section chevron is missing')
  const reference = columns.find(column => column.name === 'Overview')!.svg!.getBoundingClientRect().x
  requireContract(columns.every(column => column.svg && Math.abs(column.svg.getBoundingClientRect().x - reference) <= 0.5), 'Section chevrons do not share the Overview column')
}
function assertScope(pane: HTMLElement, trace: TraceSeries) {
  const spend = pane.querySelector<HTMLElement>('section[aria-label="Spend"]')!, label = spend.querySelector('[data-details-token-label]')?.textContent ?? '', period = spend.querySelector('[data-details-token-period]')?.textContent ?? '', meter = trace.meters.spend
  requireContract(label.includes('Cumulative tokens'), 'Known cumulative token meter is missing')
  const scopeNote = spend.querySelector<HTMLElement>('[data-details-scope-note]')
  if (scopeNote) requireContract(scopeNote.getBoundingClientRect().height <= parseFloat(getComputedStyle(scopeNote).lineHeight) + 0.5, 'Unreported scope note is not one quiet line')
  if (meter.scope._tag === 'Unknown') {
    requireContract(!/main session|including subagents|excludes subagents|this agent|\bpartial\b/i.test(spend.textContent ?? '') && !spend.querySelector('[aria-label="Subagent spend breakdown"],[aria-label="Session and descendant spend"]'), 'Unreported total makes an unproven scope claim')
    const notes = spend.querySelectorAll('[data-details-partial-note]')
    requireContract(notes.length === 1 && notes[0]?.textContent === "Scope of this total isn't reported by this source.", 'Unreported token total lacks its exact scope explanation')
  } else {
    requireContract(label === `Cumulative tokens · ${meter.scope._tag === 'Self' ? 'this agent' : 'this agent + subagents'}`, 'Reported token scope is absent from its inline label')
    if (meter.tokensWindow._tag === 'Known') {
      const window = meter.tokensWindow.value
      if (window._tag === 'AllTime') requireContract(period === 'all time', 'Reported all-time window is absent')
      else { const from = new Date(window.fromMs).toISOString(), to = new Date(window.toMs).toISOString(); requireContract(period.includes(from.slice(0, 10)) && period.includes(from.slice(11, 16)) && period.includes(to.slice(11, 16)) && period.includes('UTC'), 'Reported accumulation interval is absent from its separate date line') }
    } else requireContract(spend.textContent?.includes("Accumulation window isn't reported by this source.") && !spend.textContent.includes('all time'), 'Unreported accumulation window claims all-time scope')
  }
}
function assertContext(pane: HTMLElement, trace: TraceSeries) {
  const projection = adaptTrace(trace), chart = pane.querySelector<HTMLElement>('[data-chart-samples]')
  const compact = pane.querySelector('[data-details-chart="C2"]') !== null
  const samples = projection.lane.turnSamples !== undefined && (compact || projection.lane.samples.length === 0) ? projection.lane.turnSamples : projection.lane.samples
  if (!pane.querySelector('[data-details-chart="C1"]') && samples.some(sample => sample.contextTokens !== null)) requireContract(chart, 'Context plot does not match retained source samples')
  if (!chart) return
  const points = [...chart.querySelectorAll('circle')]
  requireContract(points.length > 0 && points.length === samples.filter(sample => sample.contextTokens !== null).length && Number(chart.dataset.chartSourceCount) === samples.length, 'Context plot does not match retained source samples')
  const kind = projection.lane.turnSamples !== undefined && (compact || projection.lane.samples.length === 0) ? 'turn' : 'bucket'
  requireContract(chart.dataset.chartSampleKind === kind, 'Context plot mislabels source granularity')
  if (compact) {
    const axis = chart.querySelector('[data-chart-axis-label]')?.textContent ?? ''
    requireContract(kind === 'turn' ? axis.includes('per turn') : axis.includes('per minute') && !/per turn/i.test(pane.innerText), 'Context axis renames buckets as turns')
    if (kind === 'turn' && points.length > 2 && samples.every(sample => sample.contextTokens !== null)) { const xs = points.map(point => Number(point.getAttribute('cx'))), step = (xs.at(-1)! - xs[0]!) / (xs.length - 1); requireContract(xs.slice(1).every((x, index) => Math.abs(x - xs[index]! - step) <= 0.001), 'Per-turn context samples are not spaced by turn index') }
  }
}
function assertFixtureFacts() {
  requireContract(full.nativeSessionId._tag === 'Known' && full.turns._tag === 'Known' && full.series._tag === 'Known', 'Full trace fixture lacks retained sources')
  const nativeSessionId = full.nativeSessionId.value
  const requests = sessionTraceFixture.requests.filter(request => request.sessionId === nativeSessionId).toSorted((a, b) => a.ts - b.ts)
  requireContract(full.meters.context.usedTokens._tag === 'Known' && full.meters.context.usedTokens.source === 'st' && full.meters.context.usedTokens.value === 61206 && full.subagents._tag === 'Known' && full.meters.spend.scope._tag === 'IncludingSubagents', 'Full trace fixture loses snapshot provenance or attribution')
  requireContract(requests.length === 17 && full.turns.value.points.length === requests.length && full.turns.value.points.every((point, index) => point.turnIndex === index + 1 && point.tMs === requests[index]?.ts) && requests.every(request => request.requests === 1), 'Trace turns are not the actual source requests')
  const partial = detailsTraceFixtures.partial, unknown = detailsTraceFixtures.unknown
  requireContract(partial.series._tag === 'Unknown' && partial.meters.spend.mainUsd._tag === 'Unknown' && partial.meters.spend.subagentUsd._tag === 'Unknown' && partial.meters.spend.scope._tag === 'Unknown' && partial.meters.spend.scope.reason === 'not-attributed' && partial.meters.spend.tokens._tag === 'Known' && partial.meters.spend.tokens.source === 'st', 'Partial trace fixture invents attribution')
  requireContract(unknown.nativeSessionId._tag === 'Unknown' && unknown.meters.spend.totalUsd._tag === 'Unknown', 'Unknown trace fixture invents identity or spend')
  const projection = adaptTrace(full), compaction = projection.lane.compactions[0], history = traceHistorySection(full)
  requireContract(compaction?.before === 897170 && compaction.after === 132952, 'Compaction includes the straddling bucket')
  requireContract(full.history._tag === 'Known' && history.items._tag === 'Known' && history.items.value.length === full.history.value.length && history.items.value[0]?.uri === `session:${full.history.value[0]?.nativeSessionId}` && traceHistorySection(partial).items._tag === 'Unknown', 'History projection loses source identity or unavailability')
  const turnOnly = adaptTrace({ ...full, series: missing })
  requireContract(turnOnly.lane.samples.length === 0 && turnOnly.lane.turnSamples?.length === requests.length && turnOnly.lane.cumulativeValues.length === 0, 'Turn-only trace fabricates bucket spend')
  const nullTurn = adaptTrace({ ...full, turns: { ...full.turns, value: { ...full.turns.value, points: full.turns.value.points.map((point, index) => index === 0 ? { ...point, contextTokens: null } : point) } } })
  requireContract(nullTurn.lane.turnSamples?.[0]?.contextTokens === null, 'Null context observation is fabricated')
  for (const context of ['C1', 'C2', 'C3'] as const) for (const spend of ['S1', 'S2', 'S3'] as const) for (const section of ['context', 'spend'] as const) {
    requireContract(!hasDetailsChartData({ trace: unknown, context, spend, section }) && hasDetailsChartData({ trace: full, context, spend, section }), 'Chart availability confuses missing and known data')
  }
}

export const Interactions: Story = { play: async ({ canvasElement }) => {
  await ready(canvasElement)
  const pane = paneFor(canvasElement), root = canvasElement.querySelector<HTMLElement>('[data-testid="resource-gates"]')!, canvas = within(canvasElement)
  const writes = () => Number(root.dataset.intentWrites), before = writes(), overview = within(pane).getByRole('button', { name: 'Overview', exact: true })
  await userEvent.click(overview); requireContract(overview.getAttribute('aria-expanded') === 'false' && writes() === before, 'Local collapse writes synced visibility intent')
  await userEvent.click(overview); requireContract(overview.getAttribute('aria-expanded') === 'true' && writes() === before, 'Local expansion writes synced visibility intent')
  const workspace = within(pane).getByRole('button', { name: /^Workspace/ }); await userEvent.click(workspace); requireContract(workspace.getAttribute('aria-expanded') === 'false' && writes() === before, 'Resource collapse writes synced visibility intent'); await userEvent.click(workspace)
  await userEvent.click(canvas.getByRole('button', { name: 'Visible sections', exact: true })); requireContract(writes() === before, 'Visibility disclosure writes synced intent')
  const context = within(canvasElement.ownerDocument.body).getByRole('checkbox', { name: 'Context', exact: true }) as HTMLInputElement
  await userEvent.click(context.closest('label')!); await waitFor(() => requireContract(!context.checked && writes() === before + 1 && !pane.querySelector('section[aria-label="Context"]'), 'Explicit hide does not commit exactly one visibility intent'))
  context.focus(); await userEvent.keyboard(' '); await waitFor(() => requireContract(context.checked && writes() === before + 2 && pane.querySelector('section[aria-label="Context"]'), 'Keyboard restore does not commit exactly one visibility intent'))
  await userEvent.keyboard('{Escape}')
  const events = () => JSON.parse(root.dataset.openEvents!) as { key: string; split: boolean }[]
  const terminal = within(pane).getByRole('button', { name: 'Open Session terminal', exact: true })
  await userEvent.click(terminal); await waitFor(() => requireContract(events().at(-1)?.key === 'terminal:sample/1' && !events().at(-1)?.split, 'Details terminal callback loses exact session identity'))
  await userEvent.click(terminal); requireContract(events().length === 2 && writes() === before + 2, 'Repeated terminal request alters layout intent or fails to reattach')
  const file = within(pane).getAllByRole('button', { name: /^Open src\// })[0]!
  await userEvent.click(file); requireContract(events().at(-1)?.key.startsWith('diff:sample/') && !events().at(-1)?.split, 'Details resource open callback loses pane identity')
  await userEvent.keyboard('{Control>}'); try { await userEvent.click(file) } finally { await userEvent.keyboard('{/Control}') }
  requireContract(events().at(-1)?.split, 'Details modified resource open loses split intent')
  const split = within(pane).getAllByRole('button', { name: /in split$/ })[0]!
  await userEvent.click(split); requireContract(events().at(-1)?.split && writes() === before + 2, 'Explicit split action alters synced visibility or loses split intent')
  await userEvent.click(file)
  await rejectsControl('Details modified resource open loses split intent', () => requireContract(events().at(-1)?.split, 'Details modified resource open loses split intent'))
  await humanControl(pane)
} }
export const PresentationMatrix: Story = { args: { cases: presentationCases }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, (pane, item) => assertCraft(pane, item)); await humanControl(paneFor(canvasElement))
} }
export const TraceContracts: Story = { args: { cases: [...sourceCases, example('turn-only context', 400, 'busy', {}, { ...full, series: missing }), example('null-turn context', 400, 'busy', {}, full.turns._tag === 'Known' ? { ...full, turns: { ...full.turns, value: { ...full.turns.value, points: full.turns.value.points.map((point, index) => index === 0 ? { ...point, contextTokens: null } : point) } } } : full)] }, play: async ({ canvasElement, args }) => {
  assertFixtureFacts()
  await eachCase(canvasElement, args.cases, (pane, item) => {
    const trace = item.trace!, projected = adaptTrace(trace), meter = trace.meters.spend
    requireContract(projected.lane.totalUsd === meter.mainUsd && projected.descendantSpend.totalUsd === meter.subagentUsd, 'Projection loses tagged source totals')
    assertHuman(pane); assertContext(pane, trace)
    if (!item.name.includes('/')) return
    const spend = pane.querySelector('section[aria-label="Spend"]')!, text = spend.textContent ?? '', amounts = text.match(/\$\d[\d,.]*/g) ?? []
    if (item.name.includes('/unknown/')) requireContract(amounts.length === 0 && /Unreported.*omitted/i.test(text), 'Unknown spend source renders inferred currency')
    else if (item.name.includes('/zero/')) requireContract(amounts.length > 0 && amounts.every(amount => amount === '$0.00'), 'Known zero spend is absent or fabricated')
    else requireContract(meter.mainUsd._tag === 'Known' && text.includes(`$${meter.mainUsd.value.toFixed(2)}`), 'Known source spend total is omitted')
  })
  const chart = paneFor(canvasElement).querySelector<HTMLElement>('[data-chart-samples]')!, circle = chart.querySelector('circle')!
  circle.remove(); await rejectsControl('Context plot does not match retained source samples', () => assertContext(paneFor(canvasElement), args.cases.at(-1)!.trace!)); chart.querySelector('svg')!.append(circle)
} }
export const BucketFallback: Story = { args: { cases: widths.map(width => example(`bucket/${width}`, width, 'busy', {}, detailsTraceFixtures.bucketOnly)) }, play: async ({ canvasElement, args }) => {
  requireContract(detailsTraceFixtures.bucketOnly.turns._tag === 'Unknown' && detailsTraceFixtures.bucketOnly.turns.reason === 'not-covered' && JSON.stringify(detailsTraceFixtures.bucketOnly.series) === JSON.stringify(full.series) && adaptTrace(detailsTraceFixtures.bucketOnly).lane.turnSamples === undefined, 'Bucket-only provider fabricates turns or changes source buckets')
  await eachCase(canvasElement, args.cases, (pane, item) => assertContext(pane, item.trace!))
  const pane = paneFor(canvasElement), axis = pane.querySelector<HTMLElement>('[data-chart-axis-label]')!, text = axis.textContent
  try { axis.textContent = 'per turn'; await rejectsControl('Context axis renames buckets as turns', () => assertContext(pane, detailsTraceFixtures.bucketOnly)) } finally { axis.textContent = text }
  assertContext(pane, detailsTraceFixtures.bucketOnly)
} }
const scopeCases = widths.flatMap(width => [example(`including/${width}`, width, 'busy', {}, full), example(`self-all-time/${width}`, width, 'busy', {}, { ...full, meters: { ...full.meters, spend: { ...full.meters.spend, scope: { _tag: 'Self' }, tokensWindow: { ...zero, value: { _tag: 'AllTime' } } } } }), example(`unreported-window/${width}`, width, 'busy', {}, { ...full, meters: { ...full.meters, spend: { ...full.meters.spend, tokensWindow: missing } } })])
async function scopePlay(canvasElement: HTMLElement, cases: readonly Case[]) {
  await eachCase(canvasElement, cases, (pane, item) => { assertCraft(pane, item); assertScope(pane, item.trace!) })
  const pane = paneFor(canvasElement), label = pane.querySelector<HTMLElement>('[data-details-token-label]')!, text = label.textContent
  try { label.textContent = 'Cumulative tokens · invented attribution'; const reason = cases.at(-1)!.trace!.meters.spend.scope._tag === 'Unknown' ? 'Unreported total makes an unproven scope claim' : 'Reported token scope is absent from its inline label'; if (reason.startsWith('Unreported')) label.textContent = 'Cumulative tokens · this agent'; await rejectsControl(reason, () => assertScope(pane, cases.at(-1)!.trace!)) } finally { label.textContent = text }
  assertScope(pane, cases.at(-1)!.trace!)
}
export const ReportedSpendScope: Story = { args: { cases: scopeCases }, play: ({ canvasElement, args }) => scopePlay(canvasElement, args.cases) }
export const UnreportedSpendScope: Story = { args: { cases: widths.map(width => example(`unreported/${width}`, width, 'partial', {}, detailsTraceFixtures.partial)) }, play: ({ canvasElement, args }) => scopePlay(canvasElement, args.cases) }
export const ResourceFit: Story = { args: { cases: resourceCases(['R1', 'R2', 'R3'], ['busy', 'partial'], widths) }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, (pane, item) => { assertFit(pane, item); assertCraft(pane, item) })
  const pane = paneFor(canvasElement), slot = [...pane.querySelectorAll<HTMLElement>('[data-resource-meta]')].find(meta => meta.dataset.resourceMetaFit === 'whole')!, original = slot.dataset.resourceMetaFit
  requireContract(slot, 'Fit negative control has no whole metadata')
  try { slot.dataset.resourceMetaFit = 'absent'; await rejectsControl('Metadata is neither whole nor absent', () => assertFit(pane, args.cases.at(-1)!)) } finally { slot.dataset.resourceMetaFit = original }
  assertFit(pane, args.cases.at(-1)!)
} }
export const ContainmentRegression: Story = { args: { cases: resourceCases(['R1', 'R2', 'R3'], ['busy'], widths) }, play: ({ canvasElement, args }) => eachCase(canvasElement, args.cases, containmentGate) }
export const ChevronColumns: Story = { args: { cases: widths.map(width => example(`chevrons/${width}`, width)) }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, assertChevron)
  const pane = paneFor(canvasElement), svg = pane.querySelector<SVGElement>('section[aria-label="Workspace"] button[aria-expanded] svg')!, original = svg.style.transform
  try { svg.style.transform = 'translateX(2px)'; await rejectsControl('Section chevrons do not share the Overview column', () => assertChevron(pane)) } finally { svg.style.transform = original }
  assertChevron(pane)
} }
export const OrientationAnchor: Story = { args: { cases: widths.map(width => example(`orientation/${width}`, width, 'idle')) }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, async pane => {
    const top = () => { pane.scrollTop = 0; return pane.querySelector('section')!.getBoundingClientRect().top - pane.getBoundingClientRect().top }
    const assertAnchor = () => requireContract(Math.abs(top()) <= 0.1, 'Orientation displaces the first Details section')
    for (const state of ['idle', 'new', 'absent']) { await userEvent.click(within(canvasElement).getByRole('button', { name: `Set ${state}`, exact: true })); await frames(); assertAnchor(); if (state === 'new') requireContract(pane.querySelector('section[aria-label="Overview"] [data-details-orientation]'), 'New orientation is outside Overview') }
    const section = pane.querySelector<HTMLElement>('section')!, original = section.style.marginTop
    try { section.style.marginTop = '4px'; await rejectsControl('Orientation displaces the first Details section', assertAnchor) } finally { section.style.marginTop = original }
    assertAnchor()
  })
} }
export const MissionTitleFit: Story = { args: { cases: [example('missions/320/R1', 320, 'busy', { resources: 'R1' })] }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, assertMissions)
  const pane = paneFor(canvasElement), title = [...pane.querySelectorAll<HTMLElement>('[data-details-resource-title]')].find(title => title.textContent === 'Review the source projection')!, original = title.style.textOverflow
  try { title.style.textOverflow = 'ellipsis'; await rejectsControl('Mission title is truncated', () => assertMissions(pane)) } finally { title.style.textOverflow = original }
  assertMissions(pane)
} }
export const BranchOrphan: Story = { args: { cases: widths.map(width => example(`branch/${width}`, width, 'partial')) }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, assertBranch)
  const pane = paneFor(canvasElement), title = pane.querySelector<HTMLElement>('[data-resource-title-lines="wrap"]')!, row = title.closest('[data-details-resource-row]')!, original = row.getAttribute('aria-label')!, originalText = title.textContent, originalWhiteSpace = title.style.whiteSpace
  try {
    title.textContent = 'declared branch\norphan'; title.style.whiteSpace = 'pre'; row.setAttribute('aria-label', `Open ${title.textContent}`)
    await rejectsControl('Wrapped identity leaves an orphaned final word', () => assertBranch(pane))
  } finally { title.textContent = originalText; title.style.whiteSpace = originalWhiteSpace; row.setAttribute('aria-label', original) }
  assertBranch(pane)
} }
export const IdentityClamp: Story = { args: { cases: resourceCases(['R1', 'R2'], ['busy', 'partial'], [240, 320, 400, 520]) }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, assertIdentity)
  const pane = paneFor(canvasElement), title = pane.querySelector<HTMLElement>('[data-resource-title-lines="wrap"]')!, original = title.style.webkitLineClamp
  try { title.style.webkitLineClamp = '1'; await rejectsControl('Resource identity is clamped instead of wrapping in full', () => assertIdentity(pane)) } finally { title.style.webkitLineClamp = original }
  assertIdentity(pane)
} }
export const AllStates: Story = { args: { cases: allStatesCases, comparison: true }, play: async ({ canvasElement, args }) => {
  await ready(canvasElement)
  const panes = [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="details-pane"]')]
  requireContract(panes.length === args.cases.length, 'AllStates omits a production Details state')
  for (const [index, pane] of panes.entries()) { assertCraft(pane, args.cases[index]!); if (args.cases[index]!.state === 'busy') await containmentGate(pane) }
  await humanControl(panes[0]!)
} }

const e3Traces: readonly { name: string; trace: TraceSeries }[] = [
  { name: 'full', trace: full }, { name: 'partial', trace: detailsTraceFixtures.partial }, { name: 'unknown', trace: detailsTraceFixtures.unknown },
  { name: 'bucket-only', trace: detailsTraceFixtures.bucketOnly },
  { name: 'unattributed', trace: { ...full, subagents: missing, meters: { ...full.meters, spend: { ...full.meters.spend, scope: { _tag: 'Unknown', reason: 'not-attributed' }, mainUsd: missing, subagentUsd: missing } } } },
]
const e3Cases = e3Traces.flatMap(({ name, trace }) => (['C1', 'C2', 'C3'] as const).flatMap((context, index) => (['S1', 'S2', 'S3'] as const).map(spend => example(`${name}/${context}/${spend}`, widths[index]!, 'busy', { context, spend }, trace))))
export const E3ChartMatrix: Story = { args: { cases: e3Cases }, play: async ({ canvasElement, args }) => {
  await eachCase(canvasElement, args.cases, (pane, item) => {
    assertHuman(pane)
    assertContext(pane, item.trace!)
    for (const name of ['Context', 'Spend']) {
      const section = pane.querySelector<HTMLElement>(`section[aria-label="${name}"]`)!
      requireContract(section, 'E3 omits its chart section')
      const body = section.querySelector(`[data-details-chart="${name === 'Context' ? item.variants.context : item.variants.spend}"]`)!
      const data = hasDetailsChartData({ trace: item.trace!, context: item.variants.context, spend: item.variants.spend, section: name === 'Context' ? 'context' : 'spend', empty: 'E3' })
      if (!data) requireContract(body.querySelector('[data-details-reason]'), 'E3 hides unavailable field explanations')
      else requireContract(body.querySelector('[data-details-data-row],dl,svg[role="img"]'), 'E3 drops known chart rows')
    }
    if (item.trace!.meters.spend.tokens._tag === 'Known') assertScope(pane, item.trace!)
  })
  await humanControl(paneFor(canvasElement))
} }
