import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor, within } from 'storybook/test'
import { Spinner } from './Spinner'
import { ThreadHeader } from './Shell'
import { LiveActivity } from './Transcript'
import { SidebarAgentRow } from '../sidebar/SidebarAgentRow'
import type { SidebarAgentRow as Row } from '../sidebar/model'
import { WorkLogV1 } from '../taste/WorkLogV1'
import type { WorkLogCall, WorkLogTurn } from '../taste/work-log'
import { baselineTheme } from '../neutral-theme'
import { lightTheme, type Scheme } from '../composition-theme'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/** Synthetic fixtures: one working agent, its idle neighbour, and a run with one live and one settled command. */
const now = Date.UTC(2026, 9, 13, 12, 0, 0)
const working: Row = {
  ref: 'agent/north-studio', id: 'north-studio', title: 'north-studio', description: 'Rebases the release branch',
  status: 'working', statusLabel: 'Working', host: 'host-1', statusSince: now - 120_000, lastActivityAt: now - 5_000, needsMe: false, freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Known', scope: '24h-activity-span', ms: 120_000 }, lastTurn: { _tag: 'Unknown' }, children: [],
}
const idle: Row = { ...working, ref: 'agent/harbor-ops', id: 'harbor-ops', title: 'harbor-ops', description: undefined, status: 'idle', statusLabel: 'Idle' }
const calls: readonly WorkLogCall[] = [
  { id: 'call-typecheck', kind: 'run', title: 'pnpm --filter @smalltalk/fractal-ui typecheck', status: 'running', startedAt: new Date(now - 12_000).toISOString() },
  { id: 'call-build', kind: 'run', title: 'pnpm --filter @smalltalk/fractal-ui build-storybook', status: 'success', startedAt: new Date(now - 60_000).toISOString(), endedAt: new Date(now - 20_000).toISOString() },
]
const run: WorkLogTurn = { calls, durationMs: undefined, running: true, failed: false, interrupted: false, startedAt: new Date(now - 60_000).toISOString() }
const sizes = [12, 14, 16] as const

/**
 * The production spinner (Lucide loader-circle, one linear turn per second) in the sidebar agent row, the thread
 * header status and the work block. A send in flight in the composer adopts it once the composer send states land.
 */
function Spinners({ scheme, width = 1440 }: { readonly scheme: Scheme; readonly width?: 440 | 1440 }) {
  return <main data-spinner-root data-scheme={scheme} data-width={width} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.root, styles.width(width))}>
    <section aria-label="Sizes" data-context="sizes" {...stylex.props(styles.sizes)}>
      {sizes.map(size => <span key={size} data-size={size} {...stylex.props(styles.size)}><Spinner size={size} /><span>{size}px · Working</span></span>)}
    </section>
    <section aria-label="Sidebar agent row" data-context="sidebar" {...stylex.props(styles.sidebar)}>
      <SidebarAgentRow item={working} now={now} active />
      <SidebarAgentRow item={idle} now={now} />
    </section>
    <section aria-label="Thread header" data-context="header" {...stylex.props(styles.frame)}>
      <ThreadHeader folder="fractal" title="Rebase the release branch" status="working" statusLabel="Working" statusSince={now - 120_000} now={now} freshness="live" panelOpen={false} drawerOpen={false} onTogglePanel={() => {}} onToggleDrawer={() => {}} />
    </section>
    <section aria-label="Work block" data-context="work" {...stylex.props(styles.frame, styles.work)}>
      <WorkLogV1 turn={run} ariaLabel="Work log, running" interactiveCalls={false} />
      <LiveActivity state={{ _tag: 'Working' }} startedAt={run.startedAt} now={now} />
    </section>
    {/* Focusable, so keyboard users can scroll the clipped run into view. */}
    <section aria-label="Off-screen work" data-context="offscreen" tabIndex={0} {...stylex.props(styles.frame, styles.scroller)}>
      <div {...stylex.props(styles.spacer)} />
      <LiveActivity state={{ _tag: 'Working' }} startedAt={run.startedAt} now={now} />
    </section>
  </main>
}

