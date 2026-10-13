import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { quotaGeometry } from './quota-geometry.stylex.ts'
import { usageMeterGridVars } from './UsageMeterGrid.stylex.ts'
import { Button } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { QuotaDrawer } from './QuotaDrawer.tsx'
import { QuotaUsagePage } from './QuotaUsagePage.tsx'
import type { QuotaDrawerProps, QuotaSort, QuotaStatus, QuotaUnit, QuotaUsageProps } from './quota-design-types.ts'
import { quotaAccounts, quotaClock, quotaColumns, quotaCoverageDays, quotaDays, quotaEconomics, quotaRefreshedAccounts, quotaStatuses } from './quota-design-fixtures.ts'
import { assertQuotaDrawerDensity } from './quota-design-density-proof.ts'
import { expectNoNativeTitles, expectSingleFullValueTooltip } from './full-value-tooltip-play.ts'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { accentVars as accent, borderVars as border, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex'

export interface QuotaStoryArgs {
  scheme: Scheme
  width: 200 | 240 | 320 | 420
  pageWidth: 900 | 1440
  drawerVariant: QuotaDrawerProps['variant']
  usageVariant: QuotaUsageProps['variant']
  status: QuotaStatus['kind']
  empty: boolean
  initialView: 'drawer' | 'usage'
  landmarkContext?: string
  coverageScenario?: boolean
}
export const quotaStoryDefaults: QuotaStoryArgs = { scheme: 'dark', width: 320, pageWidth: 1440, drawerVariant: 'columns', usageVariant: 'overview', status: 'current', empty: false, initialView: 'drawer' }
export const quotaStoryControls = {
  scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [200, 240, 320, 420], control: 'radio' },
  pageWidth: { options: [900, 1440], control: 'radio' }, drawerVariant: { options: ['columns', 'stacked', 'chips'], control: 'radio' },
  usageVariant: { options: ['overview', 'multiples', 'heatmap'], control: 'radio' }, status: { options: Object.keys(quotaStatuses), control: 'select' }, empty: { control: 'boolean' }, initialView: { control: false },
} as const
interface State {
  sort: QuotaSort; unit: QuotaUnit; provider: string; plan: string; state: string; grouped: boolean; collapsed: boolean
  view: 'drawer' | 'usage'; range: readonly [number, number]; selectedDay?: string; selectedAccount?: string
  tab: QuotaUsageProps['detailTab']; stackBy: QuotaUsageProps['stackBy']; refreshed: boolean; statusOverride?: QuotaStatus['kind']; escaped: number
}
const initial = (view: State['view']): State => ({ sort: { key: 'governing', direction: 'descending' }, unit: 'usd', provider: '', plan: '', state: '', grouped: false, collapsed: false, view, range: [0, 29], tab: 'limits', stackBy: 'account', refreshed: false, escaped: 0 })
export function QuotaStoryHarness(args: QuotaStoryArgs) {
  const [state, update] = React.useReducer((state: State, patch: Partial<State>): State => ({ ...state, ...patch }), args.initialView, initial)
  const accounts = args.empty ? [] : state.refreshed ? quotaRefreshedAccounts : quotaAccounts
  const sourceStatus = quotaStatuses[state.statusOverride ?? args.status]
  const status: QuotaStatus = args.empty && sourceStatus.kind === 'loading' ? { kind: 'loading', text: 'Loading usage · no previous snapshot' }
    : args.empty && sourceStatus.kind === 'current' ? { kind: 'current', text: 'No accounts in this reported snapshot' }
      : args.empty && sourceStatus.kind === 'failed' ? { kind: 'failed', text: 'Collection failed · no previous snapshot available' }
        : sourceStatus
  const usage = state.view === 'usage'
  return <div {...stylex.props(styles.frame, ...baselineTheme, args.scheme === 'light' && lightTheme, !usage && (args.width === 200 ? styles.pressure : args.width === 240 ? styles.small : args.width === 420 ? styles.large : styles.medium), usage && (args.pageWidth === 900 ? styles.pageNarrow : styles.pageWide))}
    data-quota-story data-quota-account-count={accounts.length} data-sort-key={state.sort.key} data-sort-direction={state.sort.direction} data-unit={state.unit} data-range={`${state.range[0]}:${state.range[1]}`} data-selected-day={state.selectedDay ?? ''} data-selected-account={state.selectedAccount ?? ''} data-view={state.view} data-collapsed={String(state.collapsed)} data-escaped={state.escaped} data-refreshed={String(state.refreshed)} data-provider={state.provider} data-plan={state.plan} data-state={state.state}>
    <ThemePortal>
      <div {...stylex.props(styles.proofToolbar)}>
        <Button {...stylex.props(styles.button)} onPress={() => update({ refreshed: !state.refreshed, statusOverride: state.refreshed ? undefined : 'current' })}>Refresh snapshot</Button>
        <Button {...stylex.props(styles.button)} onPress={() => update({ statusOverride: 'failed' })}>Simulate failed refresh</Button>
      </div>
      {usage ? <QuotaUsagePage landmarkContext={args.landmarkContext} accounts={accounts} days={args.empty ? [] : args.coverageScenario ? quotaCoverageDays : quotaDays} economics={args.empty ? [] : quotaEconomics} status={status} variant={args.usageVariant} unit={state.unit} stackBy={state.stackBy} range={state.range}
        selectedDay={state.selectedDay} selectedAccount={state.selectedAccount} detailTab={state.tab}
        onUnit={unit => update({ unit })} onStackBy={stackBy => update({ stackBy })} onRange={range => update({ range, ...(args.coverageScenario ? {} : { selectedDay: undefined }) })}
        onDay={selectedDay => update({ selectedDay })} onAccount={selectedAccount => update({ selectedAccount: selectedAccount || undefined })} onDetailTab={tab => update({ tab })} onBack={() => update({ view: 'drawer' })} />
      : <QuotaDrawer landmarkContext={args.landmarkContext} accounts={accounts} columns={quotaColumns} status={status} variant={args.drawerVariant} sort={state.sort} unit={state.unit}
        provider={state.provider} plan={state.plan} state={state.state} grouped={state.grouped} collapsed={state.collapsed} now={quotaClock}
        onSort={sort => update({ sort })} onUnit={unit => update({ unit })} onFilter={filter => update(filter)} onGrouped={grouped => update({ grouped })}
        onCollapsed={collapsed => update({ collapsed })} onOpenUsage={selectedAccount => update({ selectedAccount, view: 'usage' })} onEscape={() => update({ escaped: state.escaped + 1 })} />}
    </ThemePortal>
  </div>
}
export const QuotaDrawerMatrix = (args: QuotaStoryArgs) => <div {...stylex.props(styles.matrix)}>{(['dark', 'light'] as const).flatMap(scheme => (['columns', 'stacked', 'chips'] as const).map(drawerVariant => <section key={`${scheme}-${drawerVariant}`} {...stylex.props(styles.matrixItem)}><h2 {...stylex.props(styles.caption)}>{scheme} · {drawerVariant}</h2><QuotaStoryHarness {...args} scheme={scheme} drawerVariant={drawerVariant} landmarkContext={`${scheme} ${drawerVariant}`} /></section>))}</div>
export const QuotaUsageMatrix = (args: QuotaStoryArgs) => <div {...stylex.props(styles.matrix)}>{(['dark', 'light'] as const).flatMap(scheme => (['overview', 'multiples', 'heatmap'] as const).map(usageVariant => <section key={`${scheme}-${usageVariant}`} {...stylex.props(styles.matrixItem)}><h2 {...stylex.props(styles.caption)}>{scheme} · {usageVariant}</h2><QuotaStoryHarness {...args} scheme={scheme} usageVariant={usageVariant} initialView="usage" landmarkContext={`${scheme} ${usageVariant}`} /></section>))}</div>
export const QuotaStatesMatrix = (args: QuotaStoryArgs) => <div {...stylex.props(styles.matrix)}>{(['dark', 'light'] as const).flatMap(scheme => (Object.keys(quotaStatuses) as QuotaStatus['kind'][]).map(status => <section key={`${scheme}-${status}`} {...stylex.props(styles.matrixItem)}><h2 {...stylex.props(styles.caption)}>{scheme} · {status} · retained inventory</h2><QuotaStoryHarness {...args} scheme={scheme} status={status} landmarkContext={`${scheme} ${status}`} /></section>))}</div>
const storyRoot = (canvas: HTMLElement) => canvas.querySelector<HTMLElement>('[data-quota-story]')!
const ids = (canvas: HTMLElement) => [...canvas.querySelectorAll('[data-quota-row]')].map(row => row.getAttribute('data-quota-row'))
const defaultOrder = ['harbor', 'north', 'birch', 'delta', 'meadow', 'lake', 'elm', 'spruce', 'cove', 'willow', 'unknown']
/** Exercise the real controlled refresh callback, including a disabled-action control; after the refresh no native title doubles a custom hover anywhere in the story. */
export const quotaControlPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  for (const root of canvasElement.querySelectorAll<HTMLElement>('[data-quota-story]')) {
    const refresh = within(root).getByRole<HTMLButtonElement>('button', { name: 'Refresh snapshot' })
    const before = root.dataset.refreshed
    const after = before === 'true' ? 'false' : 'true'
    refresh.disabled = true
    try {
      await userEvent.click(refresh)
      await expect(waitFor(() => expect(root.dataset.refreshed).toBe(after), { timeout: 100 })).rejects.toThrow()
    } finally {
      refresh.disabled = false
    }
    await userEvent.click(refresh)
    await waitFor(() => expect(root.dataset.refreshed).toBe(after))
    await userEvent.click(refresh)
    await waitFor(() => expect(root.dataset.refreshed).toBe(before))
    await expectNoNativeTitles(root)
    root.dataset.quotaControlPassed = 'true'
  }
}
export const quotaDensityPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
 await quotaControlPlay({ canvasElement })
 for (const drawer of canvasElement.querySelectorAll<HTMLElement>('[data-quota-drawer]')) {
  // React Aria's collection can populate after the first static render. Wait
  // for supplied inventory, not for the presence of already-rendered rows.
  if (Number(drawer.closest<HTMLElement>('[data-quota-story]')?.dataset.quotaAccountCount) > 0) await waitFor(() => assertQuotaDrawerDensity(drawer))
 }
}

