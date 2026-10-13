import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button, ToggleButton } from 'react-aria-components'
import { Icon } from '../assistant-ui/composition/Icons'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { lightTheme } from '../assistant-ui/composition-theme'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { VariantPicker, VariantPickerTable } from './VariantPicker'
import { ProgressSpinner, spinnerChoices, type SpinnerSize, type SpinnerVariant } from './ProgressSpinner'
import { spinnerGeometry as sg } from './spinner-tokens.stylex'

const dimensions = {
  S: spinnerChoices,
  Z: [{ id: '12', description: '12px glyph, centred in the 16px status column.' }, { id: '14', description: '14px, the status-icon size of work rows and sidebar glyphs.' }, { id: '16', description: '16px, the full icon size; fills the status column.' }],
  W: [{ id: '440', description: '440 host: the agent list stacks above the thread.' }, { id: '1440', description: '1440 host: a 256px sidebar beside the thread.' }],
  M: [{ id: 'M1', description: 'Animated: transform-only motion that the compositor runs.' }, { id: 'M2', description: 'Reduced motion: the prefers-reduced-motion frame; nothing animates.' }],
} as const
type Size = typeof dimensions.Z[number]['id']
type Width = typeof dimensions.W[number]['id']
type Motion = typeof dimensions.M[number]['id']
interface Args { spinner: SpinnerVariant; size: Size; width: Width; motion: Motion; compareS: boolean; compareZ: boolean; compareW: boolean; compareM: boolean; scheme: 'dark' | 'light'; probe: boolean }
const defaults: Args = { spinner: 'S1', size: '14', width: '1440', motion: 'M1', compareS: false, compareZ: true, compareW: false, compareM: false, scheme: 'dark', probe: false }
/** Synthetic fixture copy. */
const fixture = { agent: 'north-studio', idleAgent: 'harbor-ops', elapsed: '2m', idleTime: '9:41', task: 'Rebase the release branch', command: 'pnpm --filter @smalltalk/fractal-ui typecheck', settledCommand: 'pnpm --filter @smalltalk/fractal-ui build-storybook', draft: 'Tighten the spinner sizes in the sidebar' }

/** One app frame: sidebar agent row, thread header status, a running work row and the composer send, each with its settled neighbour. */
function Shell({ variant, size, width, motion }: { readonly variant: SpinnerVariant; readonly size: Size; readonly width: Width; readonly motion: Motion }) {
  const px = Number(size) as SpinnerSize
  const spinner = <ProgressSpinner variant={variant} size={px} reduced={motion === 'M2'} />
  const name = `${variant} · ${size}px · ${width} · ${motion === 'M1' ? 'animated' : 'reduced motion'}`
  const narrow = width === '440'
  return <section aria-label={name} data-shell data-spinner-variant={variant} data-size={size} data-width={width} data-motion={motion} {...stylex.props(styles.lane, narrow ? styles.host440 : styles.host1440)}>
    <h2 {...stylex.props(styles.caption)}>{name}</h2>
    <div {...stylex.props(styles.shell, narrow ? styles.stacked : styles.split)}>
      <nav aria-label={`Agents · ${name}`} data-row-pair {...stylex.props(styles.sidebar, narrow ? styles.sidebarStacked : styles.sidebarSplit)}>
        <div data-context="sidebar" data-row {...stylex.props(styles.sideRow)}>
          <span {...stylex.props(styles.slot, styles.softInk)}>{spinner}</span>
          <span data-baseline data-row-title {...stylex.props(styles.sideTitle)}>{fixture.agent}</span>
          <span data-baseline {...stylex.props(styles.meta)}>Working · {fixture.elapsed}</span>
        </div>
        <div data-row {...stylex.props(styles.sideRow)}>
          <span {...stylex.props(styles.slot, styles.mutedInk)}><Icon name="dot" size={px} /></span>
          <span data-row-title {...stylex.props(styles.sideTitle)}>{fixture.idleAgent}</span>
          <span {...stylex.props(styles.meta)}>Idle · {fixture.idleTime}</span>
        </div>
      </nav>
      <div {...stylex.props(styles.thread)}>
        <header data-context="header" {...stylex.props(styles.header)}>
          <strong data-baseline {...stylex.props(styles.headerTitle)}>{fixture.task}</strong>
          <span role="status" {...stylex.props(styles.headerStatus)}>{spinner}<span data-baseline>Working</span></span>
        </header>
        <div role="list" aria-label={`Work · ${name}`} data-row-pair {...stylex.props(styles.work)}>
          <div role="listitem" data-context="work" data-row {...stylex.props(styles.workRow)}>
            <span {...stylex.props(styles.slot)}>{spinner}</span><Icon name="drawer" size={14} />
            <code data-baseline data-row-title {...stylex.props(styles.workLabel)}>{fixture.command}</code>
            <span data-baseline {...stylex.props(styles.workState)}>Running</span>
          </div>
          <div role="listitem" data-row {...stylex.props(styles.workRow)}>
            <span {...stylex.props(styles.slot)}><Icon name="check" size={px} /></span><Icon name="drawer" size={14} />
            <code data-row-title {...stylex.props(styles.workLabel)}>{fixture.settledCommand}</code>
            <span {...stylex.props(styles.workState)}>OK</span>
          </div>
        </div>
        <div data-context="composer" {...stylex.props(styles.composer)}>
          <span data-baseline {...stylex.props(styles.draft)}>{fixture.draft}</span>
          <Button isDisabled aria-label="Sending message" {...stylex.props(styles.send)}>{spinner}</Button>
        </div>
      </div>
    </div>
  </section>
}

