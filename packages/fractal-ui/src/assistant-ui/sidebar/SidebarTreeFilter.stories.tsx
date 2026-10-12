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
/** Samples per input per phase; p90 is the 27th of 30, so three load spikes cannot set it alone. */
const sampleCount = 30
/** Asynchronous settling on a contended CI host; the samples themselves carry the measurements. */
const settled = { timeout: 10_000 }
/** Filter keystroke-to-next-frame p90 may be at most this multiple of the reference keystroke p90. */
const ratioBudget = 3
/** The synchronous regression control: each filter keystroke busy-waits this multiple of its paired reference keystroke. */
const controlRatio = 4
/** Renders are attributed to a keystroke through this many frames after its first post-commit frame. */
const settleFrames = 2

interface RenderCall { readonly key: string; readonly pinned: boolean; readonly retained: boolean }
interface Sample {
  readonly durationMs: number
  readonly value: string
  readonly count: number
  readonly retainedIdentity: boolean
  readonly paintedKeys: readonly string[]
  /** Painted rows whose keyed DOM row is new for this keystroke: newly mounted, or a reused RAC view rebound to the key. */
  readonly changedKeys: readonly string[]
  readonly renders?: readonly RenderCall[]
}
interface PendingInput { readonly start: number; readonly value: string; readonly rowsBefore: ReadonlyMap<string, Element> }

const paintedRows = (element: HTMLElement) => new Map([...element.querySelectorAll<HTMLElement>('[data-sidebar-tree-key]')].map(row => [row.dataset.sidebarTreeKey!, row] as const))
const nextFrame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()))

/**
 * Times input capture through the first animation frame after React commits (the
 * browser's next-paint boundary, not a physical display-paint claim), then attaches
 * every renderRow call made until `settleFrames` later, so deferred work is counted.
 */
function useKeystrokeSample(value: string, pending: React.RefObject<PendingInput | undefined>, record: (input: PendingInput, durationMs: number) => Sample, settle: (sample: Sample) => void) {
  const recordEvent = React.useEffectEvent(record)
  const settleEvent = React.useEffectEvent(settle)
  React.useLayoutEffect(() => {
    const input = pending.current
    if (input === undefined || input.value !== value) return
    let cancelled = false
    const frame = requestAnimationFrame(async () => {
      // Use now(), not the frame timestamp: the latter can precede a costly input handler.
      const durationMs = performance.now() - input.start
      pending.current = undefined
      const sample = recordEvent(input, durationMs)
      for (let index = 0; index < settleFrames; index++) await nextFrame()
      if (!cancelled) settleEvent(sample)
    })
    return () => { cancelled = true; cancelAnimationFrame(frame) }
  }, [value, pending])
}

