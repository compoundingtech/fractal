import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button, Dialog, DialogTrigger, Popover, VisuallyHidden } from 'react-aria-components'
import { Tooltip } from '../assistant-ui/composition/Controls'
import { UsageMeterReading, UsageMeterRowDetail, type UsageMeterRow } from '../UsageMeterTable'
import { usageMeterGridVars } from '../UsageMeterGrid.stylex'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { lightTheme } from '../assistant-ui/composition-theme'
import { quotaGeometry } from '../quota-geometry.stylex'
import { spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { quotaPriorityGeometry as g } from './quota-priority.stylex'
import { quotaPriorityAccounts as accounts } from './quota-priority-fixtures'

const drops = [{ id: 'D1', description: 'Truncate secondary columns; retain the primary reset in full.' }, { id: 'D2', description: 'Hide secondary columns at narrow widths; account details retain the evidence.' }, { id: 'D3', description: 'Collapse secondary windows into a +N readings popover.' }] as const
const tables = [{ id: 'T1', description: 'One aligned table, without a column header row.' }, { id: 'T2', description: 'The same table, with named column headers.' }] as const
const gaps = [{ id: 'G1', description: '8 between columns.' }, { id: 'G2', description: '12 between columns.' }, { id: 'G3', description: '16 between columns.' }] as const
const widths = [{ id: 200, description: '200-wide sidebar under pressure.' }, { id: 240, description: '240-wide sidebar.' }, { id: 320, description: '320-wide sidebar with secondary usage.' }] as const
const bars = [{ id: 'B1', description: 'Keep the meter bar at every width; the primary slot fits "1w ▬ 100%".' }, { id: 'B2', description: 'Below 320, drop the meter bar (keep "5h 72%") so the account keeps more characters.' }] as const
interface Args { D: typeof drops[number]['id']; T: typeof tables[number]['id']; G: typeof gaps[number]['id']; W: typeof widths[number]['id']; B: typeof bars[number]['id']; scheme: 'light' | 'dark'; compare: boolean; viewport: 1440 | 900; probe: boolean }

function Picker<Value extends string | number>({ label, value, choices, onChange }: { label: string; value: Value; choices: readonly { id: Value; description: string }[]; onChange: (value: Value) => void }) {
  return <div role="group" aria-label={label} {...stylex.props(styles.picker)}><strong {...stylex.props(styles.pickerLabel)}>{label}</strong>{choices.map(choice => <Button key={choice.id} aria-label={`${label} ${choice.id}`} aria-pressed={value === choice.id} onPress={() => onChange(choice.id)} {...stylex.props(styles.button, value === choice.id && styles.selected)}>{choice.id}</Button>)}<span data-picker-description={label} {...stylex.props(styles.copy)}>{choices.find(choice => choice.id === value)?.description}</span></div>
}

function AccountRow({ row, args, onSelect }: { row: UsageMeterRow; args: Args; onSelect: (id: string) => void }) {
  const full = args.D === 'D1' || args.W === 320
  const bare = args.B === 'B2' && args.W !== 320
  const governing = bare ? { ...row.governing, fraction: undefined } : row.governing
  const comparison = bare && row.comparison ? { ...row.comparison, fraction: undefined } : row.comparison
  const extraCount = row.omitted + (!full && row.comparison ? 1 : 0)
  const reset = row.reset === 'Unreported' ? '—' : row.reset.replace(/\s/g, '')
  return <div role="row" data-priority-row={row.id} data-stale={row.stale || undefined} data-visible-count={full ? 5 : 3} {...stylex.props(styles.row)}>
    <div role="cell" data-priority="1" {...stylex.props(styles.cell, styles.accountCell)}><Tooltip label={row.identity}><Button aria-label={`Account ${row.label}${row.stale ? ' · Stale' : ''}${row.note ? ` · ${row.note}` : ''}`} aria-describedby={`${args.D}-${row.id}-identity`} onPress={() => onSelect(row.id)} {...stylex.props(styles.account)}><span data-line-run>{row.identity}</span></Button></Tooltip><VisuallyHidden id={`${args.D}-${row.id}-identity`}>{row.identity}</VisuallyHidden></div>
    <div role="cell" data-priority="2" data-reading aria-label={`Primary ${row.governing.label} · ${row.governing.text}${row.stale ? ' · Stale' : ''}${row.note ? ` · ${row.note}` : ''}`} {...stylex.props(styles.cell)}><UsageMeterReading meter={governing} stale={row.stale} compact /></div>
    <div role="cell" data-priority="3" data-reset aria-label={row.reset === 'Unreported' ? 'Reset unreported' : `Primary reset: ${row.reset}`} {...stylex.props(styles.cell, styles.reset)}><span data-line-run>{reset}</span></div>
    {full && <div role="cell" data-priority="4" data-reading={args.D === 'D1' ? undefined : ''} aria-label={`Secondary ${row.comparison?.label} · ${row.comparison?.text}`} {...stylex.props(styles.cell)}>{comparison && <UsageMeterReading meter={comparison} stale={row.stale} compact />}</div>}
    {(args.D === 'D3' || full) && <div role="cell" data-priority={full ? '5' : undefined} data-overflow-cell {...stylex.props(styles.cell)}><DialogTrigger><Button aria-label={`Show ${extraCount} secondary windows for ${row.label}`} data-overflow-count={extraCount} {...stylex.props(styles.more)}>+{extraCount}</Button><Popover {...stylex.props(...baselineTheme, args.scheme === 'light' && lightTheme, styles.popover)}><Dialog aria-label={`Secondary windows for ${row.label}`} {...stylex.props(styles.dialog)}>{({ close }) => <><h3 {...stylex.props(styles.heading)}>{row.label}</h3>{!full && row.comparison && <UsageMeterReading meter={row.comparison} stale={row.stale} compact />}{row.detail.windows.filter(window => window.id !== `${row.id}-primary` && window.id !== `${row.id}-secondary`).map(window => <UsageMeterReading key={window.id} meter={window.meter} stale={row.stale} compact />)}<Button onPress={close} {...stylex.props(styles.button)}>Close readings</Button></>}</Dialog></Popover></DialogTrigger></div>}
  </div>
}

function QuotaTable({ args }: { args: Args }) {
  const [selectedId, setSelectedId] = React.useState<string>()
  const selected = accounts.find(row => row.id === selectedId)
  const full = args.D === 'D1' || args.W === 320
  const headers = full ? ['Account', 'Primary', 'Reset', 'Secondary', 'More'] : args.D === 'D3' ? ['Account', 'Primary', 'Reset', 'More'] : ['Account', 'Primary', 'Reset']
  const bare = args.B === 'B2' && args.W !== 320
  return <section data-sidebar={args.W} data-drop={args.D} data-table-form={args.T} data-gap={args.G} data-bar={args.B} {...stylex.props(styles.sidebar, args.W === 200 ? styles.width200 : args.W === 240 ? styles.width240 : styles.width320)}>
    <h2 {...stylex.props(styles.heading)}>{args.D} · {args.W}</h2>
    <div role="table" aria-label={`${args.D} quota accounts · ${args.T} · ${args.G} · ${args.W} · ${args.B} · ${args.scheme}`} data-quota-table {...stylex.props(styles.table, args.G === 'G1' ? styles.gap8 : args.G === 'G2' ? styles.gap12 : styles.gap16, args.D === 'D1' ? (bare ? styles.truncateBare : styles.truncate) : full ? styles.full : args.D === 'D3' ? (bare ? styles.collapsedBare : styles.collapsed) : bare ? styles.hiddenBare : styles.hidden)}>
      {args.T === 'T2' && <div role="row" data-column-headers {...stylex.props(styles.row, styles.header)}>{headers.map((header, index) => <div key={header} role="columnheader" {...stylex.props(styles.cell, index === 0 && styles.accountCell)}>{header}</div>)}</div>}
      {accounts.map(row => <AccountRow key={row.id} row={row} args={args} onSelect={setSelectedId} />)}
    </div>
    {selected && <UsageMeterRowDetail row={selected} onClose={() => setSelectedId(undefined)} />}
  </section>
}

function Explorer(args: Args & { onChange: (patch: Partial<Args>) => void }) {
  return <main data-current-drop={args.D} data-current-table={args.T} data-current-gap={args.G} data-current-width={args.W} data-current-bar={args.B} data-current-compare={String(args.compare)} {...stylex.props(...baselineTheme, args.scheme === 'light' && lightTheme, styles.root)}>
    <h1 {...stylex.props(styles.heading)}>Quota under pressure</h1>
    <Picker label="D" value={args.D} choices={drops} onChange={D => args.onChange({ D })} />
    <Picker label="T" value={args.T} choices={tables} onChange={T => args.onChange({ T })} />
    <Picker label="G" value={args.G} choices={gaps} onChange={G => args.onChange({ G })} />
    <Picker label="W" value={args.W} choices={widths} onChange={W => args.onChange({ W })} />
    <Picker label="B" value={args.B} choices={bars} onChange={B => args.onChange({ B })} />
    <div {...stylex.props(styles.picker)}><Button aria-pressed={args.scheme === 'light'} onPress={() => args.onChange({ scheme: args.scheme === 'dark' ? 'light' : 'dark' })} {...stylex.props(styles.button)}>Light scheme</Button><Button aria-pressed={args.compare} onPress={() => args.onChange({ compare: !args.compare })} {...stylex.props(styles.button)}>Compare policies</Button><Button onPress={() => args.onChange({ viewport: args.viewport === 1440 ? 900 : 1440 })} {...stylex.props(styles.button)}>Host {args.viewport}</Button></div>
    <div data-host={args.viewport} data-scheme={args.scheme} {...stylex.props(styles.host, args.viewport === 1440 ? styles.host1440 : styles.host900)}><div {...stylex.props(styles.previews)}>{(args.compare ? drops.map(drop => drop.id) : [args.D]).map(D => <QuotaTable key={D} args={{ ...args, D }} />)}</div></div>
  </main>
}
function ProbeExplorer(initial: Args) {
  const [args, setArgs] = React.useState(initial)
  return <Explorer {...args} onChange={patch => setArgs(current => ({ ...current, ...patch }))} />
}
const meta = {
  title: 'Fractal UI/Explore/QuotaPriority',
  render: function Render() {
    const [args, updateArgs] = useArgs<Args>()
    return args.probe ? <ProbeExplorer {...args} /> : <Explorer {...args} onChange={updateArgs} />
  },
  parameters: { layout: 'fullscreen' },
  args: { D: 'D3', T: 'T1', G: 'G1', W: 240, B: 'B1', scheme: 'dark', compare: false, viewport: 1440, probe: false },
  argTypes: { D: { control: 'select', options: drops.map(choice => choice.id) }, T: { control: 'select', options: tables.map(choice => choice.id) }, G: { control: 'select', options: gaps.map(choice => choice.id) }, W: { control: 'select', options: widths.map(choice => choice.id) }, B: { control: 'select', options: bars.map(choice => choice.id) }, scheme: { control: 'select', options: ['light', 'dark'] }, compare: { control: 'boolean' }, viewport: { control: 'select', options: [1440, 900] }, probe: { table: { disable: true } } },
} satisfies Meta<Args>
export default meta
type Story = StoryObj<typeof meta>
export const Exploration: Story = {}

function assertTables(canvas: HTMLElement) {
  const sidebars = canvas.querySelectorAll<HTMLElement>('[data-sidebar]')
  expect(sidebars.length).toBeGreaterThan(0)
  for (const sidebar of sidebars) {
    const width = Number(sidebar.dataset.sidebar)
    const full = sidebar.dataset.drop === 'D1' || width === 320
    const gap = sidebar.dataset.gap === 'G1' ? 8 : sidebar.dataset.gap === 'G2' ? 12 : 16
    expect(Math.round(sidebar.getBoundingClientRect().width)).toBe(width)
    const rows = sidebar.querySelectorAll<HTMLElement>('[data-priority-row]')
    expect(rows.length).toBe(accounts.length)
    const bare = sidebar.dataset.bar === 'B2' && width !== 320
    expect(sidebar.querySelectorAll('[data-reading] [data-usage-meter-bar]').length === 0).toBe(bare)
    expect(sidebar.querySelectorAll('[data-quota-table] [title]:not([data-usage-meter-tone] [title])').length).toBe(0)
    const firstColumns = Array.from(rows[0]!.children).map(cell => cell.getBoundingClientRect().left)
    for (const row of rows) {
      expect(Math.round(row.getBoundingClientRect().height)).toBe(22)
      const priorities = Array.from(row.querySelectorAll<HTMLElement>('[data-priority]')).map(cell => Number(cell.dataset.priority))
      expect(priorities).toEqual(full ? [1, 2, 3, 4, 5] : [1, 2, 3])
      const cells = Array.from(row.children) as HTMLElement[]
      cells.forEach((cell, index) => {
        const rect = cell.getBoundingClientRect()
        expect(Math.abs(rect.left - firstColumns[index]!)).toBeLessThan(1)
        expect(rect.width).toBeGreaterThan(0)
        expect(rect.height).toBeLessThanOrEqual(22)
        expect(getComputedStyle(cell).whiteSpace).toBe('nowrap')
        if (index > 0) expect(rect.left - cells[index - 1]!.getBoundingClientRect().right).toBeGreaterThanOrEqual(gap - .1)
        for (const run of cell.querySelectorAll('[data-line-run]')) {
          const range = document.createRange(); range.selectNodeContents(run)
          const rects = Array.from(range.getClientRects())
          expect(rects.length).toBeGreaterThan(0)
          expect(new Set(rects.map(rect => Math.round(rect.top))).size).toBe(1)
        }
      })
      const reset = row.querySelector<HTMLElement>('[data-reset]')!
      expect(reset.scrollWidth).toBeLessThanOrEqual(reset.clientWidth)
      expect(reset.textContent).not.toContain('Unrep')
      const overflow = row.querySelector('[data-overflow-count]')
      if (overflow) expect(overflow.getAttribute('data-overflow-count')).toBe(full ? '1' : '2')
      for (const reading of row.querySelectorAll<HTMLElement>('[data-reading]')) expect(reading.scrollWidth).toBeLessThanOrEqual(reading.clientWidth)
      expect(row.scrollWidth).toBeLessThanOrEqual(Math.ceil(row.getBoundingClientRect().width))
    }
    expect(sidebar.querySelector('[data-priority-row="meadow"] [data-reset]')?.getAttribute('aria-label')).toBe('Reset unreported')
    expect(sidebar.querySelector('[data-priority-row="harbor"] [data-usage-meter-tone="exhausted"]')).not.toBeNull()
    expect(sidebar.textContent).toContain('gh/fieldworks-ventures')
    expect(sidebar.querySelector('[data-priority-row="quarry"]')?.hasAttribute('data-stale')).toBe(true)
    expect(sidebar.querySelector('[data-priority-row="slate"] [data-usage-meter-tone="unknown"]')).not.toBeNull()
    expect(sidebar.querySelectorAll('[data-column-headers]').length).toBe(sidebar.dataset.tableForm === 'T2' ? 1 : 0)
  }
}
export const QuotaPriorityProbe: Story = { args: { probe: true }, play: async ({ canvasElement, step }) => {
  const canvas = within(canvasElement)
  for (const table of tables) for (const gap of gaps) for (const width of widths) for (const bar of bars) await step(`${table.id}/${gap.id}/${width.id}/${bar.id}: aligned priority table after real controls`, async () => {
    await userEvent.click(canvas.getByRole('button', { name: `T ${table.id}` }))
    await userEvent.click(canvas.getByRole('button', { name: `G ${gap.id}` }))
    await userEvent.click(canvas.getByRole('button', { name: `W ${width.id}` }))
    await userEvent.click(canvas.getByRole('button', { name: `B ${bar.id}` }))
    if (canvas.getByRole('button', { name: 'Compare policies' }).getAttribute('aria-pressed') !== 'true') await userEvent.click(canvas.getByRole('button', { name: 'Compare policies' }))
    await waitFor(() => { expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(3); expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-sidebar')).toBe(String(width.id)); expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-gap')).toBe(gap.id); expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-table-form')).toBe(table.id); expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-bar')).toBe(bar.id) })
    assertTables(canvasElement)
  })
  await step('Single policy, persistent reset and secondary-only disclosure', async () => {
    await userEvent.click(canvas.getByRole('button', { name: 'Compare policies' }))
    await userEvent.click(canvas.getByRole('button', { name: 'W 200' }))
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(1)); assertTables(canvasElement)
    for (const bar of [...bars].reverse()) for (const drop of drops) {
      await userEvent.click(canvas.getByRole('button', { name: `B ${bar.id}` }))
      await userEvent.click(canvas.getByRole('button', { name: `D ${drop.id}` }))
      await waitFor(() => { expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-drop')).toBe(drop.id); expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-bar')).toBe(bar.id) })
      assertTables(canvasElement)
    }
    const north = canvas.getByRole('button', { name: 'Account North studio' })
    expect(north).toHaveAccessibleDescription('gh/north-studio')
    await userEvent.keyboard('{Shift}'); north.focus()
    expect(await within(document.body).findByRole('tooltip')).toHaveTextContent('gh/north-studio')
    await userEvent.keyboard('{Escape}'); await waitFor(() => expect(within(document.body).queryByRole('tooltip')).toBeNull())
    await userEvent.click(canvas.getByRole('button', { name: 'Show 2 secondary windows for North studio' }))
    const dialog = await within(document.body).findByRole('dialog', { name: 'Secondary windows for North studio' })
    expect(within(dialog).getByText('41%')).toBeVisible(); expect(within(dialog).getByText('18%')).toBeVisible()
    expect(dialog.textContent).not.toContain('reset')
    await userEvent.click(within(dialog).getByRole('button', { name: 'Close readings' }))
    await waitFor(() => expect(dialog).not.toBeInTheDocument()); assertTables(canvasElement)
    const primary = canvasElement.querySelector<HTMLElement>('[data-priority="2"]')!
    const parent = primary.parentElement!; const next = primary.nextSibling
    primary.remove(); expect(() => assertTables(canvasElement)).toThrow(); parent.insertBefore(primary, next)
    const reset = canvasElement.querySelector<HTMLElement>('[data-reset]')!
    reset.style.whiteSpace = 'normal'; expect(() => assertTables(canvasElement)).toThrow(); reset.style.removeProperty('white-space'); assertTables(canvasElement)
    const overflow = canvasElement.querySelector<HTMLElement>('[data-overflow-count]')!
    overflow.setAttribute('data-overflow-count', '3'); expect(() => assertTables(canvasElement)).toThrow(); overflow.setAttribute('data-overflow-count', '2')
    const table = canvasElement.querySelector<HTMLElement>('[data-quota-table]')!
    table.style.columnGap = '0'; expect(() => assertTables(canvasElement)).toThrow(); table.style.removeProperty('column-gap'); assertTables(canvasElement)
    const exhausted = canvasElement.querySelector<HTMLElement>('[data-priority-row="harbor"] [data-reading] [data-usage-meter-tone="exhausted"]')!
    exhausted.style.paddingInlineStart = '24px'; expect(() => assertTables(canvasElement)).toThrow(); exhausted.style.removeProperty('padding-inline-start'); assertTables(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /Account Meadow research/ }))
    expect(await canvas.findByRole('region', { name: 'Meadow research usage details' })).toBeVisible()
    expect(canvas.getAllByText('Unreported').length).toBeGreaterThan(0)
    await userEvent.click(canvas.getByRole('button', { name: 'Close usage details' }))
    await waitFor(() => expect(canvas.queryByRole('region', { name: 'Meadow research usage details' })).not.toBeInTheDocument()); assertTables(canvasElement)
    const scheme = canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme')
    await userEvent.click(canvas.getByRole('button', { name: 'Light scheme' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme')).not.toBe(scheme)); assertTables(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: 'Host 1440' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-host')).toBe('900')); assertTables(canvasElement)
  })
} }

