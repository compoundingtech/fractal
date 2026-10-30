import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { NeedsYouBar, NeedsYouInbox, type NeedsYouRequest } from './assistant-ui/NeedsYou'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t, spaceVars as s, geometryVars as g } from './assistant-ui/composition-tokens.stylex'
import { needsYouRequest, needsYouRequests } from './needs-you-fixtures'

/** Stands in for the host-owned body (an answer form or a terminal hand-off); the kit only frames it. */
const HostPanel = ({ request, label, onOpenTerminal }: { readonly request: NeedsYouRequest; readonly label: string; readonly onOpenTerminal: () => void }) => <section aria-label={label} {...stylex.props(styles.panel)}>
  <p {...stylex.props(styles.prompt)}>{request.prompt}</p>
  <Button {...stylex.props(styles.hostButton)} onPress={onOpenTerminal}>Open terminal</Button>
</section>

function AttentionComposition({ scrollDriven = false }: { readonly scrollDriven?: boolean }) {
  const [expanded, setExpanded] = React.useState(true)
  const [ratio, setRatio] = React.useState(0)
  const scroll = React.useRef<HTMLDivElement>(null)
  const observe = React.useCallback((node: HTMLElement | null) => {
    if (node === null || !scrollDriven) return
    const observer = new IntersectionObserver(entries => setRatio(entries[0]?.intersectionRatio ?? 0), { root: scroll.current, threshold: [0, 0.6, 1] })
    observer.observe(node)
    return () => observer.disconnect()
  }, [scrollDriven])
  return <>
    <div ref={scroll} data-needs-you-scroll {...stylex.props(styles.scroll)}>
      <article ref={observe} aria-label="Request in thread" {...stylex.props(styles.panel)}><p {...stylex.props(styles.prompt)}>{needsYouRequest.agentLabel} · {needsYouRequest.prompt}</p></article>
      {scrollDriven ? <div {...stylex.props(styles.scrollSpace)}>Earlier conversation</div> : null}
    </div>
    <NeedsYouBar requests={[needsYouRequest]} expanded={expanded} onToggle={() => setExpanded(value => !value)} transcriptVisibility={scrollDriven ? ratio : 0}><HostPanel request={needsYouRequest} label="Pinned request" onOpenTerminal={fn()} /></NeedsYouBar>
    {!scrollDriven ? <NeedsYouInbox requests={[needsYouRequest]} onShowInThread={fn()} onOpenTerminal={fn()}>{() => <HostPanel request={needsYouRequest} label="Inbox request" onOpenTerminal={fn()} />}</NeedsYouInbox> : null}
  </>
}

const assertAttentionComposition = async (root: HTMLElement) => {
  await expect(root.querySelector('[data-fractal-card] [data-fractal-card]')).toBeNull()
  for (const dock of root.querySelectorAll<HTMLElement>('aside[data-fractal-card]')) await expect(dock.querySelector('button svg[data-icon="question"]')).not.toBeNull()
}

const assertKeyboardRing = async (target: HTMLElement) => {
  if (target.tagName === 'BUTTON') await expect(getComputedStyle(target).outlineStyle).toBe('none')
  await userEvent.tab({ shift: true })
  await userEvent.tab()
  await expect(target).toHaveFocus()
  await expect(getComputedStyle(target).outlineStyle).toBe('solid')
  await expect(getComputedStyle(target).outlineWidth).toBe('2px')
}

const assertBlockGap = async (target: HTMLElement) => {
  await expect(getComputedStyle(target).display).toBe('flex')
  await expect(getComputedStyle(target).flexDirection).toBe('column')
  await expect(getComputedStyle(target).rowGap).toBe('8px')
}

const meta = {
  title: 'Fractal/Plugins/Questions/Needs you',
  excludeStories: ['scenarioExemptions'],
  scenario: { _tag: 'contract-fixture', reason: 'Isolated needs-you framing and keyboard contracts use host-owned request fixtures.' },
  parameters: { layout: 'fullscreen' },
  argTypes: { scheme: { control: 'select', options: ['dark', 'light'] }, width: { control: 'select', options: [440, 1440] } },
  decorators: [(Story, context) => <main data-scheme={context.args.scheme ?? context.parameters.scheme ?? 'dark'} {...stylex.props(styles.root, ...baselineTheme, (context.args.scheme ?? context.parameters.scheme) === 'light' && lightTheme, (context.args.width ?? context.parameters.width) === 1440 ? styles.wide : styles.narrow)}><Story /></main>],
} satisfies Meta<{ readonly scheme?: 'dark' | 'light'; readonly width?: 440 | 1440 }>
export default meta
type Story = StoryObj<typeof meta>