function FilterProbe({ light, narrow }: { readonly light: boolean; readonly narrow: boolean }) {
  const id = `${light ? 'light' : 'dark'}-${narrow ? '240' : '440'}`
  const [query, setQuery] = React.useState('')
  const [reference, setReference] = React.useState('')
  const root = React.useRef<HTMLElement>(null)
  const pendingFilter = React.useRef<PendingInput | undefined>(undefined)
  const pendingReference = React.useRef<PendingInput | undefined>(undefined)
  const renders = React.useRef<RenderCall[]>([])
  const filterSamples = React.useRef<Sample[]>([])
  const referenceSamples = React.useRef<Sample[]>([])
  const items = React.useMemo(() => {
    const filtered = query === '' ? fixture : fixture.filter(item => item.textValue.toLowerCase().includes(query.toLowerCase()))
    // Rebuild control: fresh item objects on every keystroke, as a host that rebuilds
    // its projection would hand RAC. The work and identity checks must reject it.
    return root.current?.dataset.rebuildControl === 'true' ? filtered.map(item => ({ ...item })) : filtered
  }, [query])
  const renderRow = React.useCallback((item: Agent, state: { readonly pinned: boolean }) => {
    renders.current.push({ key: String(item.id), pinned: state.pinned, retained: fixture.includes(item) })
    return <div {...stylex.props(styles.row)}>{item.textValue}</div>
  }, [])
  const capture = (pending: React.RefObject<PendingInput | undefined>) => (event: React.FormEvent<HTMLInputElement>) => {
    // Start at input capture, BEFORE onChange's synchronous work and React's commit.
    renders.current = []
    pending.current = { start: performance.now(), value: event.currentTarget.value, rowsBefore: root.current === null ? new Map() : paintedRows(root.current) }
  }
  const publish = () => {
    const element = root.current!
    element.dataset.filterSamples = JSON.stringify(filterSamples.current)
    element.dataset.referenceSamples = JSON.stringify(referenceSamples.current)
  }
  const recordInto = (samples: React.RefObject<Sample[]>, count: number) => (input: PendingInput, durationMs: number) => {
    const rows = paintedRows(root.current!)
    const changedKeys = [...rows].filter(([key, row]) => input.rowsBefore.get(key) !== row).map(([key]) => key)
    const sample: Sample = { durationMs, value: input.value, count, retainedIdentity: items.every(item => fixture.includes(item)), paintedKeys: [...rows.keys()], changedKeys }
    samples.current.push(sample)
    return sample
  }
  const settleInto = (samples: React.RefObject<Sample[]>) => (sample: Sample) => {
    const index = samples.current.indexOf(sample)
    if (index < 0) return
    samples.current[index] = { ...sample, renders: [...renders.current] }
    publish()
  }
  useKeystrokeSample(query, pendingFilter, recordInto(filterSamples, items.length), settleInto(filterSamples))
  useKeystrokeSample(reference, pendingReference, recordInto(referenceSamples, items.length), settleInto(referenceSamples))
  const changeQuery = (event: React.ChangeEvent<HTMLInputElement>) => {
    const cost = Number(root.current?.dataset.synchronousCostMs ?? 0)
    if (cost > 0) {
      const until = performance.now() + cost
      while (performance.now() < until) { /* Deliberate synchronous regression control. */ }
    }
    setQuery(event.currentTarget.value)
  }
  return <section ref={root} data-testid={`filter-probe-${id}`} data-synchronous-cost-ms="0" data-rebuild-control="false" data-visible-count={items.length} {...stylex.props(styles.probe, light && lightTheme, narrow ? styles.narrow : styles.wide)}>
    <div {...stylex.props(styles.toolbar)}>
      <span>{light ? 'Light' : 'Dark'} · {narrow ? '240' : '440'} · 78 agents</span>
      <input aria-label={`Filter agents ${id}`} type="search" value={query} onInputCapture={capture(pendingFilter)} onChange={changeQuery} {...stylex.props(styles.input)} />
      <input aria-label={`Reference keystroke ${id}`} type="text" value={reference} onInputCapture={capture(pendingReference)} onChange={event => setReference(event.currentTarget.value)} {...stylex.props(styles.input)} />
    </div>
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
    docs: { description: { component: 'Production SidebarTree filter probe with 78 identity-retained synthetic agents. Primary, deterministic: each keystroke may render only painted rows whose keyed DOM row changed, with no churn of item identity; a fresh-object rebuild control must reject. Secondary, calibrated: the filter keystroke-to-next-frame p90 must stay within 3x the p90 of a trivial reference keystroke in the same page, the two alternating keystroke by keystroke and pooled over the phases before and after the controls; a busy-wait control making each filter keystroke wait 4x its paired reference keystroke must reject. This kit-only probe does not measure application agent-search latency.' } },
  },
} satisfies Meta<typeof SidebarTree>
export default meta
type Story = StoryObj<typeof meta>

const read = (root: HTMLElement, key: 'filterSamples' | 'referenceSamples'): readonly Sample[] => JSON.parse(root.dataset[key] ?? '[]') as Sample[]
const p90 = (samples: readonly Sample[]) => [...samples].map(sample => sample.durationMs).sort((a, b) => a - b)[Math.ceil(samples.length * 0.9) - 1]!

type Inputs = { readonly reference: HTMLInputElement; readonly filter: HTMLInputElement }