function Explorer(args: Args & { readonly onChange: (patch: Partial<Args>) => void }) {
  const variants = args.compareS ? dimensions.S.map(choice => choice.id) : [args.spinner]
  const widths = args.compareW ? dimensions.W.map(choice => choice.id) : [args.width]
  const motions = args.compareM ? dimensions.M.map(choice => choice.id) : [args.motion]
  const sizes = args.compareZ ? dimensions.Z.map(choice => choice.id) : [args.size]
  return <main data-spinner-explorer data-current-spinner={args.spinner} data-current-size={args.size} data-current-width={args.width} data-current-motion={args.motion} data-scheme={args.scheme} {...stylex.props(...baselineTheme, args.scheme === 'light' && lightTheme, styles.root)}>
    {/* Resolved ink references for the play: spinners may only take fg, fgSoft or fgMuted. */}
    <span aria-hidden="true" {...stylex.props(styles.inkProbe)}><span data-ink="fg" {...stylex.props(styles.fgInk)} /><span data-ink="fgSoft" {...stylex.props(styles.softInk)} /><span data-ink="fgMuted" {...stylex.props(styles.mutedInk)} /></span>
    <h1 {...stylex.props(styles.title)}>Progress spinners</h1>
    <p {...stylex.props(styles.explanation)}>Four spinners in the sidebar agent row, the thread header status, a running work row and the composer send. Motion is transform-only, pauses off screen, and removing a spinner moves no text. S1 was picked and ships as the kit spinner (Fractal/Foundations/Spinner).</p>
    <div {...stylex.props(styles.pickers)}>
      <VariantPickerTable label="Variant pickers">
        <VariantPicker label="Spinner" choices={dimensions.S} value={args.spinner} compare={args.compareS} note="Picked: S1, shipped." onChange={spinner => args.onChange({ spinner: spinner as SpinnerVariant })} onCompare={compareS => args.onChange({ compareS })} />
        <VariantPicker label="Size" choices={dimensions.Z} value={args.size} compare={args.compareZ} onChange={size => args.onChange({ size: size as Size })} onCompare={compareZ => args.onChange({ compareZ })} />
        <VariantPicker label="Width" choices={dimensions.W} value={args.width} compare={args.compareW} onChange={width => args.onChange({ width: width as Width })} onCompare={compareW => args.onChange({ compareW })} />
        <VariantPicker label="Motion" choices={dimensions.M} value={args.motion} compare={args.compareM} onChange={motion => args.onChange({ motion: motion as Motion })} onCompare={compareM => args.onChange({ compareM })} />
      </VariantPickerTable>
      <ToggleButton isSelected={args.scheme === 'light'} onChange={light => args.onChange({ scheme: light ? 'light' : 'dark' })} {...stylex.props(styles.control, args.scheme === 'light' && styles.controlOn)}>Light scheme</ToggleButton>
    </div>
    <div data-lanes {...stylex.props(styles.lanes)}>{variants.flatMap(variant => widths.flatMap(width => motions.flatMap(motion => sizes.map(size => <Shell key={`${variant}-${width}-${motion}-${size}`} variant={variant} size={size} width={width} motion={motion} />))))}</div>
  </main>
}
/** Probe interactions own their mount instead of Storybook's arg lifecycle. */
function ProbeExplorer(initial: Args) {
  const [args, setArgs] = React.useState(initial)
  return <Explorer {...args} onChange={patch => setArgs(current => ({ ...current, ...patch }))} />
}