export const scenarioExemptions = [
  'CompositionFrames:contract-fixture',
  'CompositionFramesDark1440:contract-fixture',
  'CompositionFramesLight1440:contract-fixture',
  'CompositionFramesLight440:contract-fixture',
  'DockDimensions:contract-fixture',
  'DockDimensionsDark1440:contract-fixture',
  'DockDimensionsLight1440:contract-fixture',
  'DockDimensionsLight440:contract-fixture',
  'Inbox:contract-fixture',
  'InboxDark1440:contract-fixture',
  'InboxLight1440:contract-fixture',
  'InboxLight440:contract-fixture',
  'ScrollAttention:contract-fixture',
  'ScrollAttentionDark1440:contract-fixture',
  'ScrollAttentionLight1440:contract-fixture',
  'ScrollAttentionLight440:contract-fixture',
]

export const DockDimensions: Story = {
  render: function DockDimensionsExample() {
    const [expanded, setExpanded] = React.useState(false)
    return <NeedsYouBar requests={[needsYouRequest]} expanded={expanded} onToggle={() => setExpanded(value => !value)}><HostPanel request={needsYouRequest} label="Pinned request" onOpenTerminal={fn()} /></NeedsYouBar>
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const dock = canvas.getByRole('complementary', { name: 'Needs you' })
    const toggle = within(dock).getByRole('button', { name: /waiting on you/ })
    const assertDockScale = async () => {
      await expect(parseFloat(getComputedStyle(toggle).fontSize)).toBe(12)
      await expect(getComputedStyle(toggle).fontWeight).toBe('500')
      await expect(toggle.getBoundingClientRect().height).toBe(24)
      for (const label of toggle.querySelectorAll('span')) await expect(parseFloat(getComputedStyle(label).fontSize)).toBe(12)
    }
    await assertDockScale()
    const previousStyle = toggle.getAttribute('style')
    try {
      toggle.style.height = 'auto'
      toggle.style.paddingBlock = '0.25rem'
      await expect(assertDockScale()).rejects.toThrow()
    } finally {
      if (previousStyle === null) toggle.removeAttribute('style')
      else toggle.setAttribute('style', previousStyle)
    }
    await assertDockScale()
    await expect(canvas.queryByRole('region', { name: 'Pinned request' })).toBeNull()
    await userEvent.click(toggle)
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')
    await expect(within(canvas.getByRole('region', { name: 'Pinned request' })).getByRole('button', { name: 'Open terminal' })).toBeVisible()
    await expect(toggle).toHaveFocus()
    await assertBlockGap(dock)
    await assertKeyboardRing(toggle)
    await assertDockScale()
    await userEvent.keyboard('{Enter}')
    await expect(canvas.queryByRole('region', { name: 'Pinned request' })).toBeNull()
    await expect(toggle).toHaveFocus()
  },
}
export const DockDimensionsLight440: Story = { ...DockDimensions, parameters: { scheme: 'light', width: 440 } }
export const DockDimensionsDark1440: Story = { ...DockDimensions, parameters: { scheme: 'dark', width: 1440 } }
export const DockDimensionsLight1440: Story = { ...DockDimensions, parameters: { scheme: 'light', width: 1440 } }

const onShowInThread = fn(), onOpenTerminal = fn()
export const Inbox: Story = {
  render: () => <>
    <NeedsYouBar requests={needsYouRequests} expanded={false} onToggle={fn()} />
    <NeedsYouInbox requests={needsYouRequests} onShowInThread={onShowInThread} onOpenTerminal={onOpenTerminal}>{id => <p>Host body · {id}</p>}</NeedsYouInbox>
    <NeedsYouInbox ariaLabel="Needs you inbox without terminal" requests={[needsYouRequest]} onShowInThread={fn()} />
  </>,
  play: async ({ canvasElement }) => {
    onShowInThread.mockClear()
    onOpenTerminal.mockClear()
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('button', { name: /^orbit is waiting on you · Where should the build report be delivered\? · \+1/ })).toBeVisible()
    const inbox = canvas.getByRole('region', { name: 'Needs you inbox' })
    const entries = inbox.querySelectorAll<HTMLDetailsElement>('details')
    await expect(entries).toHaveLength(2)
    for (const entry of entries) {
      await expect(entry).not.toHaveAttribute('open')
      const summary = entry.querySelector('summary')!
      await expect(getComputedStyle(summary).minHeight).toBe('24px')
      await expect(summary.getBoundingClientRect().height).toBeGreaterThanOrEqual(24)
      const label = summary.querySelector<HTMLElement>('[data-needs-you-label]')!
      const chevron = summary.querySelector<HTMLElement>('[data-needs-you-chevron]')!
      await expect(getComputedStyle(label).webkitLineClamp).toBe('2')
      await expect(label.getBoundingClientRect().height).toBeLessThanOrEqual(parseFloat(getComputedStyle(label).lineHeight) * 2 + 1)
      await expect(chevron.querySelector('svg')).toHaveAttribute('data-icon', 'chevron-right')
      await expect(chevron.getBoundingClientRect().width).toBe(16)
      await expect(getComputedStyle(chevron).transform).toBe('matrix(1, 0, 0, 1, 0, 0)')
      if (matchMedia('(prefers-reduced-motion: reduce)').matches) await expect(getComputedStyle(chevron).transitionDuration).toBe('0s')
    }
    await assertBlockGap(inbox)
    await expect(within(inbox).getByText('Host body · ask-release-2')).not.toBeVisible()
    await userEvent.click(entries[1]!.querySelector('summary')!)
    await expect(entries[1]).toHaveAttribute('open')
    await expect(within(entries[1]!).getByText('Host body · ask-release-2')).toBeVisible()
    await expect(within(entries[0]!).getByText('Host body · ask-release-1')).not.toBeVisible()
    await assertBlockGap(entries[1]!)
    await assertBlockGap(entries[1]!.querySelector('div')!)
    await waitFor(() => expect(getComputedStyle(entries[1]!.querySelector('[data-needs-you-chevron]')!).transform).toBe('matrix(0, 1, -1, 0, 0, 0)'))
    await userEvent.tab()
    const summary = entries[1]!.querySelector('summary')!
    summary.focus()
    await expect(summary).toHaveFocus()
    await expect(getComputedStyle(summary).outlineStyle).toBe('solid')
    await expect(getComputedStyle(summary).outlineWidth).toBe('2px')
    await userEvent.click(within(entries[1]!).getByRole('button', { name: 'Show in thread' }))
    await expect(onShowInThread).toHaveBeenCalledWith('ask-release-2')
    await assertKeyboardRing(within(entries[1]!).getByRole('button', { name: 'Show in thread' }))
    await userEvent.click(within(entries[1]!).getByRole('button', { name: 'Open terminal' }))
    await expect(onOpenTerminal).toHaveBeenCalledWith('ask-release-2')
    await assertKeyboardRing(within(entries[1]!).getByRole('button', { name: 'Open terminal' }))
    const plain = canvas.getByRole('region', { name: 'Needs you inbox without terminal' })
    await userEvent.click(plain.querySelector('summary')!)
    await expect(within(plain).getByRole('button', { name: 'Show in thread' })).toBeVisible()
    await expect(within(plain).queryByRole('button', { name: 'Open terminal' })).toBeNull()
  },
}
export const InboxLight440: Story = { ...Inbox, parameters: { scheme: 'light', width: 440 } }
export const InboxDark1440: Story = { ...Inbox, parameters: { scheme: 'dark', width: 1440 } }
export const InboxLight1440: Story = { ...Inbox, parameters: { scheme: 'light', width: 1440 } }

