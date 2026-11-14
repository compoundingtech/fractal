import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as React from 'react'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { SidebarAgentRow, assistantDarkTheme, compositionLightTheme, liveAccentTheme, type SidebarAgentRowData } from '@fractal/ui/assistant-ui/shell'
import { RegistryContext } from '@effect/atom-react'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'

import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { colorVars as c, geometryNumbers, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { SidebarTabs } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarTabs.tsx'
import { accountOf, FIXTURE_NOW, incompatibleSummaryAccount, quotaEnvelope } from './fixtures.ts'
import { initialQuota, reduceQuota, type QuotaState } from './model.ts'
import { UsageSection } from './UsageSection.tsx'
import { usageSectionView } from './usageSectionView.ts'
import type { QuotaAccount } from './wire.ts'
import { usageSectionHeight, usageSectionOpen } from './usageSectionState.ts'

const HOUR = 3_600_000
const DAY = 24 * HOUR
const now = FIXTURE_NOW

/** The shape of a real fleet with made-up accounts: limits, ok, a window with no reading since its reset. */
const quota = ({ id, short, percent, resetsIn, exhausted = false, extra = [], retainedFor }: {
  readonly id: string
  readonly short: string
  readonly percent: number
  readonly resetsIn: number
  readonly exhausted?: boolean
  readonly extra?: ReadonlyArray<{ readonly short: string; readonly percent: number; readonly exhausted?: boolean }>
  readonly retainedFor?: number
}): QuotaAccount => {
  const [harness = '', name = ''] = id.split('/')
  return accountOf({
    accountId: id, harness, provider: harness, name, label: name,
    ...(retainedFor === undefined ? {} : { retained: { observedAgo: retainedFor, reason: 'source did not answer' } }),
    limits: [
      { id: short, label: short, short, slot: 'governing', fraction: percent / 100, exhausted, resetsIn },
      ...extra.map(limit => ({ id: limit.short, label: limit.short, short: limit.short, slot: 'none' as const, fraction: limit.percent / 100, exhausted: limit.exhausted ?? false, resetsIn: DAY })),
    ],
  })
}
const fleet: readonly QuotaAccount[] = [
  quota({ id: 'codex/team-north', short: '7d', percent: 100, exhausted: true, resetsIn: 4 * DAY, extra: [{ short: '5h', percent: 12 }] }),
  quota({ id: 'copilot/team-north', short: 'Mon', percent: 100, exhausted: true, resetsIn: 24 * DAY }),
  quota({ id: 'zai/personal', short: '1w', percent: 100, exhausted: true, resetsIn: 4 * DAY, extra: [{ short: '5h', percent: 31 }] }),
  quota({ id: 'grok/personal', short: '1w', percent: 99, resetsIn: 2 * DAY }),
  quota({ id: 'codex/studio', short: '7d', percent: 72, resetsIn: 3 * DAY }),
  quota({ id: 'codex/contractor', short: '7d', percent: 60, resetsIn: 3 * DAY }),
  quota({ id: 'claude/personal', short: '7d', percent: 28, resetsIn: 2 * DAY, extra: [{ short: 'Fbl', percent: 0 }] }),
  quota({ id: 'claude/a-rather-long-organisation-account-name', short: '7d', percent: 18, resetsIn: 5 * DAY, extra: [{ short: 'Fbl', percent: 0 }, { short: 'Son', percent: 44 }] }),
  quota({ id: 'opencode/personal', short: 'Mon', percent: 40, resetsIn: -17 * DAY }),
]

const loaded = (list: readonly QuotaAccount[], generatedAgo = 40_000): QuotaState =>
  reduceQuota({ state: initialQuota(true), event: { _tag: 'Succeeded', at: now, envelope: quotaEnvelope({ list, generatedAgo }) } })
const failed = (state: QuotaState, kind: 'unavailable' | 'timeout' | 'unreachable' | 'incompatible'): QuotaState =>
  reduceQuota({ state, event: { _tag: 'Failed', at: now, failure: { kind, reason: 'story' } } })

const states = {
  ready: loaded(fleet),
  loading: initialQuota(true),
  'no quota source': failed(initialQuota(true), 'unavailable'),
  'first read failed': failed(initialQuota(true), 'unreachable'),
  'other format': failed(initialQuota(true), 'incompatible'),
  empty: loaded([]),
  'stale, read failed': failed(loaded(fleet.slice(0, 5)), 'timeout'),
  'stale, past horizon': loaded(fleet.slice(3, 7), 2 * HOUR),
  'one account held': loaded([quota({ id: 'codex/held', short: '7d', percent: 100, exhausted: true, resetsIn: DAY, retainedFor: 3 * HOUR }), quota({ id: 'claude/personal', short: '7d', percent: 28, resetsIn: 2 * DAY })]),
  unreadable: loaded([incompatibleSummaryAccount]),
} satisfies Record<string, QuotaState>
type StateName = keyof typeof states

const row = (title: string): SidebarAgentRowData => ({ ref: `agent/${title}`, id: `agent/${title}`, title, host: 'local', status: 'idle', statusLabel: 'Idle', freshness: 'live', children: [],
  usage: { _tag: 'Known', scope: '24h-root-and-subagents', usd: 4.2, tokens: 2_300_000 }, duration: { _tag: 'Known', scope: '24h-activity-span', ms: HOUR }, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: now - 180_000 } })