// TODO: remove this exploration (and src/explore/ProgressSpinner*, spinner-tokens) once the shipped S1 spinner has settled.
const meta = {
  title: 'Fractal/Explore/ProgressSpinners',
  render: function Render() {
    const [args, updateArgs] = useArgs<Args>()
    return args.probe ? <ProbeExplorer {...args} /> : <Explorer {...args} onChange={updateArgs} />
  },
  parameters: { layout: 'fullscreen' },
  args: defaults,
  argTypes: {
    spinner: { control: 'select', options: dimensions.S.map(choice => choice.id) },
    size: { control: 'select', options: dimensions.Z.map(choice => choice.id) },
    width: { control: 'select', options: dimensions.W.map(choice => choice.id) },
    motion: { control: 'select', options: dimensions.M.map(choice => choice.id) },
    compareS: { control: 'boolean' }, compareZ: { control: 'boolean' }, compareW: { control: 'boolean' }, compareM: { control: 'boolean' },
    scheme: { control: 'inline-radio', options: ['dark', 'light'] },
    probe: { table: { disable: true } },
  },
} satisfies Meta<Args>
export default meta
type Story = StoryObj<typeof meta>

const snap = (value: number) => Math.round(value * 64) / 64
const textBox = (element: Element) => {
  const range = document.createRange()
  range.selectNodeContents(element)
  return range.getBoundingClientRect()
}
/** Top and bottom of every baseline-tracked text run, so a baseline shift of any size registers. */
const textEdges = (context: HTMLElement) => [...context.querySelectorAll('[data-baseline]')].flatMap(text => { const box = textBox(text); return [snap(box.top), snap(box.bottom)] })
const timingKeys: Record<string, true> = { offset: true, computedOffset: true, easing: true, composite: true }

