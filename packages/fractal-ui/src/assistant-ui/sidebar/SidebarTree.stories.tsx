import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { Key } from 'react-aria-components'
import { SidebarTree, type SidebarTreeHandle, type SidebarTreeItem } from './SidebarTree'
import { lightTheme } from '../composition-theme'
import { spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

interface Item extends SidebarTreeItem<Item> { readonly children: readonly Item[] }
const makeBranch = (depth: number, prefix: string): Item => ({ id: prefix, textValue: `Folder ${prefix}`, depth, children: depth < 6 ? [makeBranch(depth + 1, `${prefix}.a`), ...Array.from({ length: 12 }, (_, index) => ({ id: `${prefix}.${index}`, textValue: `Review ${index + 1}`, depth: depth + 1, children: [] }))] : Array.from({ length: 70 }, (_, index) => ({ id: `${prefix}.${index}`, textValue: `Review ${index + 1}`, depth: depth + 1, children: [] })) })
const fixture: readonly Item[] = [makeBranch(0, 'north'), makeBranch(0, 'south')]
const deepKey = 'north.a.a.a.a.a.a.35'
const index = new Map<Key, Item>()
const parents = new Map<Key, readonly Key[]>()
function visit(items: readonly Item[], chain: readonly Key[] = []) { for (const item of items) { index.set(item.id, item); parents.set(item.id, chain); visit(item.children, [...chain, item.id]) } }
visit(fixture)
const expanded = new Set([...index.values()].filter(item => item.children.length).map(item => item.id))
const sizes = stylex.defineVars({ narrow: '240px', small: '440px', wide: '1440px', viewport: '360px', row: '24px', resizedRow: '32px' })
function Example({ sticky = true, cold = false, width = 440, light = false, noSettle = false }: { readonly sticky?: boolean; readonly cold?: boolean; readonly width?: number; readonly light?: boolean; readonly noSettle?: boolean }) {
  const [keys, setKeys] = React.useState<Set<Key>>(() => cold ? new Set() : new Set(expanded))
  const [filtered, setFiltered] = React.useState(false)
  const [reverse, setReverse] = React.useState(false)
  const [resized, setResized] = React.useState(false)
  const handle = React.useRef<SidebarTreeHandle>(null)
  const root = React.useRef<HTMLDivElement>(null)
  const filter = (items: readonly Item[]): readonly Item[] => items.filter(item => item.children.length > 0 || Number(String(item.id).split('.').at(-1)) % 2 === 1).map(item => ({ ...item, children: filter(item.children) }))
  const items = filtered ? filter(fixture) : reverse ? [...fixture].reverse() : fixture
  React.useLayoutEffect(() => {
    if (!cold) return
    if (noSettle) { setKeys(new Set(expanded)); const tree = root.current?.querySelector('[role="treegrid"]'); if (tree instanceof HTMLElement) tree.scrollTop = 0 }
    else handle.current?.reveal(deepKey)
    const frame = requestAnimationFrame(() => setResized(true))
    return () => cancelAnimationFrame(frame)
  }, [cold, noSettle])
  return <section data-example={sticky ? noSettle ? 'unsettled' : 'sticky' : 'control'} ref={root} {...stylex.props(styles.example, light && lightTheme, width === 240 ? styles.narrow : width === 1440 ? styles.wide : styles.small)}>
    <div {...stylex.props(styles.toolbar)}><button onClick={() => handle.current?.reveal(deepKey)}>Reveal review</button><button onClick={() => setFiltered(value => !value)}>Filter reviews</button><button onClick={() => setKeys(new Set())}>Collapse all</button><button onClick={() => setReverse(value => !value)}>Reorder</button></div>
    <div {...stylex.props(styles.viewport)}><SidebarTree ref={handle} aria-label="Project reviews" items={items} expandedKeys={keys} onExpandedChange={setKeys} stickyAncestors={sticky} renderRow={item => <div {...stylex.props(styles.label, resized && styles.resized)}>{item.textValue}</div>} /></div>
  </section>
}
function Pair({ cold = false, width = 440, light = false }: { readonly cold?: boolean; readonly width?: number; readonly light?: boolean }) { return <main {...stylex.props(styles.matrix)}><Example cold={cold} width={width} light={light} /><Example sticky={!cold} noSettle={cold} cold={cold} width={width} light={light} /></main> }
const meta = { title: 'Fractal UI/Sidebar tree', component: SidebarTree, parameters: { layout: 'fullscreen', docs: { description: { component: 'A measured RAC treegrid. Sticky copies are hidden from accessibility and inert; every regression includes a production-component comparison with stickiness disabled. RAC represents tree items as treegrid rows.' } } } } satisfies Meta<typeof SidebarTree>
export default meta
type Story = StoryObj<typeof meta>
const frames = async () => { await document.fonts.ready; for (let i = 0; i < 6; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve())) }
const examples = (canvas: HTMLElement) => [canvas.querySelector('[data-example="sticky"]')!, canvas.querySelector('[data-example="control"]')!] as const
const viewport = (example: Element) => example.querySelector('[role="treegrid"]') as HTMLElement
const stack = (example: Element) => example.querySelector('[data-sidebar-pinned-stack]') as HTMLElement
const real = (example: Element, key: string) => example.querySelector(`[data-sidebar-tree-key="${key}"]`) as HTMLElement | null
const clear = (example: Element, row: HTMLElement) => { const bounds = row.getBoundingClientRect(); const view = viewport(example).getBoundingClientRect(); return bounds.top >= view.top + stack(example).getBoundingClientRect().height - 1 && bounds.bottom <= view.bottom + 1 }
async function scroll(example: Element, offset = 600) { viewport(example).scrollTop = offset; viewport(example).dispatchEvent(new Event('scroll', { bubbles: true })); await frames() }
async function assertChain(example: Element) {
  const top = stack(example).dataset.topKey!
  const actual = [...example.querySelectorAll('[data-pinned-ancestors]')].flatMap(row => row.getAttribute('data-pinned-ancestors')!.split(','))
  await expect(actual).toEqual(parents.get(top))
}
const pair = () => <Pair />
export const StickyChain: Story = { render: pair, play: async ({ canvasElement }) => { const [yes, no] = examples(canvasElement); await frames(); await scroll(yes); await scroll(no); await assertChain(yes); await expect(no.querySelectorAll('[data-pinned-key]').length).toBe(0); await expect(parents.get(stack(no).dataset.topKey!)!.length).toBeGreaterThan(0) } }
export const PushOutBoundary: Story = { render: pair, play: async ({ canvasElement }) => {
  const [yes, no] = examples(canvasElement); await frames()
  // Sweep the first deep-folder boundary across the pinned stack, not just one endpoint.
  for (const offset of [1770, 1782, 1794, 1806, 1818]) { await scroll(yes, offset); const pins = [...yes.querySelectorAll('[data-pinned-key]')]; for (let i = 1; i < pins.length; i++) await expect(pins[i]!.getBoundingClientRect().top).toBeGreaterThanOrEqual(pins[i - 1]!.getBoundingClientRect().bottom - 1) }
  await scroll(no); await expect(no.querySelectorAll('[data-pinned-key]').length).toBe(0)
} }
export const DepthCap: Story = { render: pair, play: async ({ canvasElement }) => { const [yes, no] = examples(canvasElement); await frames(); await scroll(yes); await scroll(no); await expect(yes.querySelector('[data-sidebar-breadcrumb]')).not.toBeNull(); await expect(stack(yes).getBoundingClientRect().height).toBeLessThanOrEqual(viewport(yes).clientHeight * 0.4); await expect(yes.querySelectorAll('[data-pinned-key]').length).toBeLessThanOrEqual(4); await expect(no.querySelector('[data-sidebar-breadcrumb]')).toBeNull() } }
export const AccessibleRows: Story = { render: pair, play: async ({ canvasElement }) => { const [yes, no] = examples(canvasElement); await frames(); await scroll(yes); await scroll(no); for (const example of [yes, no]) { await expect(within(example as HTMLElement).getAllByRole('treegrid')).toHaveLength(1); await expect(within(example as HTMLElement).getAllByRole('row')).toHaveLength(example.querySelectorAll('[data-sidebar-tree-key]').length); await expect(stack(example).querySelectorAll('[role="row"]')).toHaveLength(0); await expect(stack(example).getAttribute('aria-hidden')).toBe('true') } await expect(yes.querySelectorAll('[data-pinned-key]').length).toBeGreaterThan(0); await expect(no.querySelectorAll('[data-pinned-key]').length).toBe(0) } }
export const PinnedClick: Story = { render: pair, play: async ({ canvasElement }) => { const [yes, no] = examples(canvasElement); await frames(); await scroll(yes); await scroll(no); const pin = [...yes.querySelectorAll('[data-pinned-key]')].at(-1) as HTMLElement; const key = pin.dataset.pinnedKey!; await userEvent.click(pin); await waitFor(() => expect(document.activeElement).toBe(real(yes, key))); await frames(); await expect(clear(yes, real(yes, key)!)).toBe(true); await expect(no.querySelector('[data-pinned-key]')).toBeNull() } }
export const FocusAndReveal: Story = { render: pair, play: async ({ canvasElement }) => { const [yes, no] = examples(canvasElement); await frames(); await scroll(yes); await scroll(no); const top = stack(yes).dataset.topKey!; real(yes, top)!.focus(); await frames(); await expect(clear(yes, real(yes, top)!)).toBe(true); await userEvent.keyboard('{ArrowDown}'); await frames(); await expect(clear(yes, document.activeElement as HTMLElement)).toBe(true); await userEvent.click(within(yes as HTMLElement).getByRole('button', { name: 'Reveal review' })); await waitFor(() => expect(real(yes, deepKey)).not.toBeNull()); await frames(); await expect(clear(yes, real(yes, deepKey)!)).toBe(true); await expect(no.querySelector('[data-pinned-key]')).toBeNull() } }
export const CollapseFilterReorder: Story = { render: pair, play: async ({ canvasElement }) => { const [yes, no] = examples(canvasElement); await frames(); for (const example of [yes, no]) { await scroll(example); await userEvent.click(within(example as HTMLElement).getByRole('button', { name: 'Filter reviews' })); await frames(); if (example === yes) await assertChain(yes); await userEvent.click(within(example as HTMLElement).getByRole('button', { name: 'Reorder' })); await frames(); await userEvent.click(within(example as HTMLElement).getByRole('button', { name: 'Collapse all' })); await frames(); await expect(example.querySelectorAll('[data-pinned-key]')).toHaveLength(0); await expect(within(example as HTMLElement).getAllByRole('row')).toHaveLength(2) } } }
export const ColdRevealBeforeMeasurement: Story = { render: () => <Pair cold />, play: async ({ canvasElement }) => { const yes = canvasElement.querySelector('[data-example="sticky"]')!; const no = canvasElement.querySelector('[data-example="unsettled"]')!; await waitFor(() => expect(real(yes, deepKey)).not.toBeNull()); await frames(); await expect(clear(yes, real(yes, deepKey)!)).toBe(true); await frames(); await expect(clear(yes, real(yes, deepKey)!)).toBe(true); const control = real(no, deepKey); await expect(control === null || !clear(no, control)).toBe(true) } }
export const Light440: Story = { render: () => <Pair light /> }
export const Dark1440: Story = { render: () => <Pair width={1440} /> }
export const Light1440: Story = { render: () => <Pair width={1440} light /> }
export const Modal240: Story = { render: () => <Pair width={240} /> }
export const Modal240Light: Story = { render: () => <Pair width={240} light /> }
export const AllStates: Story = { render: () => <main {...stylex.props(styles.matrix)}>{[240, 440, 1440].flatMap(width => [false, true].map(light => <Example key={`${width}-${light}`} width={width} light={light} />))}</main> }
const styles = stylex.create({
  matrix: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.xl, padding: s.xl, backgroundColor: surface.canvas },
  example: { minWidth: 0, color: ink.fg, backgroundColor: surface.sidebar, fontFamily: t.fontSans, fontSize: t.metaSize },
  narrow: { width: sizes.narrow }, small: { width: sizes.small }, wide: { width: sizes.wide },
  toolbar: { display: 'flex', flexWrap: 'wrap', gap: s.xs, padding: s.xs },
  viewport: { height: sizes.viewport, minHeight: 0 },
  label: { display: 'flex', alignItems: 'center', height: sizes.row, overflow: 'hidden', whiteSpace: 'nowrap', paddingInline: s.sm, minWidth: 0 },
  resized: { height: sizes.resizedRow },
})