const roster = ['Review the sidebar', 'Ship the relay', 'Tidy fixtures', 'Migrate tokens', 'Fix stream retry', 'Write release notes', 'Audit palettes', 'Rebuild daemon', 'Trim footer', 'Check narrow drawer'].map(row)

interface Args { readonly state: StateName; readonly open: boolean; readonly width: number; readonly narrow: boolean; readonly scheme: 'dark' | 'light'; readonly onOpenUsage: () => void }

const SeededSection = ({ open, children }: { readonly open: boolean; readonly children: React.ReactNode }) => {
  const [registry] = React.useState(() => AtomRegistry.make({ initialValues: [[usageSectionOpen, open], [usageSectionHeight, geometryNumbers.drawerDefault]] }))
  React.useEffect(() => () => registry.dispose(), [registry])
  return <RegistryContext.Provider value={registry}>{children}</RegistryContext.Provider>
}

/** The real tabs, agent rows and section, at the sidebar's width, so the boundary above the section is the one live has. */
const InSidebar = ({ state, open, width, narrow, onOpenUsage, label }: Args & { readonly label: string }) => {
  const [mounted, setMounted] = React.useState(0)
  return <SeededSection open={open}><NarrowLayoutContext.Provider value={narrow}>
    <aside aria-label={label} data-usage-frame={label} style={{ inlineSize: width }} {...stylex.props(styles.frame)}>
      <SidebarTabs panelHasTabbableContent selectedId="agents" onSelectionChange={() => undefined} tabs={[{ id: 'agents', label: 'Agents' }, { id: 'alerts', label: 'Alerts' }]}>
        <nav aria-label={`${label} roster`} {...stylex.props(styles.roster)}>
          {roster.map(item => <SidebarAgentRow key={item.ref} item={item} now={now} active={false} onOpen={() => undefined} />)}
        </nav>
        <UsageSection key={mounted} view={usageSectionView({ state: states[state], now })} onOpenUsage={onOpenUsage} />
      </SidebarTabs>
    </aside>
    <button type="button" data-remount={label} tabIndex={-1} aria-hidden="true" style={{ display: 'none' }} onClick={() => setMounted(value => value + 1)} />
  </NarrowLayoutContext.Provider></SeededSection>
}

const Surface = (args: Args) =>
  <main data-scheme={args.scheme} {...stylex.props(styles.root, args.scheme === 'dark' ? assistantDarkTheme : compositionLightTheme, liveAccentTheme)}>
    <InSidebar {...args} label={args.state} />
  </main>

const AllStatesSurface = (args: Args) =>
  <main data-scheme={args.scheme} {...stylex.props(styles.root, styles.grid, args.scheme === 'dark' ? assistantDarkTheme : compositionLightTheme, liveAccentTheme)}>
    {(Object.keys(states) as StateName[]).flatMap(name => [false, true].map(open =>
      <figure key={`${name}-${open}`} {...stylex.props(styles.specimen)}>
        <figcaption {...stylex.props(styles.caption)}>{name} · {open ? 'open' : 'collapsed'}</figcaption>
        <div {...stylex.props(styles.bare)} style={{ inlineSize: args.width }}><SeededSection open={open}><UsageSection view={usageSectionView({ state: states[name], now })} onOpenUsage={args.onOpenUsage} /></SeededSection></div>
      </figure>))}
  </main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/UsageSection',
  component: UsageSection,
  render: Surface,
  args: { state: 'ready', open: true, width: 256, narrow: false, scheme: 'dark', onOpenUsage: fn() },
  argTypes: { state: { control: 'select', options: Object.keys(states) }, scheme: { control: 'radio', options: ['dark', 'light'] }, onOpenUsage: { table: { disable: true } } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Made-up v3 quota envelopes through the monitor model pin the section at sidebar density: worst-first order, at-limit text, and each honest state.' } },
} satisfies Meta<Args & React.ComponentProps<typeof UsageSection>>
export default meta
type Story = StoryObj<Args>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'AllStatesLight:contract-fixture',
  'Collapsed:contract-fixture',
  'KeyboardAndMemory:contract-fixture',
  'Light:contract-fixture',
  'MinimumWidth:contract-fixture',
  'NarrowDrawer:contract-fixture',
  'NoQuotaSource:contract-fixture',
  'Open:contract-fixture',
]

