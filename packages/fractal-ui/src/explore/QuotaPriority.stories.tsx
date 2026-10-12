import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button, Dialog, DialogTrigger, Popover } from 'react-aria-components'
import { UsageMeterReading, UsageMeterRowDetail, type UsageMeterRow } from '../UsageMeterTable'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { lightTheme } from '../assistant-ui/composition-theme'
import { quotaGeometry } from '../quota-geometry.stylex'
import { spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { quotaPriorityGeometry as g } from './quota-priority.stylex'

const variants = ['truncate', 'hide', 'collapse'] as const
const widths = [200, 240, 320] as const
type Variant = typeof variants[number]
interface Args { variant: Variant; sidebar: 200 | 240 | 320; scheme: 'light' | 'dark'; viewport: 1440 | 900; compare: boolean; probe: boolean }
const accounts: readonly UsageMeterRow[] = [
  { id: 'north', label: 'North studio', identity: 'Team · north-studio', governing: { label: '5h', text: '72%', fraction: .72, tone: 'warning' }, comparison: { label: '1w', text: '41%', fraction: .41, tone: 'ok' }, reset: '1h 12m', omitted: 1, stale: false, detail: { facts: [{ label: 'Plan', value: 'Team' }], windows: [{ id: 'month', label: 'Monthly', meter: { label: '1mo', text: '18%', fraction: .18, tone: 'ok' }, facts: [] }], resets: [{ label: 'Primary reset', value: 'Today at 14:12' }] } },
  { id: 'harbor', label: 'Harbor lab', identity: 'Personal · harbor-lab', governing: { label: '1w', text: '100%', fraction: 1, tone: 'exhausted' }, comparison: { label: '5h', text: '86%', fraction: .86, tone: 'critical' }, reset: '2d 4h', omitted: 1, stale: false, note: 'Exhausted · requests resume after reset', detail: { facts: [{ label: 'State', value: 'Exhausted' }], windows: [{ id: 'month', label: 'Monthly', meter: { label: '1mo', text: '63%', fraction: .63, tone: 'warning' }, facts: [] }], resets: [{ label: 'Primary reset', value: 'Wednesday at 17:00' }] } },
  { id: 'meadow', label: 'Meadow research', identity: 'Pro · meadow-research', governing: { label: '5h', text: '34%', fraction: .34, tone: 'ok' }, comparison: { label: '1w', text: '19%', fraction: .19, tone: 'ok' }, reset: 'Unreported', omitted: 1, stale: false, detail: { facts: [{ label: 'Plan', value: 'Pro' }], windows: [{ id: 'month', label: 'Monthly', meter: { label: '1mo', text: '8%', fraction: .08, tone: 'ok' }, facts: [] }], resets: [{ label: 'Primary reset', value: 'Unreported' }] } },
]

function AccountRow({ row, variant, width, scheme }: { row: UsageMeterRow; variant: Variant; width: Args['sidebar']; scheme: Args['scheme'] }) {
  const [open, setOpen] = React.useState(false)
  const count = variant === 'truncate' ? 5 : width === 200 ? 2 : width === 240 ? 3 : 5
  const hiddenCount = (count < 3 ? 1 : 0) + (count < 4 ? 1 : 0) + row.omitted
  return <>
    <div data-priority-row={row.id} data-visible-count={count} {...stylex.props(styles.row, variant === 'truncate' ? styles.truncate : width === 200 ? styles.compact : width === 240 ? styles.medium : styles.full)}>
      <Button data-priority="1" aria-label={`Account ${row.label} · ${row.identity}${row.note ? ` · ${row.note}` : ''}`} onPress={() => setOpen(true)} {...stylex.props(styles.account, styles.cell)}><span data-line-run>{row.label}</span></Button>
      <div data-priority="2" title={`Primary ${row.governing.label} · ${row.governing.text}${row.note ? ` · ${row.note}` : ''}`} {...stylex.props(styles.cell)}><UsageMeterReading meter={row.governing} stale={row.stale} compact /></div>
      {count >= 3 && <span data-priority="3" title={`Primary reset: ${row.reset}`} {...stylex.props(styles.cell, styles.reset)}><span data-line-run>{row.reset}</span></span>}
      {count >= 4 && <div data-priority="4" title={`Secondary ${row.comparison?.label} · ${row.comparison?.text}`} {...stylex.props(styles.cell)}>{row.comparison && <UsageMeterReading meter={row.comparison} stale={row.stale} compact />}</div>}
      {variant === 'collapse' || count === 5 ? <DialogTrigger><Button aria-label={`Show ${hiddenCount} more readings for ${row.label}`} data-priority={count === 5 ? '5' : undefined} data-overflow {...stylex.props(styles.button, styles.cell)}>+{hiddenCount}</Button><Popover {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.popover)}><Dialog aria-label={`More readings for ${row.label}`} {...stylex.props(styles.dialog)}>{({ close }) => <><h3 {...stylex.props(styles.heading)}>{row.label}</h3>{count < 3 && <p>Primary reset: {row.reset}</p>}{count < 4 && row.comparison && <UsageMeterReading meter={row.comparison} stale={row.stale} compact />}{row.detail.windows.map(window => <UsageMeterReading key={window.id} meter={window.meter} stale={row.stale} compact />)}<Button onPress={close} {...stylex.props(styles.button)}>Close readings</Button></>}</Dialog></Popover></DialogTrigger> : null}
    </div>
    {open && <UsageMeterRowDetail row={row} onClose={() => setOpen(false)} />}
  </>
}
function Sidebar({ variant, width, scheme }: { variant: Variant; width: Args['sidebar']; scheme: Args['scheme'] }) {
  return <section data-sidebar={width} data-variant={variant} aria-label={`${variant} · ${width} sidebar`} {...stylex.props(styles.sidebar, width === 200 ? styles.width200 : width === 240 ? styles.width240 : styles.width320)}><h3 {...stylex.props(styles.heading)}>{width} · {variant}</h3><div {...stylex.props(styles.nav)}>Workspace / Sessions</div><header {...stylex.props(styles.caption)}>Accounts · primary · reset · next</header>{accounts.map(row => <AccountRow key={row.id} row={row} variant={variant} width={width} scheme={scheme} />)}</section>
}
function Explorer(initial: Args & { onArgsChange?: (patch: Partial<Args>) => void }) {
  const [probeArgs, setProbeArgs] = React.useState(initial)
  const args = initial.probe ? probeArgs : initial
  const update = (patch: Partial<Args>) => initial.probe ? setProbeArgs(current => ({ ...current, ...patch })) : initial.onArgsChange?.(patch)
  return <main {...stylex.props(...baselineTheme, styles.root)}>
    <h1 {...stylex.props(styles.heading)}>Quota under pressure</h1><p {...stylex.props(styles.copy)}>Account first, then primary usage, then its reset. Secondary windows yield before those three. Every account stays on one line.</p>
    <div {...stylex.props(styles.controls)}>{variants.map(variant => <Button key={variant} aria-label={`Choose ${variant}`} aria-pressed={args.variant === variant} onPress={() => update({ variant, compare: false })} {...stylex.props(styles.button)}>{variant}</Button>)}{widths.map(sidebar => <Button key={sidebar} aria-label={`Sidebar ${sidebar}`} onPress={() => update({ sidebar, compare: false })} {...stylex.props(styles.button)}>{sidebar}</Button>)}<Button onPress={() => update({ compare: !args.compare })} {...stylex.props(styles.button)}>Compare all</Button><Button onPress={() => update({ scheme: args.scheme === 'dark' ? 'light' : 'dark', compare: false })} {...stylex.props(styles.button)}>Switch theme</Button><Button onPress={() => update({ viewport: args.viewport === 1440 ? 900 : 1440, compare: false })} {...stylex.props(styles.button)}>Switch host width</Button></div>
    <div data-matrix {...stylex.props(styles.matrix)}>{(args.compare ? [1440, 900] as const : [args.viewport]).flatMap(viewport => (args.compare ? ['dark', 'light'] as const : [args.scheme]).map(scheme => <section key={`${viewport}-${scheme}`} data-host={viewport} data-scheme={scheme} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.host, viewport === 1440 ? styles.host1440 : styles.host900)}><h2 {...stylex.props(styles.heading)}>{viewport} host · {scheme}</h2>{(args.compare ? variants : [args.variant]).map(variant => <div key={variant} {...stylex.props(styles.sidebars)}>{(args.compare ? widths : [args.sidebar]).map(width => <Sidebar key={width} variant={variant} width={width} scheme={scheme} />)}</div>)}</section>))}</div>
  </main>
}
const meta = {
  title: 'Fractal UI/Explore/QuotaPriority', component: Explorer,
  render: function Render(args) {
    const [, updateArgs] = useArgs<Args>()
    return <Explorer {...args} onArgsChange={updateArgs} />
  },
  parameters: { layout: 'fullscreen' },
  args: { variant: 'collapse', sidebar: 240, scheme: 'dark', viewport: 1440, compare: true, probe: false },
  argTypes: { variant: { control: 'select', options: variants }, sidebar: { control: 'select', options: widths }, scheme: { control: 'select', options: ['light', 'dark'] }, viewport: { control: 'select', options: [1440, 900] }, probe: { table: { disable: true } }, onArgsChange: { table: { disable: true } } },
} satisfies Meta<typeof Explorer>
export default meta
type Story = StoryObj<typeof meta>
export const Exploration: Story = {}
export const AllStates: Story = {}

