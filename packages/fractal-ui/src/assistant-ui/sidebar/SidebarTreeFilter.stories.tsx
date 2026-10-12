import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { Key } from 'react-aria-components'
import { SidebarTree, type SidebarTreeItem } from './SidebarTree'
import { lightTheme } from '../composition-theme'
import { borderVars as border, geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { sidebarTreeStoryVars as sizes } from './sidebar-tree-story-tokens.stylex'

interface Agent extends SidebarTreeItem<Agent> {}
// Original synthetic agents, not application data. Filter only the array: every
// retained object is the production fixture object RAC previously received.
const fixture: readonly Agent[] = Array.from({ length: 78 }, (_, index) => ({
  id: `filter-agent-${index + 1}`,
  depth: 0,
  textValue: `${['Brindle scout', 'Aster flux', 'Cobalt surveyor'][index % 3]} ${String(Math.floor(index / 3) + 1).padStart(2, '0')}`,
}))
const emptyExpanded = new Set<Key>()
const onExpandedChange = (_keys: Set<Key>) => {}
const onRowContextMenu = (_item: Agent, event: React.MouseEvent) => event.preventDefault()
const renderRow = (item: Agent) => <div {...stylex.props(styles.row)}>{item.textValue}</div>
const budgetMs = 50
const controlCostMs = 120
const sampleCount = 10
interface Sample {
  readonly durationMs: number
  readonly query: string
  readonly count: number
  readonly retainedIdentity: boolean
  readonly paintedKeys: readonly string[]
}
interface PendingInput { readonly start: number; readonly query: string }

function FilterProbe({ light, narrow }: { readonly light: boolean; readonly narrow: boolean }) {
  const id = `${light ? 'light' : 'dark'}-${narrow ? '240' : '440'}`
  const [query, setQuery] = React.useState('')
  const root = React.useRef<HTMLElement>(null)
  const pending = React.useRef<PendingInput | undefined>(undefined)
  const samples = React.useRef<Sample[]>([])
  const items = React.useMemo(() => query === '' ? fixture : fixture.filter(item => item.textValue.toLowerCase().includes(query.toLowerCase())), [query])
  const captureInput = React.useCallback((event: React.FormEvent<HTMLInputElement>) => {
    // Start at input capture, BEFORE onChange's synchronous work and React's
    // commit. A bracket begun in rAF would incorrectly hide the 120ms control.
    pending.current = { start: performance.now(), query: event.currentTarget.value }
  }, [])
  const changeQuery = React.useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    const cost = Number(root.current?.dataset.synchronousCostMs ?? 0)
    if (cost > 0) {
      const until = performance.now() + cost
      while (performance.now() < until) { /* Deliberate synchronous regression control. */ }
    }
    setQuery(event.currentTarget.value)
  }, [])
  React.useLayoutEffect(() => {
    const input = pending.current
    if (input === undefined || input.query !== query) return
    const frame = requestAnimationFrame(() => {
      const element = root.current
      if (element === null) return
      // Use now(), not the frame timestamp: the latter can precede the costly
      // input handler. This is the first animation frame after React's commit,
      // the browser's next-paint boundary (not a physical display-paint claim).
      const durationMs = performance.now() - input.start
      samples.current.push({
        durationMs,
        query,
        count: items.length,
        retainedIdentity: items.every(item => fixture.includes(item)),
        paintedKeys: [...element.querySelectorAll<HTMLElement>('[data-sidebar-tree-key]')].map(row => row.dataset.sidebarTreeKey!),
      })
      pending.current = undefined
      element.dataset.filterSamples = JSON.stringify(samples.current)
      element.dataset.sampleCount = String(samples.current.length)
    })
    return () => cancelAnimationFrame(frame)
  }, [query, items])
  return <section ref={root} data-testid={`filter-probe-${id}`} data-synchronous-cost-ms="0" data-visible-count={items.length} {...stylex.props(styles.probe, light && lightTheme, narrow ? styles.narrow : styles.wide)}>
    <label {...stylex.props(styles.toolbar)}>
      <span>{light ? 'Light' : 'Dark'} · {narrow ? '240' : '440'} · 78 agents</span>
      <input data-testid={`filter-input-${id}`} aria-label={`Filter agents ${id}`} type="search" value={query} onInputCapture={captureInput} onChange={changeQuery} {...stylex.props(styles.input)} />
    </label>
    <div {...stylex.props(styles.viewport)}>
      <SidebarTree aria-label={`Filtered agents ${id}`} items={items} expandedKeys={emptyExpanded} onExpandedChange={onExpandedChange} renderRow={renderRow} onRowContextMenu={onRowContextMenu} />
    </div>
  </section>
}