const section = (canvas: HTMLElement) => canvas.querySelector<HTMLElement>('[data-usage-section]')!

export const Open: Story = {
  play: async ({ canvasElement, args }) => {
    const rows = [...canvasElement.querySelectorAll<HTMLElement>('[data-usage-row]')]
    await expect(rows[0]).toHaveAttribute('data-usage-level', 'limit')
    await expect(rows[0]).toHaveTextContent('100%!')
    await expect(rows.at(-1)).toHaveAttribute('data-usage-row', 'opencode/personal')
    await expect(rows.at(-1)).toHaveTextContent('reset 17d ago')
    for (const element of rows) await expect(element.scrollWidth).toBeLessThanOrEqual(element.clientWidth)
    await userEvent.click(rows[0]!)
    await expect(args.onOpenUsage).toHaveBeenCalledTimes(1)
  },
}

export const Collapsed: Story = {
  args: { open: false },
  play: async ({ canvasElement }) => {
    const trigger = within(section(canvasElement)).getByRole('button', { name: /^Usage codex\/team-north/ })
    await expect(trigger).toHaveAttribute('aria-expanded', 'false')
    await expect(trigger).toHaveTextContent('100%!')
    await expect(canvasElement.querySelector('[data-usage-row]')).not.toBeVisible()
    await expect(trigger.scrollWidth).toBeLessThanOrEqual(trigger.clientWidth)
  },
}

export const KeyboardAndMemory: Story = {
  args: { open: false, state: 'ready' },
  play: async ({ canvasElement }) => {
    const trigger = within(section(canvasElement)).getByRole('button', { name: /^Usage\b/ })
    trigger.focus()
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(trigger).toHaveAttribute('aria-expanded', 'true'))
    const resizer = within(section(canvasElement)).getByRole('separator', { name: 'Resize Usage section' })
    const before = Number(resizer.getAttribute('aria-valuenow'))
    resizer.focus()
    await userEvent.keyboard('{ArrowUp}')
    await waitFor(() => expect(Number(resizer.getAttribute('aria-valuenow'))).toBe(before + 8))
    canvasElement.querySelector<HTMLElement>('[data-remount]')!.click()
    await waitFor(() => expect(within(section(canvasElement)).getByRole('button', { name: /^Usage\b/ })).toHaveAttribute('aria-expanded', 'true'))
    await expect(Number(within(section(canvasElement)).getByRole('separator').getAttribute('aria-valuenow'))).toBe(before + 8)
    within(section(canvasElement)).getByRole('button', { name: /^Usage\b/ }).focus()
    await userEvent.keyboard(' ')
    await waitFor(() => expect(canvasElement.querySelector('[data-usage-row]')).not.toBeVisible())
  },
}

export const NoQuotaSource: Story = {
  args: { state: 'no quota source', open: false },
  play: async ({ canvasElement, args }) => {
    await expect(section(canvasElement)).toHaveTextContent('No quota source configured')
    await userEvent.click(within(section(canvasElement)).getByRole('button', { name: 'Open Usage' }))
    await expect(args.onOpenUsage).toHaveBeenCalled()
  },
}

export const NarrowDrawer: Story = {
  args: { narrow: true, width: 240 },
  play: async ({ canvasElement }) => {
    await expect(within(section(canvasElement)).queryByRole('separator')).toBeNull()
    await expect(within(section(canvasElement)).getByRole('button', { name: /^Usage\b/ }).getBoundingClientRect().height).toBeGreaterThanOrEqual(44)
  },
}

export const MinimumWidth: Story = {
  args: { width: 208 },
  play: async ({ canvasElement }) => {
    for (const element of canvasElement.querySelectorAll<HTMLElement>('[data-usage-row]')) await expect(element.scrollWidth).toBeLessThanOrEqual(element.clientWidth)
  },
}

export const Light: Story = { args: { scheme: 'light' } }

export const AllStates: Story = { render: AllStatesSurface }

export const AllStatesLight: Story = { render: AllStatesSurface, args: { scheme: 'light' } }

const styles = stylex.create({
  root: { minHeight: '100dvh', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  frame: { display: 'flex', flexDirection: 'column', blockSize: '100dvh', backgroundColor: c.sidebar, borderInlineEndWidth: 1, borderInlineEndStyle: 'solid', borderInlineEndColor: c.border },
  roster: { flexGrow: 1, minHeight: 0, overflowY: 'auto' },
  grid: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(256px, 1fr))', gap: 24, padding: 16, alignItems: 'start' },
  specimen: { margin: 0, display: 'flex', flexDirection: 'column', gap: 6 },
  caption: { color: c.fgMuted, fontSize: t.denseSize },
  bare: { display: 'flex', flexDirection: 'column', maxInlineSize: '100%', backgroundColor: c.sidebar },
})
