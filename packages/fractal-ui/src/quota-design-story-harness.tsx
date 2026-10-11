import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { QuotaDrawer } from './QuotaDrawer.tsx'
import { QuotaUsagePage } from './QuotaUsagePage.tsx'
import type { QuotaDrawerProps, QuotaSort, QuotaStatus, QuotaUnit, QuotaUsageProps } from './quota-design-types.ts'
import { quotaAccounts, quotaClock, quotaColumns, quotaDays, quotaEconomics, quotaRefreshedAccounts, quotaStatuses } from './quota-design-fixtures.ts'
import { assertQuotaDrawerDensity } from './quota-design-density-proof.ts'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { accentVars as accent, borderVars as border, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex'

export interface QuotaStoryArgs {
  scheme: Scheme
  width: 240 | 320 | 420
  pageWidth: 900 | 1440
  drawerVariant: QuotaDrawerProps['variant']
  usageVariant: QuotaUsageProps['variant']
  status: QuotaStatus['kind']
  empty: boolean
  initialView: 'drawer' | 'usage'
  landmarkContext?: string
}
export const quotaStoryDefaults: QuotaStoryArgs = { scheme: 'dark', width: 320, pageWidth: 1440, drawerVariant: 'columns', usageVariant: 'overview', status: 'current', empty: false, initialView: 'drawer' }
export const quotaStoryControls = {
  scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [240, 320, 420], control: 'radio' },
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
  return <div {...stylex.props(styles.frame, ...baselineTheme, args.scheme === 'light' && lightTheme, !usage && (args.width === 240 ? styles.small : args.width === 420 ? styles.large : styles.medium), usage && (args.pageWidth === 900 ? styles.pageNarrow : styles.pageWide))}
    data-quota-story data-sort-key={state.sort.key} data-sort-direction={state.sort.direction} data-unit={state.unit} data-range={`${state.range[0]}:${state.range[1]}`} data-selected-day={state.selectedDay ?? ''} data-selected-account={state.selectedAccount ?? ''} data-view={state.view} data-collapsed={String(state.collapsed)} data-escaped={state.escaped} data-refreshed={String(state.refreshed)}>
    <ThemePortal>
      <div {...stylex.props(styles.proofToolbar)}>
        <Button {...stylex.props(styles.button)} onPress={() => update({ refreshed: !state.refreshed, statusOverride: 'current' })}>Refresh snapshot</Button>
        <Button {...stylex.props(styles.button)} onPress={() => update({ statusOverride: 'failed' })}>Simulate failed refresh</Button>
      </div>
      {usage ? <QuotaUsagePage landmarkContext={args.landmarkContext} accounts={accounts} days={args.empty ? [] : quotaDays} economics={args.empty ? [] : quotaEconomics} status={status} variant={args.usageVariant} unit={state.unit} stackBy={state.stackBy} range={state.range}
        selectedDay={state.selectedDay} selectedAccount={state.selectedAccount} detailTab={state.tab}
        onUnit={unit => update({ unit })} onStackBy={stackBy => update({ stackBy })} onRange={range => update({ range, selectedDay: undefined })}
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
export const quotaDensityPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
 for (const drawer of canvasElement.querySelectorAll<HTMLElement>('[data-quota-drawer]')) {
  if (drawer.querySelector('[data-quota-row]') !== null) await waitFor(() => assertQuotaDrawerDensity(drawer))
 }
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
export const quotaBrushPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  const start = canvas.getByRole('slider', { name: /Range start/ })
  start.focus()
  await userEvent.keyboard('{ArrowRight}{ArrowRight}{ArrowRight}')
  await waitFor(() => expect(storyRoot(canvasElement).dataset.range).toBe('3:29'))
  const day = canvasElement.querySelector<HTMLElement>('[data-quota-day="2030-04-15"]')!
  await userEvent.click(day)
  await waitFor(() => expect(storyRoot(canvasElement).dataset.selectedDay).toBe('2030-04-15'))
  await expect(canvasElement.querySelector('[data-quota-breakdown]')).toHaveTextContent(/15 Apr|2030-04-15/)
}
const geometry = (canvas: HTMLElement) => [...canvas.querySelectorAll<HTMLElement>('[data-quota-row], [data-quota-status], [data-quota-breakdown], [data-quota-usage] svg')].map(node => {
  const box = node.getBoundingClientRect(); return [box.x, box.y, box.width, box.height]
})
export const quotaRefreshPlay = async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await quotaDensityPlay({ canvasElement })
  const canvas = within(canvasElement), before = geometry(canvasElement)
  await expect(before.length).toBeGreaterThan(0)
  await userEvent.click(canvas.getByRole('button', { name: 'Refresh snapshot' }))
  await waitFor(() => expect(storyRoot(canvasElement).dataset.refreshed).toBe('true'))
  await expect(geometry(canvasElement)).toEqual(before)
  await userEvent.click(canvas.getByRole('button', { name: 'Simulate failed refresh' }))
  await waitFor(() => expect(canvasElement.querySelector('[data-quota-status]')).toHaveTextContent(/failed/i))
  await expect(geometry(canvasElement)).toEqual(before)
}
const styles = stylex.create({
  frame: { display: 'flex', flexDirection: 'column', boxSizing: 'border-box', backgroundColor: surface.canvas, color: ink.fg, fontFamily: type.fontSans, fontSize: type.metaSize, maxWidth: '100%', minWidth: 0 },
  small: { width: 240, height: 640 }, medium: { width: 320, height: 640 }, large: { width: 420, height: 640 },
  pageNarrow: { width: 900 }, pageWide: { width: 1440 },
  matrix: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: 20, padding: 16, backgroundColor: surface.canvas, boxSizing: 'border-box', maxWidth: '100%' },
  matrixItem: { minWidth: 0, maxWidth: '100%' },
  caption: { fontFamily: type.fontSans, fontSize: 12, fontWeight: 500, margin: '0 0 8px', color: ink.fgMuted },
  proofToolbar: { display: 'flex', flexShrink: 0, flexWrap: 'wrap', gap: 4, minHeight: 36, padding: 4, borderBottom: `1px solid ${border.border}` },
  button: { fontFamily: type.fontSans, fontSize: 10, padding: '4px 6px', backgroundColor: surface.controlFill, color: ink.fgMuted, border: `1px solid ${border.borderStrong}`, borderRadius: 4, cursor: 'pointer', ':focus-visible': { outline: `2px solid ${accent.primary}`, outlineOffset: 2 } },
})