const meta = {
  title: 'Fractal UI/Sidebar tree',
  component: SidebarTree,
  parameters: {
    layout: 'fullscreen',
    docs: { description: { component: 'Production SidebarTree filter probe with 78 identity-retained synthetic agents. Measures input-capture through the first animation frame after React commits. Each composition checks the 50ms p90 budget, proves that a synchronous 120ms regression rejects that same budget, then restores and checks the production path again. This kit-only probe does not measure application agent-search latency.' } },
  },
} satisfies Meta<typeof SidebarTree>
export default meta
type Story = StoryObj<typeof meta>

const readSamples = (root: HTMLElement): readonly Sample[] => JSON.parse(root.dataset.filterSamples ?? '[]') as Sample[]
const p90 = (samples: readonly Sample[]) => [...samples].map(sample => sample.durationMs).sort((a, b) => a - b)[Math.ceil(samples.length * 0.9) - 1]!
// Both paths call the exact same budget check; the negative control must reject.
const assertBudget = async (samples: readonly Sample[]) => {
  await expect(samples).toHaveLength(sampleCount)
  await expect(p90(samples)).toBeLessThan(budgetMs)
}
async function collectSamples(root: HTMLElement, input: HTMLInputElement) {
  const before = readSamples(root).length
  await expect(input.value).toBe('')
  for (let index = 0; index < sampleCount; index++) {
    const filtered = index % 2 === 0
    await userEvent.type(input, filtered ? 'x' : '{Backspace}')
    // Exactly one keystroke per sample, and await its commit/frame before the
    // next input. No userEvent delay or polling time is included in the metric.
    await waitFor(() => expect(readSamples(root)).toHaveLength(before + index + 1))
    const sample = readSamples(root)[before + index]!
    await expect(sample.query).toBe(filtered ? 'x' : '')
    await expect(sample.count).toBe(filtered ? 26 : 78)
    await expect(root.dataset.visibleCount).toBe(String(sample.count))
    await expect(sample.retainedIdentity).toBe(true)
    await expect(sample.paintedKeys.length).toBeGreaterThan(0)
    const allowed = fixture.filter(item => !filtered || item.textValue.includes('flux')).map(item => String(item.id))
    for (const key of sample.paintedKeys) await expect(allowed).toContain(key)
    // Prove the actual RAC collection changed, not just the count/readout.
    await expect(sample.paintedKeys[0]).toBe(filtered ? 'filter-agent-2' : 'filter-agent-1')
    await expect(within(root).getByRole('treegrid')).toBeVisible()
  }
  return readSamples(root).slice(before)
}

export const FilterKeystrokeToPaint: Story = {
  render: () => <main {...stylex.props(styles.matrix)}>{[false, true].flatMap(light => [true, false].map(narrow => <FilterProbe key={`${light}-${narrow}`} light={light} narrow={narrow} />))}</main>,
  play: async ({ canvasElement }) => {
    await document.fonts.ready
    // Allow initial virtualizer measurements to settle outside all samples.
    for (let index = 0; index < 6; index++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
    for (const root of canvasElement.querySelectorAll<HTMLElement>('[data-testid^="filter-probe-"]')) {
      const input = within(root).getByRole('searchbox') as HTMLInputElement
      await expect(root.dataset.visibleCount).toBe('78')
      const production = await collectSamples(root, input)
      root.dataset.productionP90Ms = String(p90(production))
      await assertBudget(production)
      try {
        root.dataset.synchronousCostMs = String(controlCostMs)
        const control = await collectSamples(root, input)
        root.dataset.controlP90Ms = String(p90(control))
        await expect(p90(control)).toBeGreaterThan(controlCostMs)
        await expect(assertBudget(control)).rejects.toThrow()
      } finally {
        root.dataset.synchronousCostMs = '0'
      }
      const restored = await collectSamples(root, input)
      root.dataset.restoredP90Ms = String(p90(restored))
      await assertBudget(restored)
    }
  },
}

const styles = stylex.create({
  matrix: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.xl, padding: s.xl, backgroundColor: surface.canvas },
  probe: { minWidth: 0, color: ink.fg, backgroundColor: surface.sidebar, fontFamily: t.fontSans, fontSize: t.metaSize },
  narrow: { width: sizes.narrow },
  wide: { width: sizes.small },
  toolbar: { display: 'flex', flexDirection: 'column', gap: s.sm, padding: s.md },
  input: { boxSizing: 'border-box', width: '100%', minWidth: 0, height: g.controlLg, paddingInline: s.md, color: ink.fg, backgroundColor: surface.controlFill, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, fontFamily: t.fontSans, fontSize: t.uiSize, outlineColor: ink.fgMuted },
  viewport: { height: sizes.viewport, minHeight: 0 },
  row: { display: 'flex', alignItems: 'center', height: sizes.row, paddingInline: s.sm, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', minWidth: 0 },
})
