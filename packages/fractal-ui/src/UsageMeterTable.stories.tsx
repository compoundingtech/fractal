import type * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { UsageMeterHistoryChart, UsageMeterTable, type UsageMeterHistory, type UsageMeterHistoryPoint, type UsageMeterRow, type UsageMeterSortKey, type UsageMeterTableProps } from './UsageMeterTable'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const rows: readonly UsageMeterRow[] = [
  {
    id: 'meter-north', label: 'North workspace', identity: 'team plan · seat 1',
    governing: { label: '5h window', text: '72% · 3.6k / 5k', fraction: 0.72, tone: 'warning' },
    comparison: { label: 'weekly', text: '41%', fraction: 0.41, tone: 'ok' },
    reset: 'in 1h 12m', resetCount: 2, omitted: 1, stale: false,
    detail: {
      facts: [{ label: 'Plan', value: 'Team' }, { label: 'Observed', value: '10:40 local' }],
      windows: [
        { id: 'north-5h', label: '5h window', meter: { label: '5h window', text: '72% · 3.6k / 5k', fraction: 0.72, tone: 'warning' }, facts: [{ label: 'Used', value: '3,600 requests' }] },
        { id: 'north-week', label: 'Weekly', meter: { label: 'weekly', text: '41%', fraction: 0.41, tone: 'ok' }, facts: [{ label: 'Used', value: '41%' }] },
        { id: 'north-month', label: 'Monthly', meter: { label: 'monthly', text: '18%', fraction: 0.18, tone: 'ok' }, facts: [] },
      ],
      resets: [{ label: '5h window', value: '11:52 local' }, { label: 'Weekly', value: 'Monday 09:00 local' }],
    },
  },
  {
    id: 'meter-harbor', label: 'Harbor sandbox', identity: 'personal plan',
    governing: { label: 'weekly', text: 'exhausted!', tone: 'exhausted' },
    comparison: { label: '5h window', text: '100% · 50 / 50', fraction: 1, tone: 'critical' },
    reset: 'in 2d 4h', resetCount: 0, omitted: 0, stale: false, note: 'Requests resume after reset',
    detail: {
      facts: [{ label: 'Plan', value: 'Personal' }],
      windows: [
        { id: 'harbor-week', label: 'Weekly', meter: { label: 'weekly', text: 'exhausted!', tone: 'exhausted' }, facts: [{ label: 'Reported state', value: 'Exhausted without a reported fraction' }] },
        { id: 'harbor-5h', label: '5h window', meter: { label: '5h window', text: '100% · 50 / 50', fraction: 1, tone: 'critical' }, facts: [] },
      ],
      resets: [{ label: 'Weekly', value: 'Thursday 14:00 local' }],
    },
  },
  {
    id: 'meter-meadow', label: 'Meadow lab', identity: 'trial plan',
    governing: { label: 'daily', text: 'unknown', tone: 'unknown' },
    reset: 'unreported', omitted: 2, stale: true,
    detail: {
      facts: [{ label: 'Observed', value: 'yesterday 18:05 local' }],
      windows: [
        { id: 'meadow-day', label: 'Daily', meter: { label: 'daily', text: 'unknown', tone: 'unknown' }, facts: [{ label: 'Reported', value: 'No fraction reported' }] },
        { id: 'meadow-week', label: 'Weekly', meter: { label: 'weekly', text: '12%', fraction: 0.12, tone: 'ok' }, facts: [] },
        { id: 'meadow-month', label: 'Monthly', meter: { label: 'monthly', text: '6%', fraction: 0.06, tone: 'ok' }, facts: [] },
      ],
      resets: [],
    },
  },
]

const denseRows: readonly UsageMeterRow[] = Array.from({ length: 11 }, (_, index): UsageMeterRow => ({
  id: `fictional-account-${index + 1}`, label: `Account ${index + 1}`, identity: `provider-${index % 3 + 1}/account-${index + 1}`,
  governing: { label: '5h', text: `${95 - index * 7}%`, fraction: (95 - index * 7) / 100, tone: index < 3 ? 'critical' : index < 5 ? 'warning' : 'ok' },
  comparison: { label: '1w', text: `${70 - index * 5}%`, fraction: (70 - index * 5) / 100, tone: index < 3 ? 'warning' : 'ok' },
  reset: '2h', ...(index % 3 === 0 ? { resetCount: index % 4 } : {}), omitted: index % 3, stale: false,
  detail: { facts: [{ label: 'Provider', value: `Provider ${index % 3 + 1}` }], windows: [], resets: [] },
}))