/** Compositor-only motion: every animation sits inside the fixed layout box, on a box-level element, and changes transform only. */
function assertMotion(shell: HTMLElement) {
  const still = shell.dataset.motion === 'M2' || window.matchMedia('(prefers-reduced-motion: reduce)').matches
  for (const spinner of shell.querySelectorAll<HTMLElement>('[data-spinner]')) {
    const animations = spinner.getAnimations({ subtree: true })
    if (still) { expect(animations).toHaveLength(0); continue }
    expect(animations.length).toBeGreaterThan(0)
    for (const animation of animations) {
      const effect = animation.effect as KeyframeEffect
      const target = effect.target
      expect(target).not.toBe(spinner)
      // Compositor transforms need a CSS box: an HTML element or an outermost <svg>, never a shape inside one.
      expect(target instanceof HTMLElement || (target instanceof SVGSVGElement && target.ownerSVGElement === null)).toBe(true)
      expect([...new Set(effect.getKeyframes().flatMap(frame => Object.keys(frame).filter(key => timingKeys[key] !== true)))]).toEqual(['transform'])
    }
  }
}
/** No spinner shifts text: text edges hold at every animation phase, and match with the spinner removed. */
function assertBaselines(shell: HTMLElement) {
  const contexts = [...shell.querySelectorAll<HTMLElement>('[data-context]')]
  expect(contexts.map(context => context.dataset.context)).toEqual(['sidebar', 'header', 'work', 'composer'])
  for (const context of contexts) {
    const spinner = context.querySelector<HTMLElement>('[data-spinner]')!
    const size = Number(spinner.dataset.spinnerSize)
    expect([spinner.offsetWidth, spinner.offsetHeight]).toEqual([size, size])
    const live = textEdges(context)
    expect(live.length).toBeGreaterThan(0)
    for (const fraction of [0.25, 0.5, 0.75]) {
      for (const animation of spinner.getAnimations({ subtree: true })) animation.currentTime = Number(animation.effect?.getComputedTiming().duration) * fraction
      expect(textEdges(context)).toEqual(live)
      expect([spinner.offsetWidth, spinner.offsetHeight]).toEqual([size, size])
    }
    spinner.style.display = 'none'
    const without = textEdges(context)
    spinner.style.removeProperty('display')
    expect(without).toEqual(live)
  }
  // A live row and its settled neighbour place their titles identically.
  for (const pair of shell.querySelectorAll<HTMLElement>('[data-row-pair]')) {
    const offsets = [...pair.querySelectorAll<HTMLElement>('[data-row]')].map(row => snap(textBox(row.querySelector('[data-row-title]')!).top - row.getBoundingClientRect().top))
    expect(offsets).toHaveLength(2)
    expect(offsets[0]).toBe(offsets[1])
  }
}
/** WCAG 2.5.3 label in name: every button's accessible name starts with its visible label. */
function assertLabelInName(root: HTMLElement) {
  const labelled = [...root.querySelectorAll<HTMLElement>('button')].flatMap(button => {
    const visible = button.cloneNode(true) as HTMLElement
    for (const hidden of visible.querySelectorAll('[aria-hidden="true"]')) hidden.remove()
    const label = visible.textContent!.trim()
    return label === '' ? [] : [{ label, name: button.getAttribute('aria-label') ?? label }]
  })
  expect(labelled.map(({ label }) => label)).toContain('Details')
  for (const { label, name } of labelled) expect(name.slice(0, label.length)).toBe(label)
}
function assertExplorer(canvas: HTMLElement) {
  const root = canvas.querySelector<HTMLElement>('[data-spinner-explorer]')!
  expect(root.querySelectorAll('[title], title')).toHaveLength(0)
  assertLabelInName(root)
  const inks = new Set([...root.querySelectorAll<HTMLElement>('[data-ink]')].map(probe => getComputedStyle(probe).color))
  expect(inks.size).toBe(3)
  const shells = [...root.querySelectorAll<HTMLElement>('[data-shell]')]
  expect(shells.length).toBeGreaterThan(0)
  for (const shell of shells) {
    const spinners = [...shell.querySelectorAll<HTMLElement>('[data-spinner]')]
    expect(spinners).toHaveLength(4)
    for (const spinner of spinners) {
      expect(spinner.dataset.spinner).toBe(shell.dataset.spinnerVariant)
      expect(spinner.dataset.spinnerSize).toBe(shell.dataset.size)
      expect(spinner.getAttribute('aria-hidden')).toBe('true')
      expect(inks.has(getComputedStyle(spinner).color)).toBe(true)
    }
    assertMotion(shell)
    assertBaselines(shell)
  }
}
const pick = (canvasElement: HTMLElement, id: string) => userEvent.click(within(canvasElement).getByRole('button', { name: new RegExp(`^${id}: `) }))