const meta = {
  title: 'Fractal/Foundations/Spinner',
  component: Spinners,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', width: 1440 },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'inline-radio' }, width: { options: [440, 1440], control: 'inline-radio' } },
} satisfies Meta<typeof Spinners>
export default meta
type Story = StoryObj<typeof meta>

const loaderPaths: Record<string, true> = { 'M14 8a6 6 0 1 1-4.146-5.707': true }
const timingKeys: Record<string, true> = { offset: true, computedOffset: true, easing: true, composite: true }
const contexts = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>('[data-context]:not([data-context="offscreen"])')]
/** Every animated glyph; assertMotion proves each target is an outermost <svg>. */
const spinnersIn = (element: Element) => element.getAnimations({ subtree: true }).map(animation => (animation.effect as KeyframeEffect).target as SVGSVGElement)
const snap = (value: number) => Math.round(value * 64) / 64
/** Top and bottom of every visible text run, so a baseline shift of any size registers. */
function textEdges(context: HTMLElement) {
  const walker = document.createTreeWalker(context, NodeFilter.SHOW_TEXT, { acceptNode: node => node.textContent!.trim() === '' ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT })
  const edges: number[] = []
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const range = document.createRange()
    range.selectNodeContents(node)
    const box = range.getBoundingClientRect()
    if (box.height > 0) edges.push(snap(box.top), snap(box.bottom))
  }
  return edges
}

/** Compositor-only motion: every animation is the loader-circle glyph's own <svg> turning by transform, once a second, forever. */
function assertMotion(root: HTMLElement) {
  for (const context of contexts(root)) {
    const animations = context.getAnimations({ subtree: true })
    expect(animations.length).toBeGreaterThan(0)
    for (const animation of animations) {
      const effect = animation.effect as KeyframeEffect
      const target = effect.target
      expect(target instanceof SVGSVGElement && target.ownerSVGElement === null).toBe(true)
      expect(loaderPaths[(target as SVGSVGElement).querySelector('path')!.getAttribute('d')!]).toBe(true)
      expect([...new Set(effect.getKeyframes().flatMap(frame => Object.keys(frame).filter(key => timingKeys[key] !== true)))]).toEqual(['transform'])
      const timing = effect.getComputedTiming()
      expect([timing.duration, timing.iterations, ...new Set(effect.getKeyframes().map(frame => frame.easing))]).toEqual([1000, Infinity, 'linear'])
    }
  }
}
/** No spinner shifts text: text edges hold at every animation phase and match with the spinner removed. */
function assertBaselines(root: HTMLElement) {
  for (const context of contexts(root)) {
    const live = textEdges(context)
    expect(live.length).toBeGreaterThan(0)
    const animations = context.getAnimations({ subtree: true })
    for (const fraction of [0.25, 0.5, 0.75]) {
      for (const animation of animations) animation.currentTime = 1000 * fraction
      expect(textEdges(context)).toEqual(live)
    }
    const boxes = spinnersIn(context).map(glyph => glyph.parentElement!)
    for (const box of boxes) box.style.display = 'none'
    const without = textEdges(context)
    for (const box of boxes) box.style.removeProperty('display')
    expect(without).toEqual(live)
  }
}
/** Each spinning glyph's own class switches the animation off under prefers-reduced-motion. */
function assertReducedMotion(glyph: Element) {
  const rules = [...document.styleSheets].flatMap(sheet => [...sheet.cssRules]).filter((rule): rule is CSSMediaRule => rule instanceof CSSMediaRule && rule.conditionText.includes('prefers-reduced-motion: reduce'))
  const still = rules.flatMap(rule => [...rule.cssRules]).filter((rule): rule is CSSStyleRule => rule instanceof CSSStyleRule && rule.style.animationName === 'none')
  expect([...glyph.classList].some(name => still.some(rule => rule.selectorText.includes(`.${name}`)))).toBe(true)
}
/** Off-screen spinners stop; on-screen spinners run. */
function assertPlayState(glyph: Element, state: 'running' | 'paused') {
  expect(glyph.getAnimations().map(animation => animation.playState)).toEqual([state])
}