/** Keep the forced row height while breaking its real content geometry. */
export const quotaLinePlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaDensityPlay({ canvasElement })
  const drawer = canvasElement.querySelector<HTMLElement>('[data-quota-drawer]')!
  const root = storyRoot(canvasElement)
  const canvas = within(canvasElement)
  const glyph = drawer.querySelector<SVGSVGElement>('[data-quota-row="harbor"] [data-quota-cell="primary"] svg')!
  const originalTransform = glyph.style.transform
  try {
    glyph.style.transform = `translateY(${usageMeterGridVars.rowHeight})`
    await expect(() => assertQuotaDrawerDensity(drawer)).toThrow('misaligned glyph')
  } finally {
    glyph.style.transform = originalTransform
  }
  await userEvent.click(canvas.getByRole('button', { name: 'Refresh snapshot' }))
  await waitFor(() => expect(root.dataset.refreshed).toBe('true'))
  await waitFor(() => assertQuotaDrawerDensity(drawer))
  const label = drawer.querySelector<HTMLElement>('[data-quota-row="north"] [data-quota-account-label]')!
  const originalStyle = label.getAttribute('style')
  try {
    label.style.display = 'inline-block'
    label.style.width = '1ch'
    label.style.whiteSpace = 'normal'
    label.style.overflowWrap = 'anywhere'
    await expect(() => assertQuotaDrawerDensity(drawer)).toThrow('wraps')
  } finally {
    if (originalStyle === null) label.removeAttribute('style')
    else label.setAttribute('style', originalStyle)
  }
  await userEvent.click(canvas.getByRole('button', { name: 'Refresh snapshot' }))
  await waitFor(() => expect(root.dataset.refreshed).toBe('false'))
  await waitFor(() => assertQuotaDrawerDensity(drawer))
}
/** Stories rest unfocused, so the last pressed control does not keep a focus ring in the at-rest frame. */
const restFocus = (canvasElement: HTMLElement) => {
  const active = canvasElement.ownerDocument.activeElement
  if (active instanceof HTMLElement && canvasElement.contains(active)) active.blur()
}
/** Content-sized sort controls keep 8px between neighbours inside the toolbar; glyphs are SVG, never text arrows. No native title doubles a custom hover, and the shortcut footer stays one line. */
export const quotaPriorityPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaLinePlay({ canvasElement })
  const drawer = canvasElement.querySelector<HTMLElement>('[data-quota-drawer]')!
  await expectNoNativeTitles(drawer)
  const toolbar = within(drawer).getByRole('toolbar', { name: 'Sort quota' })
  const buttons = [...toolbar.querySelectorAll<HTMLElement>('[data-quota-sort]')]
  await expect(buttons.map(button => button.textContent)).toEqual(['Gov', 'Cmp', 'Acct', 'Reset', '+N'])
  await expect(toolbar.querySelector('[aria-pressed="true"] [data-quota-sort-indicator] svg')).not.toBeNull()
  const boxes = buttons.map(button => button.getBoundingClientRect())
  for (const [index, button] of buttons.entries()) {
    await expect(button.scrollWidth).toBeLessThanOrEqual(button.clientWidth)
    if (index > 0) await expect(boxes[index]!.left - boxes[index - 1]!.right).toBeGreaterThanOrEqual(7.99)
  }
  const bounds = toolbar.getBoundingClientRect()
  await expect(boxes[0]!.left).toBeGreaterThanOrEqual(bounds.left)
  await expect(boxes.at(-1)!.right).toBeLessThanOrEqual(bounds.right)
  const footer = drawer.querySelector<HTMLElement>('[data-quota-shortcuts]')!
  const footerStyle = getComputedStyle(footer)
  await expect(footer.scrollWidth).toBeLessThanOrEqual(footer.clientWidth)
  await expect(footer.getBoundingClientRect().height).toBeLessThanOrEqual(Number.parseFloat(footerStyle.lineHeight) + Number.parseFloat(footerStyle.paddingTop) + Number.parseFloat(footerStyle.paddingBottom) + Number.parseFloat(footerStyle.borderTopWidth) + 0.5)
}
/** A clipped account name, an abbreviated reset and every abbreviated column header each open exactly one tooltip with the full value on hover and on keyboard focus. */
export const quotaFullValuePlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaPriorityPlay({ canvasElement })
  const row = canvasElement.querySelector<HTMLElement>('[data-quota-row="unknown"]')!
  const account = row.querySelector<HTMLElement>('[data-quota-cell="account"] button')!
  await expect(account.scrollWidth).toBeGreaterThan(account.clientWidth)
  await expectSingleFullValueTooltip(account, account.getAttribute('aria-label')!.replace(/^Open usage for /, ''))
  const reset = row.querySelector<HTMLElement>('[data-quota-cell="reset"] [data-full-value-trigger]')!
  await expectSingleFullValueTooltip(reset, reset.querySelector('[data-quota-full-value]')!.textContent!)
  // Usage, $ / tok and +N abbreviate their column names; columns the container hides have no trigger to hover.
  const headers = [...canvasElement.querySelectorAll<HTMLElement>('[data-quota-column] [data-full-value-trigger]')].filter(trigger => trigger.getClientRects().length > 0)
  await expect(headers.map(trigger => trigger.closest<HTMLElement>('[data-quota-column]')!.dataset.quotaColumn)).toEqual(expect.arrayContaining(['primary', 'more']))
  for (const trigger of headers) await expectSingleFullValueTooltip(trigger, trigger.querySelector('[data-quota-full-value]')!.textContent!)
  await expectNoNativeTitles(canvasElement.querySelector<HTMLElement>('[data-quota-drawer]')!)
  restFocus(canvasElement)
}
export const quotaSortPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaDensityPlay({ canvasElement })
  await waitFor(() => expect(ids(canvasElement)).toEqual(defaultOrder))
  const account = canvasElement.querySelector<HTMLElement>('[data-quota-sort="account"]')!
  await userEvent.click(account)
  await waitFor(() => expect(ids(canvasElement)).toEqual(['birch', 'cove', 'delta', 'elm', 'harbor', 'lake', 'meadow', 'north', 'spruce', 'unknown', 'willow']))
  await userEvent.click(account)
  await waitFor(() => expect(ids(canvasElement)).toEqual(['willow', 'unknown', 'spruce', 'north', 'meadow', 'lake', 'harbor', 'elm', 'delta', 'cove', 'birch']))
  await userEvent.click(canvasElement.querySelector<HTMLElement>('[data-quota-sort="governing"]')!)
  await waitFor(() => expect(ids(canvasElement)).toEqual(defaultOrder))
  restFocus(canvasElement)
}
export const quotaKeyboardPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaDensityPlay({ canvasElement })
  const drawer = canvasElement.querySelector<HTMLElement>('[data-quota-drawer]')!
  drawer.focus()
  await userEvent.keyboard('s')
  await waitFor(() => expect(storyRoot(canvasElement).dataset.sortKey).not.toBe('governing'))
  const previous = storyRoot(canvasElement).dataset.sortDirection
  await userEvent.keyboard('S')
  await waitFor(() => expect(storyRoot(canvasElement).dataset.sortDirection).not.toBe(previous))
  await userEvent.keyboard('u')
  await waitFor(() => expect(storyRoot(canvasElement).dataset.unit).toBe('tokens'))
  await userEvent.keyboard('m')
  await waitFor(() => expect(storyRoot(canvasElement).dataset.collapsed).toBe('true'))
  await userEvent.keyboard('m{Escape}')
  await waitFor(() => expect(storyRoot(canvasElement).dataset.escaped).toBe('1'))
}
/** Independent oracle for a day-chart button's accessible description: account stacking, the chart's series and effective unit. */
const expectedDayDescription = (dayId: string, unit: QuotaUnit, onlySeries?: string): string => {
  const exact = unit === 'usd' ? (value: number) => new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 10 }).format(value) : (value: number) => `${new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 }).format(value)} tok`
  const tally = (amounts: readonly (number | undefined)[]): string => {
    const known = amounts.filter((value): value is number => value !== undefined)
    const unknown = amounts.length - known.length
    const sum = known.reduce((total, value) => total + value, 0)
    return known.length === 0 ? unknown === 0 ? 'No rows' : 'Unreported' : unknown > 0 ? `${exact(sum)} + ${unknown} Unreported` : exact(sum)
  }
  const label = (id: string) => quotaAccounts.find(account => account.id === id)?.label ?? id
  const series = onlySeries === undefined
    ? [...new Set(quotaDays.flatMap(day => day.values.map(value => value.accountId)))].sort((left, right) => label(left).localeCompare(label(right)) || left.localeCompare(right))
    : [onlySeries]
  const day = quotaDays.find(candidate => candidate.id === dayId)!
  const amounts = (id: string) => day.values.filter(value => value.accountId === id).map(value => unit === 'usd' ? value.usd : value.tokens)
  const parts = series.flatMap(id => amounts(id).length === 0 ? [] : [`${label(id)}: ${tally(amounts(id))}`])
  return `${day.label}${day.partial ? ' (partial Berlin day, still open)' : ''}. Total ${tally(series.flatMap(amounts))}${parts.length > 0 ? `. ${parts.join('; ')}` : ''}`
}
export const quotaBrushPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaControlPlay({ canvasElement })
  const canvas = within(canvasElement)
  const start = canvas.getByRole('slider', { name: /Range start/ })
  start.focus()
  await userEvent.keyboard('{ArrowRight}{ArrowRight}{ArrowRight}')
  await waitFor(() => expect(storyRoot(canvasElement).dataset.range).toBe('3:29'))
  const dayId = '2030-04-15'
  // Day-chart buttons: the name is the day, full evidence is a mounted description, and no native title doubles the custom readout.
  const chartDays = [...canvasElement.querySelectorAll<HTMLElement>('[data-quota-day][aria-describedby]')]
  for (const button of chartDays) {
    await expect(button.querySelector('[title]')).toBeNull()
    await expect(canvasElement.ownerDocument.getElementById(button.getAttribute('aria-describedby')!)).not.toBeNull()
  }
  const chartDay = chartDays.find(button => button.dataset.quotaDay === dayId)
  if (chartDay !== undefined) {
    const unit = canvasElement.querySelector<HTMLElement>('[data-quota-usage]')!.dataset.quotaUnit as QuotaUnit
    await expect(chartDay).toHaveAccessibleName(quotaDays.find(candidate => candidate.id === dayId)!.label)
    await expect(chartDay).toHaveAccessibleDescription(expectedDayDescription(dayId, unit, chartDay.closest<HTMLElement>('[data-quota-series]')?.dataset.quotaSeries))
    const readoutDay = () => canvasElement.querySelector<HTMLElement>('[data-quota-readout]')?.dataset.quotaReadout
    await userEvent.hover(chartDay)
    await waitFor(() => expect(readoutDay()).toBe(dayId))
    await userEvent.unhover(chartDay)
    chartDay.focus()
    await waitFor(() => expect(readoutDay()).toBe(dayId))
    await expect(chartDay).toHaveAttribute('aria-pressed', 'false')
  }
  const day = canvasElement.querySelector<HTMLElement>(`[data-quota-day="${dayId}"]`)!
  await userEvent.click(day)
  await waitFor(() => expect(storyRoot(canvasElement).dataset.selectedDay).toBe(dayId))
  await expect(day).toHaveAttribute('aria-pressed', 'true')
  await expect(canvasElement.querySelector('[data-quota-breakdown]')).toHaveTextContent(/15 Apr|2030-04-15/)
  const readout = canvasElement.querySelector<HTMLElement>('[data-quota-readout]')
  if (readout !== null) {
    await expect(readout).toHaveAttribute('tabindex', '0')
    readout.focus()
    await expect(readout).toHaveFocus()
  }
}
export const quotaCoveragePlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaControlPlay({ canvasElement })
  const canvas = within(canvasElement)
  const root = storyRoot(canvasElement)
  const usage = canvasElement.querySelector<HTMLElement>('[data-quota-usage]')!
  const lowDay = '2030-04-15'
  await userEvent.click(canvasElement.querySelector<HTMLElement>(`[data-quota-day="${lowDay}"]`)!)
  await waitFor(() => expect(root.dataset.selectedDay).toBe(lowDay))
  await expect(usage.dataset.quotaUnit).toBe('tokens')
  const breakdown = canvasElement.querySelector<HTMLElement>('[data-quota-breakdown]')!
  await expect(breakdown.dataset.quotaBreakdown).toBe(lowDay)
  const start = canvas.getByRole('slider', { name: /Range start/ })
  start.focus()
  await userEvent.keyboard('{End}')
  await waitFor(() => expect(root.dataset.range).toBe('29:29'))
  await expect(root.dataset.selectedDay).toBe(lowDay)
  await expect(usage.dataset.quotaUnit).toBe('usd')
  const assertCoverage = () => {
    const lowRecords = canvasElement.querySelector(`[data-quota-breakdown="${lowDay}"], [data-quota-readout="${lowDay}"]`)
    if (usage.dataset.quotaUnit === 'usd' && lowRecords !== null) throw new Error('Low-coverage day bypassed USD admission')
    expect(breakdown.dataset.quotaBreakdown).toBe('range')
  }
  assertCoverage()
  const admittedScope = breakdown.dataset.quotaBreakdown
  try {
    breakdown.dataset.quotaBreakdown = lowDay
    expect(assertCoverage).toThrow('Low-coverage day bypassed USD admission')
  } finally {
    breakdown.dataset.quotaBreakdown = admittedScope
  }
  await userEvent.click(canvas.getByRole('button', { name: 'Refresh snapshot' }))
  await waitFor(() => expect(root.dataset.refreshed).toBe('true'))
  assertCoverage()
  await userEvent.click(canvas.getByRole('button', { name: 'Refresh snapshot' }))
  await waitFor(() => expect(root.dataset.refreshed).toBe('false'))
  assertCoverage()
}