/** Deep links land here; the play checks whatever the URL args select, without changing it. */
export const Exploration: Story = { play: async ({ canvasElement }) => {
  await waitFor(() => expect(canvasElement.querySelector('[data-shell]')).not.toBeNull())
  assertExplorer(canvasElement)
} }

export const ExplorationProbe: Story = { args: { probe: true }, play: async ({ canvasElement, step }) => {
  const canvas = within(canvasElement)
  const root = () => canvasElement.querySelector<HTMLElement>('[data-spinner-explorer]')!
  for (const spinner of dimensions.S) for (const width of dimensions.W) for (const motion of dimensions.M) await step(`${spinner.id} · ${width.id} · ${motion.id}: sizes 12/14/16 after real picker presses`, async () => {
    await pick(canvasElement, spinner.id)
    await pick(canvasElement, width.id)
    await pick(canvasElement, motion.id)
    await waitFor(() => {
      expect(root().dataset.currentSpinner).toBe(spinner.id)
      expect(root().dataset.currentWidth).toBe(width.id)
      expect(root().dataset.currentMotion).toBe(motion.id)
      expect([...root().querySelectorAll<HTMLElement>('[data-shell]')].map(shell => shell.dataset.size)).toEqual(['12', '14', '16'])
    })
    assertExplorer(canvasElement)
  })
  await step('Single size, then every spinner × width × motion × size together', async () => {
    await userEvent.click(canvas.getByRole('button', { name: 'Compare Size' }))
    await pick(canvasElement, '16')
    await waitFor(() => expect([...root().querySelectorAll<HTMLElement>('[data-shell]')].map(shell => shell.dataset.size)).toEqual(['16']))
    assertExplorer(canvasElement)
    for (const label of ['Spinner', 'Size', 'Width', 'Motion']) await userEvent.click(canvas.getByRole('button', { name: `Compare ${label}` }))
    await waitFor(() => expect(root().querySelectorAll('[data-shell]')).toHaveLength(48))
    assertExplorer(canvasElement)
    for (const label of ['Spinner', 'Width', 'Motion']) await userEvent.click(canvas.getByRole('button', { name: `Compare ${label}` }))
    await pick(canvasElement, 'S2')
    await pick(canvasElement, 'M1')
    await waitFor(() => expect([...root().querySelectorAll<HTMLElement>('[data-shell]')].map(shell => `${shell.dataset.spinnerVariant}/${shell.dataset.motion}`)).toEqual(['S2/M1', 'S2/M1', 'S2/M1']))
  })
  await step('Negative controls: each injected regression fails its assertion, then the clean view passes', async () => {
    const shell = root().querySelector<HTMLElement>('[data-shell]')!
    const work = shell.querySelector<HTMLElement>('[data-context="work"] [data-spinner]')!
    const layoutMotion = work.firstElementChild!.animate([{ marginLeft: '0px' }, { marginLeft: '1px' }], { duration: 1000, iterations: Infinity })
    expect(() => assertMotion(shell)).toThrow()
    layoutMotion.cancel(); assertMotion(shell)
    work.style.marginBlockStart = g.icon
    expect(() => assertBaselines(shell)).toThrow()
    work.style.removeProperty('margin-block-start'); assertBaselines(shell)
    work.style.color = 'rgb(34, 197, 94)'
    expect(() => assertExplorer(canvasElement)).toThrow()
    work.style.removeProperty('color')
    shell.querySelector('[data-context="header"] [data-baseline]')!.setAttribute('title', 'Working')
    expect(() => assertExplorer(canvasElement)).toThrow()
    shell.querySelector('[data-context="header"] [data-baseline]')!.removeAttribute('title')
    const details = canvas.getByRole('button', { name: 'Details: Spinner descriptions' })
    details.setAttribute('aria-label', 'Spinner descriptions · Details')
    expect(() => assertLabelInName(root())).toThrow()
    details.setAttribute('aria-label', 'Details: Spinner descriptions')
    assertExplorer(canvasElement)
  })
  await step('The other scheme resolves different inks and passes the same assertions', async () => {
    const before = getComputedStyle(root().querySelector('[data-ink="fg"]')!).color
    const scheme = root().dataset.scheme
    await userEvent.click(canvas.getByRole('button', { name: 'Light scheme' }))
    await waitFor(() => expect(root().dataset.scheme).not.toBe(scheme))
    expect(getComputedStyle(root().querySelector('[data-ink="fg"]')!).color).not.toBe(before)
    assertExplorer(canvasElement)
  })
} }