const sortKeys: readonly UsageMeterSortKey[] = ['governing', 'reset', 'account', 'comparison', 'more']
type Width = 'wide' | 'narrow'
type HistoryState = 'none' | 'loading' | 'unavailable' | 'ready' | 'empty'
interface Args { scheme: Scheme; width: Width; sortKey: UsageMeterSortKey; direction: 'ascending' | 'descending'; selectedId?: string; status: UsageMeterTableProps['status']['kind']; history: HistoryState }
const statusText: Record<Args['status'], string> = { loading: 'Loading usage…', unreachable: 'Usage source unreachable · showing last-known values', incompatible: 'Usage source version is not supported', current: 'Updated just now', stale: 'Stale · updated 14m ago', unknown: 'Freshness unknown' }

/** Deterministic fictional series: idle days are measured zeroes, one day is unmeasured, the newest period is open. */
const dailyPoints: readonly UsageMeterHistoryPoint[] = Array.from({ length: 30 }, (_, index): UsageMeterHistoryPoint => {
  const date = `2030-04-${String(index + 1).padStart(2, '0')}`
  const value = index === 11 ? undefined : index % 7 === 5 ? 0 : 4 + ((index * 37) % 23) + (index > 20 ? 9 : 0)
  const partial = index === 29
  return { id: date, label: date, ...(value === undefined ? {} : { value }), text: `${value === undefined ? 'unreported' : `$${value.toFixed(2)}`} · ${partial ? 'partial' : 'complete'}`, partial }
})
const histories: Record<Exclude<HistoryState, 'none'>, UsageMeterHistory> = {
  loading: { kind: 'loading', label: 'Usage history', text: 'Loading usage history…' },
  unavailable: { kind: 'unavailable', label: 'Usage history', text: 'Usage history unavailable: source returned HTTP 502' },
  ready: { kind: 'ready', label: 'Usage history', text: '30d · $ max $35.00 · 04-30 partial · 3 older days beyond cap', points: dailyPoints, facts: [{ label: 'Calendar', value: 'Source days' }, { label: 'Older days omitted', value: '3' }] },
  empty: { kind: 'ready', label: 'Usage history', text: 'No recorded usage retained', points: [], facts: [] },
}
const historyOf = (state: HistoryState): { readonly history?: UsageMeterHistory } => state === 'none' ? {} : { history: histories[state] }
const hourlyPoints: readonly UsageMeterHistoryPoint[] = Array.from({ length: 24 }, (_, hour): UsageMeterHistoryPoint => {
  const label = `${String(hour).padStart(2, '0')}:00`
  const value = hour < 7 ? 0 : (hour * 5) % 9
  return { id: label, label, value, text: `${value} merged`, partial: hour === 23 }
})

function Frame({ scheme, width, children }: { scheme: Scheme; width: Width; children: React.ReactNode }) {
  return <div {...stylex.props(styles.frame, ...baselineTheme, scheme === 'light' && lightTheme, width === 'narrow' && styles.narrow)}><ThemePortal>{children}</ThemePortal></div>
}
function Controlled(args: Args & { readonly displayRows?: readonly UsageMeterRow[] }) {
  const [, update] = useArgs<Args>()
  return <Frame scheme={args.scheme} width={args.width}>
    <UsageMeterTable rows={args.displayRows ?? rows} status={{ kind: args.status, text: statusText[args.status] }} sort={{ key: args.sortKey, direction: args.direction }} selectedId={args.selectedId}
      onSort={sort => update({ sortKey: sort.key, direction: sort.direction })}
      onCycleSort={() => update({ sortKey: sortKeys[(sortKeys.indexOf(args.sortKey) + 1) % sortKeys.length]! })}
      onSelect={selectedId => update({ selectedId })}
      onCloseDetail={() => update({ selectedId: undefined })} {...historyOf(args.history)} />
  </Frame>
}
function AllStatesStory() {
  return <div {...stylex.props(styles.matrix)}>{(['dark', 'light'] as const).flatMap(scheme => (['wide', 'narrow'] as const).map(width => <section key={`${scheme}-${width}`} aria-label={`${scheme} ${width}`}>
    <h2 {...stylex.props(styles.caption)}>{scheme} · {width} · detail open</h2>
    <Frame scheme={scheme} width={width}><UsageMeterTable rows={rows} status={{ kind: 'stale', text: statusText.stale }} sort={{ key: 'governing', direction: 'descending' }} selectedId="meter-harbor" onSort={() => {}} onCycleSort={() => {}} onSelect={() => {}} onCloseDetail={() => {}} history={histories.ready} /></Frame>
  </section>))}</div>
}