/** Every context at 12, 14 and 16px: transform-only motion, stable baselines, reduced motion. */
export const Contexts: Story = { play: async ({ canvasElement, step }) => {
  const root = canvasElement.querySelector<HTMLElement>('[data-spinner-root]')!
  await waitFor(() => expect(contexts(root).every(context => context.getAnimations({ subtree: true }).length > 0)).toBe(true))
  await step('Compositor-only motion and stable baselines in the sidebar row, thread header and work block', async () => {
    expect(contexts(root).map(context => context.dataset.context)).toEqual(['sizes', 'sidebar', 'header', 'work'])
    // Rotation never changes the layout box, so the computed width is the size even mid-turn.
    expect([...root.querySelectorAll<HTMLElement>('[data-size]')].map(size => spinnersIn(size).map(glyph => getComputedStyle(glyph).width))).toEqual(sizes.map(size => [`${size}px`]))
    // The working row and the thread header keep the status glyph's accessible name.
    expect(within(root).getAllByRole('img', { name: /^Working;/ }).length).toBeGreaterThanOrEqual(2)
    assertMotion(root)
    assertBaselines(root)
    for (const glyph of spinnersIn(root)) assertReducedMotion(glyph)
  })
  await step('Negative controls: each injected regression fails its assertion, then the clean view passes', async () => {
    const glyph = spinnersIn(within(root).getByRole('region', { name: 'Work block' }))[0]!
    const layoutMotion = glyph.animate([{ marginLeft: '0px' }, { marginLeft: '1px' }], { duration: 1000, iterations: Infinity })
    expect(() => assertMotion(root)).toThrow()
    layoutMotion.cancel()
    // A spinner that grows its row moves the row's text.
    const box = glyph.parentElement!
    box.style.marginBlockStart = g.icon
    expect(() => assertBaselines(root)).toThrow()
    box.style.removeProperty('margin-block-start')
    // A spinning glyph without the guarded class keeps turning under reduced motion.
    const unguarded = glyph.cloneNode(true) as SVGSVGElement
    unguarded.removeAttribute('class')
    expect(() => assertReducedMotion(unguarded)).toThrow()
    assertMotion(root)
    assertBaselines(root)
  })
} }
export const ContextsLight: Story = { ...Contexts, args: { scheme: 'light' } }
export const Contexts440: Story = { ...Contexts, args: { width: 440 } }
export const Contexts440Light: Story = { ...Contexts, args: { width: 440, scheme: 'light' } }

/** A spinner scrolled out of view pauses and resumes when it scrolls back. */
export const OffscreenPause: Story = { play: async ({ canvasElement }) => {
  const root = canvasElement.querySelector<HTMLElement>('[data-spinner-root]')!
  const scroller = within(root).getByRole('region', { name: 'Off-screen work' })
  const hidden = spinnersIn(scroller)[0]!
  const visible = spinnersIn(within(root).getByRole('region', { name: 'Work block' }))[0]!
  await waitFor(() => assertPlayState(hidden, 'paused'))
  assertPlayState(visible, 'running')
  // Negative control: a forced-running off-screen glyph fails the paused check.
  hidden.style.animationPlayState = 'running'
  expect(() => assertPlayState(hidden, 'paused')).toThrow()
  hidden.style.animationPlayState = 'paused'
  // Scrolls the scroller and, on short viewports, the page, so the glyph is on screen either way.
  hidden.scrollIntoView({ block: 'center' })
  await waitFor(() => assertPlayState(hidden, 'running'))
  scroller.scrollTop = 0
  await waitFor(() => assertPlayState(hidden, 'paused'))
} }
export const OffscreenPauseLight: Story = { ...OffscreenPause, args: { scheme: 'light' } }

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.xl, padding: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  width: (width: number) => ({ width, maxWidth: '100%' }),
  sizes: { display: 'flex', alignItems: 'center', gap: s.xl, color: ink.fgSoft },
  size: { display: 'inline-flex', alignItems: 'center', gap: s.sm },
  sidebar: { display: 'flex', flexDirection: 'column', gap: s.xs, width: g.sidebarDefault },
  frame: { maxWidth: g.lane, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.canvas },
  work: { display: 'flex', flexDirection: 'column', gap: s.sm, padding: s.md },
  scroller: { height: g.threadViewportMin, overflowY: 'auto', paddingInline: s.md },
  spacer: { height: g.previewMax },
})