const styles = stylex.create({
  root: { padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  heading: { margin: 0, paddingBlock: s.md, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  copy: { color: ink.fgSoft },
  picker: { display: 'flex', alignItems: 'center', gap: s.md, flexWrap: 'wrap', paddingBlock: s.sm },
  pickerLabel: { width: s.section, fontWeight: t.weightMedium },
  button: { color: ink.fgSoft, font: 'inherit', backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover }, border: quotaGeometry.hair, padding: s.xs, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline } },
  selected: { color: ink.fg, backgroundColor: surface.rowActive, border: quotaGeometry.strongHair },
  host: { paddingBlock: s.section, maxWidth: '100%', overflowX: 'auto' }, host1440: { width: g.host1440 }, host900: { width: g.host900 },
  previews: { display: 'flex', gap: s.section, alignItems: 'flex-start' },
  sidebar: { flexShrink: 0, backgroundColor: surface.raised, color: ink.fg, boxSizing: 'border-box' },
  width200: { width: g.sidebar200 }, width240: { width: g.sidebar240 }, width320: { width: g.sidebar320 },
  table: { display: 'grid', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: usageMeterGridVars.lineHeight, fontVariantNumeric: 'tabular-nums' },
  gap8: { columnGap: s.md }, gap12: { columnGap: s.lg }, gap16: { columnGap: s.xl },
  truncate: { gridTemplateColumns: g.truncateTracks }, full: { gridTemplateColumns: g.fullTracks }, collapsed: { gridTemplateColumns: g.collapsedTracks }, hidden: { gridTemplateColumns: g.hiddenTracks },
  truncateBare: { gridTemplateColumns: g.truncateBareTracks }, collapsedBare: { gridTemplateColumns: g.collapsedBareTracks }, hiddenBare: { gridTemplateColumns: g.hiddenBareTracks },
  row: { display: 'grid', gridTemplateColumns: 'subgrid', gridColumn: '1 / -1', alignItems: 'center', height: usageMeterGridVars.rowHeight, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover } },
  header: { color: ink.fgMuted },
  cell: { minWidth: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxHeight: usageMeterGridVars.rowHeight, textAlign: 'end' },
  accountCell: { textAlign: 'start' },
  account: { display: 'block', width: '100%', color: ink.fg, font: 'inherit', textAlign: 'start', padding: 0, border: 0, backgroundColor: surface.transparent, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline } },
  reset: { color: ink.fgMuted, overflow: 'visible', textOverflow: 'clip' },
  more: { color: ink.fgSoft, font: 'inherit', fontVariantNumeric: 'tabular-nums', textAlign: 'end', width: '100%', whiteSpace: 'nowrap', padding: 0, border: 0, backgroundColor: surface.transparent, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline } },
  popover: { backgroundColor: surface.raised, color: ink.fg, border: quotaGeometry.strongHair },
  dialog: { padding: s.lg, color: ink.fg, backgroundColor: surface.raised, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, outline: 'none' },
})