function assertRows(canvas: HTMLElement) {
  const sidebars = canvas.querySelectorAll<HTMLElement>('[data-sidebar]')
  expect(sidebars.length).toBeGreaterThan(0)
  for (const sidebar of sidebars) {
    const width = Number(sidebar.dataset.sidebar)
    expect(Math.round(sidebar.getBoundingClientRect().width)).toBe(width)
    const count = sidebar.dataset.variant === 'truncate' ? 5 : width === 200 ? 2 : width === 240 ? 3 : 5
    for (const row of sidebar.querySelectorAll<HTMLElement>('[data-priority-row]')) {
      expect(Math.round(row.getBoundingClientRect().height)).toBe(24)
      const priorities = Array.from(row.querySelectorAll<HTMLElement>('[data-priority]')).map(cell => Number(cell.dataset.priority))
      expect(priorities).toEqual(Array.from({ length: count }, (_, index) => index + 1))
      for (const cell of row.querySelectorAll<HTMLElement>('[data-priority]')) {
        expect(cell.getBoundingClientRect().width).toBeGreaterThan(0)
        expect(getComputedStyle(cell).whiteSpace).toBe('nowrap')
        expect(cell.getBoundingClientRect().height).toBeLessThanOrEqual(24)
        for (const run of cell.querySelectorAll('[data-line-run]')) {
          const range = document.createRange(); range.selectNodeContents(run)
          // Chromium reports a full text rect plus its ellipsis rect on the same line.
          const rects = Array.from(range.getClientRects())
          expect(rects.length).toBeGreaterThan(0)
          expect(new Set(rects.map(rect => Math.round(rect.top))).size).toBe(1)
        }
      }
      const rowBox = row.getBoundingClientRect()
      expect(row.scrollWidth).toBeLessThanOrEqual(Math.ceil(rowBox.width))
    }
  }
}
export const QuotaPriorityProbe: Story = { args: { probe: true }, play: async ({ canvasElement, step }) => {
  const canvas = within(canvasElement)
  await step('Complete theme, host and sidebar matrix', async () => { await waitFor(() => expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(36)); assertRows(canvasElement) })
  await step('Reject missing higher priority and wrapping controls, restore', async () => {
    const primary = canvasElement.querySelector<HTMLElement>('[data-priority="2"]')!
    const parent = primary.parentElement!
    const next = primary.nextSibling
    primary.remove()
    expect(() => assertRows(canvasElement)).toThrow()
    parent.insertBefore(primary, next)
    primary.style.whiteSpace = 'normal'
    expect(() => assertRows(canvasElement)).toThrow()
    primary.style.removeProperty('white-space')
    assertRows(canvasElement)
  })
  for (const variant of variants) for (const width of widths) await step(`${variant} at ${width} after controls rerender`, async () => {
    await userEvent.click(canvas.getByRole('button', { name: `Choose ${variant}` }))
    await userEvent.click(canvas.getByRole('button', { name: `Sidebar ${width}` }))
    await waitFor(() => { expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(1); expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-sidebar')).toBe(String(width)); expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-variant')).toBe(variant) })
    assertRows(canvasElement)
    if (variant === 'collapse') {
      await userEvent.click(canvas.getByRole('button', { name: /Show .* more readings for North studio/ }))
      const dialog = await within(document.body).findByRole('dialog', { name: 'More readings for North studio' })
      if (width === 200) expect(within(dialog).getByText('Primary reset: 1h 12m')).toBeVisible()
      expect(within(dialog).getByText('18%')).toBeVisible()
      await userEvent.click(within(dialog).getByRole('button', { name: 'Close readings' }))
      await waitFor(() => expect(dialog).not.toBeInTheDocument()); assertRows(canvasElement)
    }
  })
  await step('Real account details and unreported evidence', async () => {
    await userEvent.click(canvas.getByRole('button', { name: /Account Meadow research/ }))
    expect(await canvas.findByRole('region', { name: 'Meadow research usage details' })).toBeVisible()
    expect(canvas.getAllByText('Unreported').length).toBeGreaterThan(0)
    await userEvent.click(canvas.getByRole('button', { name: 'Close usage details' }))
    await waitFor(() => expect(canvas.queryByRole('region', { name: 'Meadow research usage details' })).not.toBeInTheDocument()); assertRows(canvasElement)
    const nextScheme = canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme') === 'dark' ? 'light' : 'dark'
    await userEvent.click(canvas.getByRole('button', { name: 'Switch theme' })); await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme')).toBe(nextScheme)); assertRows(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: 'Switch host width' })); await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-host')).toBe('900')); assertRows(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: 'Compare all' })); await waitFor(() => expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(36)); assertRows(canvasElement)
  })
} }