const styles = stylex.create({
  // The canvas grows with the widest lane, so a 1440 host past a narrower viewport scrolls the page on the themed surface.
  root: { width: 'max-content', minWidth: '100%', padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, boxSizing: 'border-box' },
  inkProbe: { display: 'none' },
  fgInk: { color: ink.fg }, softInk: { color: ink.fgSoft }, mutedInk: { color: ink.fgMuted },
  title: { margin: 0, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightMedium },
  explanation: { maxWidth: g.lane, marginBlock: s.md, color: ink.fgMuted },
  pickers: { display: 'flex', flexDirection: 'column', alignItems: 'start', gap: s.md },
  control: { minHeight: g.controlSm, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover }, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: accent.primary, outlineOffset: s.xs2 },
  controlOn: { backgroundColor: surface.rowActive, borderColor: accent.primary, color: ink.fg },
  lanes: { display: 'flex', flexDirection: 'column', gap: s.section, marginTop: s.section },
  lane: { flexShrink: 0 }, host440: { width: sg.host440 }, host1440: { width: sg.host1440 },
  caption: { margin: 0, marginBottom: s.md, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightRegular },
  shell: { display: 'grid', overflow: 'hidden', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.canvas },
  split: { gridTemplateColumns: `${g.sidebarDefault} minmax(0, 1fr)` }, stacked: { gridTemplateColumns: 'minmax(0, 1fr)' },
  sidebar: { display: 'flex', flexDirection: 'column', gap: s.xs2, padding: s.md, backgroundColor: surface.sidebar, borderColor: border.border, borderStyle: 'solid', borderWidth: 0 },
  sidebarSplit: { borderInlineEndWidth: g.hairline }, sidebarStacked: { borderBlockEndWidth: g.hairline },
  sideRow: { display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.sidebarNested, paddingInline: s.md, borderRadius: r.sm, color: ink.fg },
  // The status column is a fixed 16px wide; its height follows the glyph so a taller glyph would show up as a shift.
  slot: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, flexShrink: 0 },
  sideTitle: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontWeight: t.weightMedium },
  meta: { flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  thread: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  header: { display: 'flex', alignItems: 'center', gap: s.md, height: g.band, minHeight: g.band, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  headerTitle: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontWeight: t.weightMedium },
  headerStatus: { display: 'inline-flex', alignItems: 'center', gap: s.sm, flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  work: { display: 'flex', flexDirection: 'column', gap: s.xs2, padding: s.lg },
  workRow: { display: 'flex', alignItems: 'center', gap: s.sm, minHeight: g.toolRow, paddingInline: s.xs, color: ink.fgMuted },
  workLabel: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading },
  workState: { flexShrink: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  composer: { display: 'flex', alignItems: 'center', gap: s.md, marginInline: s.lg, marginBottom: s.lg, padding: s.md, paddingInlineStart: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.xl, backgroundColor: surface.raised },
  draft: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgSoft },
  // In flight, the send control is neutral: the spinner carries the state, not an accent fill.
  send: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.controlLg, height: g.controlLg, padding: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.full, backgroundColor: surface.controlFill, color: ink.fgSoft },
})