export const quotaFilterPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaControlPlay({ canvasElement })
  const canvas = within(canvasElement)
  const root = storyRoot(canvasElement)
  await userEvent.click(canvas.getByRole('button', { name: /Filter provider/ }))
  await userEvent.click(canvas.getByRole('option', { name: 'Cedar' }))
  await waitFor(() => expect(root.dataset.provider).toBe('Cedar'))
  await expect(root.dataset.plan).toBe('')
  await expect(root.dataset.state).toBe('')
  await userEvent.click(canvas.getByRole('button', { name: /Filter plan/ }))
  await userEvent.click(canvas.getByRole('option', { name: 'Team' }))
  await waitFor(() => expect(root.dataset.plan).toBe('Team'))
  await expect(root.dataset.provider).toBe('Cedar')
  await userEvent.click(canvas.getByRole('button', { name: /Filter state/ }))
  await userEvent.click(canvas.getByRole('option', { name: 'Stale' }))
  await waitFor(() => expect(root.dataset.state).toBe('stale'))
  await waitFor(() => expect(ids(canvasElement)).toEqual(['elm']))
  await userEvent.click(canvas.getByRole('button', { name: 'Clear quota filters' }))
  await waitFor(() => expect(ids(canvasElement)).toEqual(defaultOrder))
  restFocus(canvasElement)
}
const geometry = (canvas: HTMLElement) => [...canvas.querySelectorAll<HTMLElement>('[data-quota-row], [data-quota-status], [data-quota-breakdown], [data-quota-usage] svg')].map(node => {
  const box = node.getBoundingClientRect(); return [box.x, box.y, box.width, box.height]
})
export const quotaRefreshPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaDensityPlay({ canvasElement })
  const canvas = within(canvasElement), before = geometry(canvasElement)
  const status = () => canvasElement.querySelector<HTMLElement>('[data-quota-status]')
  const restingStatus = status()?.dataset.quotaStatus
  await expect(before.length).toBeGreaterThan(0)
  await userEvent.click(canvas.getByRole('button', { name: 'Refresh snapshot' }))
  await waitFor(() => expect(storyRoot(canvasElement).dataset.refreshed).toBe('true'))
  await expect(geometry(canvasElement)).toEqual(before)
  await userEvent.click(canvas.getByRole('button', { name: 'Simulate failed refresh' }))
  await waitFor(() => expect(status()).toHaveTextContent(/failed/i))
  await expect(geometry(canvasElement)).toEqual(before)
  // The story rests in the state its args name, not in the simulated failure.
  await userEvent.click(canvas.getByRole('button', { name: 'Refresh snapshot' }))
  await waitFor(() => expect(storyRoot(canvasElement).dataset.refreshed).toBe('false'))
  await waitFor(() => expect(status()?.dataset.quotaStatus).toBe(restingStatus))
  await expect(geometry(canvasElement)).toEqual(before)
  restFocus(canvasElement)
}
const styles = stylex.create({
  frame: { display: 'flex', flexDirection: 'column', boxSizing: 'border-box', backgroundColor: surface.canvas, color: ink.fg, fontFamily: type.fontSans, fontSize: type.metaSize, maxWidth: '100%', minWidth: 0 },
  pressure: { width: quotaGeometry.drawerPressureWidth, height: quotaGeometry.drawerFixtureHeight },
  small: { width: quotaGeometry.drawerSmallWidth, height: quotaGeometry.drawerFixtureHeight }, medium: { width: quotaGeometry.drawerMediumWidth, height: quotaGeometry.drawerFixtureHeight }, large: { width: quotaGeometry.drawerLargeWidth, height: quotaGeometry.drawerFixtureHeight },
  pageNarrow: { width: quotaGeometry.usageNarrowWidth }, pageWide: { width: quotaGeometry.usageWideWidth },
  matrix: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: space.xxl, padding: space.xl, backgroundColor: surface.canvas, boxSizing: 'border-box', maxWidth: '100%' },
  matrixItem: { minWidth: 0, maxWidth: '100%' },
  caption: { fontFamily: type.fontSans, fontSize: type.metaSize, fontWeight: type.weightMedium, margin: quotaGeometry.captionMargin, color: ink.fgMuted },
  proofToolbar: { display: 'flex', flexShrink: 0, flexWrap: 'wrap', gap: space.xs, minHeight: quotaGeometry.proofToolbarHeight, padding: space.xs, borderBottom: quotaGeometry.hair },
  button: { fontFamily: type.fontSans, fontSize: type.microSize, padding: quotaGeometry.proofButtonPadding, backgroundColor: surface.controlFill, color: ink.fgMuted, border: quotaGeometry.strongHair, borderRadius: radius.keyboard, cursor: 'pointer', ':focus-visible': { outline: quotaGeometry.focusOutline, outlineOffset: space.xs2 } },
})