const meta = {
  title: 'Fractal UI/Usage Meter Table',
  render: Controlled,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'Generic usage monitor over immutable display rows. Sorting, cycling, selection and detail are controlled by the caller. Unknown meters have no bar, exhaustion text stays visible without a fraction, reset counts render only when reported (including zero), and stale rows dim their bars. Narrow containers shrink bars first and fold the hidden comparison into the +N disclosure. The row detail can carry a supplied period history: unmeasured periods are gaps, open periods are marked.' } } },
  args: { scheme: 'dark', width: 'wide', sortKey: 'governing', direction: 'descending', status: 'current', history: 'none' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: ['wide', 'narrow'], control: 'radio' }, sortKey: { options: sortKeys, control: 'select' }, direction: { options: ['ascending', 'descending'], control: 'radio' }, selectedId: { options: [undefined, ...rows.map(row => row.id)], control: 'select' }, status: { options: Object.keys(statusText), control: 'select' }, history: { options: ['none', 'loading', 'unavailable', 'ready', 'empty'], control: 'select' } },
} satisfies Meta<Args>
export default meta
type Story = StoryObj<Args>

export const AllStates: Story = { render: AllStatesStory }
export const Wide: Story = {}
export const Light: Story = { args: { scheme: 'light' } }
export const Narrow: Story = { args: { width: 'narrow' } }
export const DenseEleven: Story = { args: { width: 'narrow' }, render: args => <Controlled {...args} displayRows={denseRows} /> }
export const DenseElevenLight: Story = { ...DenseEleven, args: { width: 'narrow', scheme: 'light' } }
export const Detail: Story = { args: { selectedId: 'meter-north' } }
export const NarrowDetail: Story = { args: { width: 'narrow', selectedId: 'meter-meadow', status: 'unreachable' } }
export const DetailHistory: Story = { args: { selectedId: 'meter-north', history: 'ready' } }
export const NarrowDetailHistory: Story = { args: { width: 'narrow', selectedId: 'meter-north', history: 'ready', scheme: 'light' } }
export const HistoryStates: Story = { render: args => <Frame scheme={args.scheme} width={args.width}>{(['loading', 'unavailable', 'empty', 'ready'] as const).map(state => <UsageMeterHistoryChart key={state} history={histories[state]} />)}</Frame> }
export const HourlyHistogram: Story = { render: args => <Frame scheme={args.scheme} width={args.width}><UsageMeterHistoryChart history={{ kind: 'ready', label: 'Merged per hour', text: '24h · max 8 · 23:00 partial', points: hourlyPoints, facts: [] }} /></Frame> }
export const Empty: Story = { render: args => <Frame scheme={args.scheme} width={args.width}><UsageMeterTable rows={[]} status={{ kind: 'loading', text: statusText.loading }} sort={{ key: args.sortKey, direction: args.direction }} onSort={() => {}} onCycleSort={() => {}} onSelect={() => {}} onCloseDetail={() => {}} /></Frame> }

const styles = stylex.create({
  frame: { width: '720px', maxWidth: '100%', backgroundColor: surface.canvas, color: ink.fg, padding: s.md },
  narrow: { width: '320px' },
  matrix: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.xl, padding: s.xl },
  caption: { fontFamily: t.fontSans, fontSize: t.metaSize, margin: `${s.md} 0` },
})