const styles = stylex.create({
  root: { padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  heading: { margin: 0, paddingBlock: s.md, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  copy: { color: ink.fgSoft, margin: 0, paddingBlock: s.md },
  controls: { display: 'flex', gap: s.md, flexWrap: 'wrap', paddingBlock: s.lg },
  button: { color: ink.fgSoft, font: 'inherit', backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover }, border: 0, padding: s.xs, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline } },
  matrix: { display: 'flex', gap: s.section, alignItems: 'flex-start', overflowX: 'auto' },
  host: { flexShrink: 0, backgroundColor: surface.canvas, color: ink.fg, padding: s.md, boxSizing: 'border-box', border: quotaGeometry.hair },
  host1440: { width: g.host1440 }, host900: { width: g.host900 },
  sidebars: { display: 'flex', gap: s.section, paddingBlock: s.lg },
  sidebar: { flexShrink: 0, backgroundColor: surface.raised, color: ink.fg, boxSizing: 'border-box' },
  width200: { width: g.sidebar200 }, width240: { width: g.sidebar240 }, width320: { width: g.sidebar320 },
  nav: { color: ink.fgSoft, paddingBlock: s.section, borderBottom: quotaGeometry.hair },
  caption: { color: ink.fgMuted, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', paddingBlock: s.md },
  row: { display: 'grid', height: g.row, alignItems: 'center', fontSize: t.denseSize, lineHeight: t.metaLeading, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover } },
  truncate: { gridTemplateColumns: g.truncateGrid }, compact: { gridTemplateColumns: g.compactGrid }, medium: { gridTemplateColumns: g.mediumGrid }, full: { gridTemplateColumns: g.fullGrid },
  cell: { minWidth: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxHeight: g.row },
  account: { color: ink.fg, font: 'inherit', textAlign: 'start', padding: 0, border: 0, backgroundColor: surface.transparent, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline } },
  reset: { color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
  popover: { backgroundColor: surface.raised, color: ink.fg, border: quotaGeometry.strongHair },
  dialog: { padding: s.lg, color: ink.fg, backgroundColor: surface.raised, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, outline: 'none' },
})