// One keystroke per sample, each waiting until the previous sample has settled. Reference
// and filter keystrokes alternate so both distributions see the same host-load windows.
// With `busyControl`, each filter keystroke busy-waits `controlRatio` times the reference
// keystroke just before it, so every filter sample is at least that multiple of its pair
// and the filter p90 is at least `controlRatio` times the reference p90, whatever the load.
async function collect(root: HTMLElement, inputs: Inputs, busyControl = false) {
  const before = { reference: read(root, 'referenceSamples').length, filter: read(root, 'filterSamples').length }
  await expect(inputs.reference.value).toBe('')
  await expect(inputs.filter.value).toBe('')
  for (let index = 0; index < sampleCount; index++) {
    const toggle = index % 2 === 0
    await userEvent.type(inputs.reference, toggle ? 'r' : '{Backspace}')
    await waitFor(() => expect(read(root, 'referenceSamples')[before.reference + index]?.renders).toBeDefined(), settled)
    if (busyControl) root.dataset.synchronousCostMs = String(controlRatio * read(root, 'referenceSamples')[before.reference + index]!.durationMs)
    await userEvent.type(inputs.filter, toggle ? 'x' : '{Backspace}')
    await waitFor(() => expect(read(root, 'filterSamples')[before.filter + index]?.renders).toBeDefined(), settled)
  }
  await expect(inputs.reference.value).toBe('')
  await expect(inputs.filter.value).toBe('')
  return { reference: read(root, 'referenceSamples').slice(before.reference), samples: read(root, 'filterSamples').slice(before.filter) }
}

// Primary check. Every keystroke renders only painted rows whose keyed DOM row changed
// (newly mounted or a rebound RAC view), and every rendered item is the retained fixture object.
async function assertWork(samples: readonly Sample[]) {
  await expect(samples).toHaveLength(sampleCount)
  for (const [index, sample] of samples.entries()) {
    const filtered = index % 2 === 0
    await expect(sample.value).toBe(filtered ? 'x' : '')
    await expect(sample.count).toBe(filtered ? 26 : 78)
    await expect(sample.paintedKeys[0]).toBe(filtered ? 'filter-agent-2' : 'filter-agent-1')
    await expect(sample.retainedIdentity).toBe(true)
    const renders = sample.renders!
    await expect(renders.every(call => call.retained)).toBe(true)
    for (const call of renders) await expect(sample.changedKeys).toContain(call.key)
    await expect(renders.length).toBeLessThanOrEqual(sample.changedKeys.length)
  }
}

// Secondary check, calibrated against reference keystrokes interleaved with the filter ones.
async function assertRatio(filter: readonly Sample[], reference: readonly Sample[]) {
  await expect(filter.length).toBeGreaterThanOrEqual(sampleCount)
  await expect(reference).toHaveLength(filter.length)
  await expect(p90(filter) / p90(reference)).toBeLessThanOrEqual(ratioBudget)
}

export const FilterKeystrokeToPaint: Story = {
  render: () => <main {...stylex.props(styles.matrix)}>{[false, true].flatMap(light => [true, false].map(narrow => <FilterProbe key={`${light}-${narrow}`} light={light} narrow={narrow} />))}</main>,
  play: async ({ canvasElement }) => {
    await document.fonts.ready
    // Allow initial virtualizer measurements to settle outside all samples.
    for (let index = 0; index < 6; index++) await nextFrame()
    for (const root of canvasElement.querySelectorAll<HTMLElement>('[data-testid^="filter-probe-"]')) {
      const inputs: Inputs = { filter: within(root).getByRole('searchbox') as HTMLInputElement, reference: within(root).getByRole('textbox') as HTMLInputElement }
      await expect(root.dataset.visibleCount).toBe('78')
      const phase = async (name: string, busyControl = false) => {
        const sampled = await collect(root, inputs, busyControl)
        root.dataset[`${name}Ratio`] = String(p90(sampled.samples) / p90(sampled.reference))
        return sampled
      }
      const production = await phase('production')
      await assertWork(production.samples)
      try {
        root.dataset.rebuildControl = 'true'
        const rebuild = await phase('rebuild')
        await expect(assertWork(rebuild.samples)).rejects.toThrow()
      } finally {
        root.dataset.rebuildControl = 'false'
      }
      try {
        const busy = await phase('busy', true)
        await expect(assertRatio(busy.samples, busy.reference)).rejects.toThrow()
      } finally {
        root.dataset.synchronousCostMs = '0'
      }
      const restored = await phase('restored')
      await assertWork(restored.samples)
      // The ratio pools both unregressed phases (60 pairs, p90 is the 54th). A control
      // left armed would make half the pool slow and so set the p90.
      const filter = [...production.samples, ...restored.samples]
      const reference = [...production.reference, ...restored.reference]
      root.dataset.pooledRatio = String(p90(filter) / p90(reference))
      await assertRatio(filter, reference)
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