export const CompositionFrames: Story = { render: () => <AttentionComposition />, play: async ({ canvasElement }) => {
  await assertAttentionComposition(canvasElement)
  const cards = canvasElement.querySelectorAll<HTMLElement>('[data-fractal-card]')
  await expect(cards).toHaveLength(2)
  const nested = cards[0]!.cloneNode(false) as HTMLElement
  cards[0]!.append(nested)
  try { await expect(assertAttentionComposition(canvasElement)).rejects.toThrow() } finally { nested.remove() }
  const glyph = canvasElement.querySelector<SVGElement>('aside[data-fractal-card] svg[data-icon="question"]')!
  const slot = glyph.parentElement!
  glyph.remove()
  try { await expect(assertAttentionComposition(canvasElement)).rejects.toThrow() } finally { slot.append(glyph) }
  await assertAttentionComposition(canvasElement)
} }
export const CompositionFramesLight440: Story = { ...CompositionFrames, parameters: { scheme: 'light', width: 440 } }
export const CompositionFramesDark1440: Story = { ...CompositionFrames, parameters: { scheme: 'dark', width: 1440 } }
export const CompositionFramesLight1440: Story = { ...CompositionFrames, parameters: { scheme: 'light', width: 1440 } }

export const ScrollAttention: Story = { render: () => <AttentionComposition scrollDriven />, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const scroll = canvasElement.querySelector<HTMLElement>('[data-needs-you-scroll]')!
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: 'Needs you' })).toBeNull())
  scroll.scrollTo({ top: scroll.scrollHeight })
  await waitFor(() => expect(canvas.getByRole('complementary', { name: 'Needs you' })).toBeVisible())
  scroll.scrollTo({ top: 0 })
  await waitFor(() => expect(canvas.queryByRole('complementary', { name: 'Needs you' })).toBeNull())
} }
export const ScrollAttentionLight440: Story = { ...ScrollAttention, parameters: { scheme: 'light', width: 440 } }
export const ScrollAttentionDark1440: Story = { ...ScrollAttention, parameters: { scheme: 'dark', width: 1440 } }
export const ScrollAttentionLight1440: Story = { ...ScrollAttention, parameters: { scheme: 'light', width: 1440 } }

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', padding: s.xl, display: 'flex', flexDirection: 'column', gap: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  narrow: { width: g.specimenNarrow, maxWidth: '100%' },
  wide: { width: g.specimenWide, maxWidth: '100%' },
  scroll: { maxHeight: g.transcriptFrameHeight, overflowY: 'auto', maxWidth: g.lane },
  scrollSpace: { height: '200vh' },
  panel: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.sm, minWidth: 0 },
  prompt: { margin: 0 },
  hostButton: { paddingBlock: s.xs, paddingInline: s.md, color: ink.fg, backgroundColor: surface.rowActive, borderWidth: 0, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer' },
})
