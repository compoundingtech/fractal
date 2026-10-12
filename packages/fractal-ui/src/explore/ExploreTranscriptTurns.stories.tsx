import * as React from 'react'
import { createPortal } from 'react-dom'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useArgs } from 'storybook/preview-api'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button, Disclosure, DisclosurePanel, Tooltip, TooltipTrigger } from 'react-aria-components'
import { useHover } from 'react-aria'
import { ThreadPrimitive, useAuiState } from '@assistant-ui/react'
import { AgentMessage } from '../assistant-ui/AgentMessage'
import { AgentMessage as TranscriptAnswer } from '../assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from '../assistant-ui/EmbraceRuntime'
import type { TextItem } from '../assistant-ui/embrace-data/model'
import { WorkLogV1 } from '../assistant-ui/taste/WorkLogV1'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { lightTheme } from '../assistant-ui/composition-theme'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { VariantPicker } from './VariantPicker'
import { replyItems, turnBlocks, workTurn, type TurnBlock } from './turns-fixture'
import { turnsGeometry as geometry } from './turns-tokens.stylex'

const dimensions = {
  T: [{ id: 'T1', description: 'Inputs are sender cards; the owning seat replies in unnamed prose.' }, { id: 'T2', description: 'Inputs align right in 75% bubbles; seat prose aligns left.' }, { id: `T${3}`, description: '24-wide sender glyph gutter; names only when the sender changes.' }],
  K: [{ id: 'K1', description: 'One card shape with kind word and Verified / Unverified.' }, { id: 'K2', description: 'Person accent rail, agent neutral rail, faint dashed relay / system rails.' }, { id: 'K3', description: 'Relay / system become expandable one-line summaries; person / agent stay cards.' }],
  M: [{ id: 'M1', description: 'Time in input header; seat time / copy overlay top-right on reveal.' }, { id: 'M2', description: 'Time in the left first-line gutter; copy overlay top-right on reveal.' }, { id: 'M3', description: 'Time divider for inputs after ≥5 minutes or a day change; exact-time tooltip.' }, { id: 'M4', description: 'Time inline after the last line; copy overlay top-right on reveal.' }],
} as const
interface ExploreArgs { turn: `T${1 | 2 | 3}`; kind: 'K1' | 'K2' | 'K3'; meta: 'M1' | 'M2' | 'M3' | 'M4'; compareT: boolean; compareK: boolean; compareM: boolean; scheme: 'dark' | 'light'; width: 1440 | 900 }
const glyphs = { Person: '○', Agent: '◇', Relay: '↪', System: '·', Seat: '✦' } as const
const clock = (at: string) => new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', hour: 'numeric', minute: '2-digit' }).format(new Date(at))
const exact = (at: string) => `${at.slice(0, 10)} · ${clock(at)} UTC`
const isDivider = (block: TurnBlock, previous?: TurnBlock) => block.kind !== 'Seat' && (previous === undefined || Date.parse(block.at) - Date.parse(previous.at) >= 300000 || block.at.slice(0, 10) !== previous.at.slice(0, 10))

function ExploreAnswer({ showMeta = false }: { readonly showMeta?: boolean }) {
  const item = useAuiState(state => state.message.metadata.custom.item) as TextItem | undefined
  if (item === undefined || item.role !== 'assistant') return null
  return <TranscriptAnswer item={item as TextItem & { role: 'assistant' }} showMeta={showMeta} />
}
// A new component closure per Block render remounted the prose after setTail, leaving
// its timestamp portal attached to a detached paragraph and dropping descendant focus.
const answerComponents = { Message: ExploreAnswer }
const metaControlComponents = { Message: () => <ExploreAnswer showMeta /> }

function FocusRemountControl() {
  const [generation, setGeneration] = React.useState(0)
  // Deliberate control: changing the component identity drops its focused node.
  const UnstableCopy = () => <Button {...stylex.props(styles.control)}>Remounting Copy control</Button>
  return <div data-control-remount data-generation={generation} onPointerDown={() => setGeneration(value => value + 1)}><UnstableCopy /></div>
}

/** Variant-only projection of production rows. No change to runtime facts or message bodies. */
function Block({ block, previous, turn, kind, meta }: Pick<ExploreArgs, 'turn' | 'kind' | 'meta'> & { block: TurnBlock; previous?: TurnBlock }) {
  const root = React.useRef<HTMLDivElement>(null)
  const { hoverProps, isHovered } = useHover({})
  const [tail, setTail] = React.useState<Element | null>(null)
  const [tapped, setTapped] = React.useState(false)
  const [copied, setCopied] = React.useState(false)
  const [expanded, setExpanded] = React.useState(false)
  const seat = block.kind === 'Seat'
  const collapsedKind = kind === 'K3' && (block.kind === 'Relay' || block.kind === 'System')
  React.useLayoutEffect(() => {
    const element = root.current
    if (element === null) return
    // The production message body is a scroll region; these short, fully visible specimens
    // do not need its tab stop. Copy is the sole metadata tab stop in every variant.
    const adoptContent = () => {
      for (const region of element.querySelectorAll<HTMLElement>('[role="region"][tabindex]')) region.tabIndex = -1
      const paragraphs = element.querySelectorAll('[data-turn-content] p')
      setTail(paragraphs.item(paragraphs.length - 1))
    }
    adoptContent()
    const observer = new MutationObserver(adoptContent)
    observer.observe(element, { childList: true, subtree: true })
    return () => observer.disconnect()
  }, [expanded, meta, block.id])
  const time = <time data-turn-time dateTime={block.at} title={exact(block.at)} {...stylex.props(styles.time)}>{clock(block.at)}</time>
  const content = seat
    ? <ThreadPrimitive.Unstable_MessageById messageId={block.id} components={answerComponents} />
    : <AgentMessage from={block.sender} via={{ _tag: 'SmallTalk' }} body={{ _tag: 'Full', markdown: block.text }} subject={block.subject} variant={block.kind === 'Person' ? { _tag: 'Human' } : { _tag: 'Incoming' }} />
  const header = (!seat || turn === `T${3}`) && (turn !== `T${3}` || previous?.sender !== block.sender)
    ? <header data-turn-header {...stylex.props(styles.header)}><strong>{block.sender}</strong>{kind === 'K1' && !seat ? <span>{block.kind} · {block.verified ? 'Verified' : 'Unverified'}</span> : null}</header> : null
  const body = <div data-turn-content {...stylex.props(styles.content)}>{content}</div>
  return <div ref={root} {...hoverProps} data-turn-block={block.id} data-turn-kind={block.kind} data-turn-structure={turn} data-kind-cue={kind} data-meta={meta} data-hovered={isHovered || undefined} data-tapped={tapped || undefined} title={meta === 'M3' ? exact(block.at) : undefined} onPointerDown={event => { if (event.pointerType === 'touch') setTapped(value => !value) }} {...stylex.props(styles.block, !seat && styles.input, turn === 'T2' && !seat && styles.bubble, turn === `T${3}` && styles.senderGutter, kind === 'K2' && block.kind === 'Person' && styles.humanRail, kind === 'K2' && block.kind === 'Agent' && styles.agentRail, kind === 'K2' && (block.kind === 'Relay' || block.kind === 'System') && styles.quietRail, collapsedKind && styles.quiet)}>
    {turn === `T${3}` ? <span aria-hidden="true" {...stylex.props(styles.glyph)}>{glyphs[block.kind]}</span> : null}
    {meta === 'M3' && isDivider(block, previous) ? <div data-time-divider {...stylex.props(styles.divider)}>── {time} ──</div> : null}
    {collapsedKind ? <Disclosure isExpanded={expanded} onExpandedChange={setExpanded}><Button slot="trigger" {...stylex.props(styles.summary)}><span data-input-kind-icon aria-hidden="true">{glyphs[block.kind]} {expanded ? '▾' : '▸'}</span><strong data-input-sender>{block.sender}</strong><span data-input-title {...stylex.props(styles.summaryText)}><span data-input-title-run {...stylex.props(styles.summaryRun)}>{block.text.split('\n')[0]}</span></span>{meta === 'M4' && !expanded ? <span data-meta-time {...stylex.props(styles.trailing)}>{' '}{time}</span> : null}</Button><DisclosurePanel>{header}{body}</DisclosurePanel></Disclosure> : <>{header}{body}</>}
    {meta === 'M1' || meta === 'M2' ? <div data-meta-time data-affordance={meta === 'M2' || seat ? 'reveal' : 'always'} {...stylex.props(styles.timeOverlay, meta === 'M2' && styles.gutterTime)}>{time}</div> : null}
    {meta === 'M4' && tail !== null && (!collapsedKind || expanded) ? createPortal(<span data-meta-time {...stylex.props(styles.trailing)}>{' '}{time}</span>, tail) : null}
    <TooltipTrigger delay={0}><Button data-copy data-affordance="reveal" aria-label={`Copy ${block.id}`} onPress={async () => { await navigator.clipboard.writeText(block.text); setCopied(true) }} {...stylex.props(styles.copy)}>{copied ? 'Copied' : 'Copy'}</Button><Tooltip {...stylex.props(styles.tooltip)}>{meta === 'M3' ? exact(block.at) : 'Copy message'}</Tooltip></TooltipTrigger>
  </div>
}

function Lane({ args }: { args: ExploreArgs }) {
  return <section data-lane data-turn-variant={args.turn} data-kind-variant={args.kind} data-meta-variant={args.meta} data-scheme={args.scheme} data-width={args.width} aria-label={`${args.turn} ${args.kind} ${args.meta} · ${args.scheme} · ${args.width}`} {...stylex.props(styles.lane)}>
    <h2 {...stylex.props(styles.laneTitle)}>{args.turn} · {args.kind} · {args.meta} / {args.scheme} / {args.width}</h2>
    <EmbraceRuntimeProvider options={{ messages: replyItems, isRunning: false }}><ThreadPrimitive.Root {...stylex.props(styles.thread)}>{turnBlocks.map((block, index) => <React.Fragment key={block.id}><Block block={block} previous={turnBlocks[index - 1]} turn={args.turn} kind={args.kind} meta={args.meta} />{index === 7 ? <div data-work-fixture {...stylex.props(styles.work)}><WorkLogV1 turn={workTurn} /></div> : null}</React.Fragment>)}</ThreadPrimitive.Root></EmbraceRuntimeProvider>
  </section>
}
function Explorer(args: ExploreArgs) {
  const [, updateArgs] = useArgs<ExploreArgs>()
  const turns = args.compareT ? dimensions.T.map(choice => choice.id) : [args.turn]
  const kinds = args.compareK ? dimensions.K.map(choice => choice.id) : [args.kind]
  const metas = args.compareM ? dimensions.M.map(choice => choice.id) : [args.meta]
  return <main data-turns-explorer data-token-probe {...stylex.props(...baselineTheme, args.scheme === 'light' && lightTheme, styles.root, args.width === 1440 ? styles.wide : styles.narrow)}>
    <style>{projectionCss}</style>
    <h1 {...stylex.props(styles.title)}>Transcript turns</h1><p {...stylex.props(styles.explanation)}>Inputs trigger the turn. The owning seat replies. Compare language, input cues and metadata without adding a footer.</p>
    <VariantPicker label="Turn structure" choices={dimensions.T} value={args.turn} compare={args.compareT} onChange={turn => updateArgs({ turn: turn as ExploreArgs['turn'] })} onCompare={compareT => updateArgs({ compareT })} />
    <VariantPicker label="Input kind" choices={dimensions.K} value={args.kind} compare={args.compareK} onChange={kind => updateArgs({ kind: kind as ExploreArgs['kind'] })} onCompare={compareK => updateArgs({ compareK })} />
    <VariantPicker label="Time and copy" choices={dimensions.M} value={args.meta} compare={args.compareM} onChange={meta => updateArgs({ meta: meta as ExploreArgs['meta'] })} onCompare={compareM => updateArgs({ compareM })} />
    <div {...stylex.props(styles.controls)}>{(['dark', 'light'] as const).map(scheme => <Button key={scheme} aria-pressed={args.scheme === scheme} onPress={() => updateArgs({ scheme })} {...stylex.props(styles.control)}>{scheme}</Button>)}{([1440, 900] as const).map(width => <Button key={width} aria-pressed={args.width === width} onPress={() => updateArgs({ width })} {...stylex.props(styles.control)}>{width} lane</Button>)}</div>
    <div {...stylex.props(styles.comparisons)}>{turns.flatMap(turn => kinds.flatMap(kind => metas.map(meta => <Lane key={`${turn}-${kind}-${meta}`} args={{ ...args, turn, kind, meta } as ExploreArgs} />)))}</div>
    <section aria-label="Deliberately failing controls" data-controls {...stylex.props(styles.negativeControls)}>
      <div data-control-height {...stylex.props(styles.block)}><p>Reply with a legacy footer.</p><div data-meta-time {...stylex.props(styles.footerControl)}><time>6:32 AM</time></div></div>
      <div data-control-owner {...stylex.props(styles.block)}><p>Owning reply.</p><time data-turn-time {...stylex.props(styles.wrongOwner)}>6:32 AM</time></div><div data-control-next {...stylex.props(styles.block)}><p>Next input.</p></div>
      <div data-control-focus {...stylex.props(styles.block)}><span data-affordance="reveal" {...stylex.props(styles.hoverOnly)}>Hover-only time</span><Button {...stylex.props(styles.control)}>Control copy</Button></div>
      <div data-control-style>Unstyled token probe</div>
      <div data-control-transition data-turn-variant="T1" data-kind-variant="K1" data-meta-variant="M1" data-scheme="dark" data-width="1440" data-lane-count="1"><Button {...stylex.props(styles.control)}>Inert transition control</Button><Button aria-expanded={false} {...stylex.props(styles.control)}>Inert disclosure control</Button></div>
      <EmbraceRuntimeProvider options={{ messages: replyItems, isRunning: false }}><ThreadPrimitive.Root data-control-production-meta><ThreadPrimitive.Unstable_MessageById messageId={replyItems[0]!.id} components={metaControlComponents} /></ThreadPrimitive.Root></EmbraceRuntimeProvider>
      <div data-control-k3 {...stylex.props(styles.quietInputControl)}><span data-input-title>Quiet summary with no sender or kind cue</span></div>
      <div data-control-k3-stacked {...stylex.props(styles.stackedInputControl)}><span data-input-title><span data-input-title-run>Sender<br />Wrapped title</span></span></div>
      <FocusRemountControl />
      <div data-control-missing-time>Reply without its own timestamp.</div>
    </section>
  </main>
}
// Scoped variant projections keep the production message / Markdown / work-log renderers.
// Every geometry value comes from composition tokens (including the locked hierarchy step).
const projectionCss = `
[data-turns-explorer] [data-turn-content] article { padding: 0; border: 0; background: transparent; }
[data-turns-explorer] [data-turn-content] article > header { display: none; }
[data-turns-explorer] [data-turn-content] article h3 { margin-top: 0; }
[data-turns-explorer] [data-turn-content] [role="region"] { max-height: none; overflow: visible; }
[data-turns-explorer] [data-turn-content] p:last-child { margin-bottom: 0; }
[data-turns-explorer] [data-meta="M4"] [data-turn-content] p:last-child { padding-inline-end: ${geometry.trailingWidth}; }
[data-turns-explorer] [data-turn-block] [data-affordance="reveal"] { opacity: 0; }
[data-turns-explorer] [data-turn-block][data-hovered] [data-affordance="reveal"],
[data-turns-explorer] [data-turn-block]:focus-within [data-affordance="reveal"],
[data-turns-explorer] [data-turn-block][data-tapped] [data-affordance="reveal"] { opacity: 1; }
`
const styles = stylex.create({
  root: { padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, boxSizing: 'border-box' },
  wide: { width: geometry.wide }, narrow: { width: geometry.narrow },
  title: { margin: 0, fontSize: t.headingSize, lineHeight: t.headingLeading }, explanation: { color: ink.fgMuted, marginBlock: s.md },
  controls: { display: 'flex', gap: s.md, marginBlock: s.md }, control: { padding: s.xs, color: ink.fg, backgroundColor: surface.rowActive, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, cursor: 'pointer', fontFamily: t.fontSans, ':focus-visible': { outlineColor: accent.primary, outlineStyle: 'solid', outlineWidth: g.focusRing } },
  comparisons: { display: 'flex', alignItems: 'start', gap: s.section, overflowX: 'auto' },
  lane: { flex: '0 0 auto', width: '100%', minWidth: 0, boxSizing: 'border-box' }, laneTitle: { marginBlock: s.lg, fontSize: t.metaSize, color: ink.fgMuted, fontWeight: t.weightMedium },
  thread: { width: g.lane, maxWidth: '100%', marginInline: 'auto', paddingLeft: geometry.gutter, boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.section },
  block: { position: 'relative', minWidth: 0, color: ink.fg, fontSize: t.bodySize, lineHeight: t.bodyLeading },
  input: { paddingBlock: s.md, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.raised },
  bubble: { alignSelf: 'end', maxWidth: '75%', borderRadius: r.lg }, senderGutter: { marginLeft: geometry.glyph }, glyph: { position: 'absolute', right: '100%', top: s.md, width: geometry.glyph, color: ink.fgMuted, textAlign: 'center' },
  humanRail: { borderLeftWidth: g.focusRing, borderLeftColor: accent.primary }, agentRail: { borderLeftWidth: g.focusRing, borderLeftColor: ink.fgMuted }, quietRail: { borderLeftWidth: g.focusRing, borderLeftColor: ink.fgFaint, borderLeftStyle: 'dashed' },
  quiet: { paddingBlock: 0, borderWidth: 0, backgroundColor: surface.transparent },
  header: { display: 'flex', alignItems: 'center', gap: s.sm, minHeight: t.metaLeading, paddingRight: geometry.metaRoom, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, marginBottom: s.xs2 },
  content: { minWidth: 0 },
  time: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, whiteSpace: 'nowrap', fontVariantNumeric: 'tabular-nums' },
  timeOverlay: { position: 'absolute', top: s.md, right: g.gutter40, lineHeight: t.metaLeading }, gutterTime: { right: '100%', marginRight: s.md, top: s.md },
  trailing: { display: 'inline-block', width: geometry.trailingWidth, marginInlineEnd: geometry.trailingOverlap, whiteSpace: 'nowrap' },
  copy: { position: 'absolute', top: s.xs, right: 0, minHeight: g.controlSm, padding: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.raised, color: ink.fgMuted, cursor: 'pointer', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  divider: { position: 'absolute', top: geometry.lift, left: 0, right: 0, textAlign: 'center', color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  summary: { display: 'flex', flexWrap: 'nowrap', alignItems: 'center', gap: s.sm, width: '100%', padding: 0, paddingRight: geometry.metaRoom, minHeight: g.toolRow, lineHeight: g.toolRow, whiteSpace: 'nowrap', borderWidth: 0, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, textAlign: 'left', cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, summaryText: { flex: '1 1 auto', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 },
  summaryRun: { display: 'inline-block', whiteSpace: 'nowrap' },
  stackedInputControl: { width: geometry.metaRoom, fontSize: t.uiSize, lineHeight: g.toolRow },
  quietInputControl: { color: ink.fgMuted, height: g.iconSm, overflow: 'hidden', fontSize: t.metaSize, lineHeight: t.metaLeading },
  work: { marginTop: s.md }, tooltip: { padding: s.md, backgroundColor: surface.raised, color: ink.fg, fontSize: t.metaSize, borderRadius: r.sm },
  negativeControls: { marginTop: s.section, display: 'flex', flexDirection: 'column', gap: s.section }, footerControl: { height: g.toolRow }, wrongOwner: { position: 'absolute', top: '100%', marginTop: s.section, fontSize: t.metaSize }, hoverOnly: { opacity: { default: 0, ':hover': 1 } },
})

const meta = { title: 'Fractal UI/Explore/Transcript Turns', component: Explorer, render: Explorer, parameters: { layout: 'fullscreen' }, args: { turn: 'T1', kind: 'K1', meta: 'M1', compareT: false, compareK: false, compareM: false, scheme: 'dark', width: 1440 }, argTypes: { turn: { options: dimensions.T.map(choice => choice.id), control: 'radio' }, kind: { options: dimensions.K.map(choice => choice.id), control: 'radio' }, meta: { options: dimensions.M.map(choice => choice.id), control: 'radio' }, scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [1440, 900], control: 'radio' } } } satisfies Meta<typeof Explorer>
export default meta
type Story = StoryObj<typeof meta>
const distance = (label: DOMRect, block: DOMRect) => Math.max(block.top - label.bottom, label.top - block.bottom, 0)
function reservedMetaHeight(block: HTMLElement) {
  const height = block.getBoundingClientRect().height
  const metadata = Array.from(block.querySelectorAll<HTMLElement>('[data-meta-time], [data-time-divider], [data-copy]'))
  const previous = metadata.map(element => element.style.display)
  metadata.forEach(element => { element.style.display = 'none' })
  const bare = block.getBoundingClientRect().height
  metadata.forEach((element, index) => { element.style.display = previous[index] ?? '' })
  return height - bare
}
async function assertProjection(canvasElement: HTMLElement, expandedBlock?: HTMLElement) {
  await waitFor(() => {
    const lanes = canvasElement.querySelectorAll('[data-lane]')
    expect(lanes.length).toBeGreaterThan(0)
    for (const lane of lanes) {
      expect(lane.querySelectorAll('[data-turn-content] [data-testid="agent-message"]')).toHaveLength(replyItems.length)
      if (lane.getAttribute('data-meta-variant') === 'M4') expect(lane.querySelectorAll('[data-meta-time]')).toHaveLength(turnBlocks.length)
    }
  })
  const styled = getComputedStyle(canvasElement.querySelector('[data-token-probe]')!)
  const styleControl = getComputedStyle(canvasElement.querySelector('[data-control-style]')!)
  await expect(parseFloat(styled.paddingTop)).toBe(24)
  await expect(parseFloat(styleControl.paddingTop)).not.toBe(24)
  await expect(styled.backgroundColor).not.toBe('rgba(0, 0, 0, 0)')
  for (const lane of canvasElement.querySelectorAll('[data-lane]')) {
    await expect(lane.querySelector('[data-turn-content] [data-testid="answer-meta"], [data-turn-content] [data-testid="message-time"], [data-turn-content] footer')).toBeNull()
    for (const block of lane.querySelectorAll<HTMLElement>('[data-kind-cue="K3"]')) {
      const trigger = block.querySelector<HTMLElement>('button[aria-expanded="false"]')
      if (trigger === null) continue
      await expect(getComputedStyle(trigger.querySelector('[data-input-title]')!).color).toBe(getComputedStyle(block).color)
      await expect(trigger.querySelector('[data-input-sender]')!.textContent?.trim().length).toBeGreaterThan(0)
      await expect(trigger.querySelector('[data-input-kind-icon]')!.textContent?.trim().length).toBeGreaterThan(0)
      await expect(trigger.getBoundingClientRect().height).toBeGreaterThanOrEqual(24)
      await expect(trigger.getBoundingClientRect().height).toBeLessThanOrEqual(parseFloat(getComputedStyle(trigger).lineHeight))
      await expect(trigger.querySelector<HTMLElement>('[data-input-title-run]')!.getClientRects()).toHaveLength(1)
    }
  }
  await waitFor(() => expect(canvasElement.querySelector('[data-control-production-meta] [data-testid="answer-meta"]')).not.toBeNull())
  const quietControl = canvasElement.querySelector<HTMLElement>('[data-control-k3]')!
  await expect(getComputedStyle(quietControl.querySelector('[data-input-title]')!).color).not.toBe(styled.color)
  await expect(quietControl.querySelector('[data-input-sender], [data-input-kind-icon]')).toBeNull()
  await expect(quietControl.getBoundingClientRect().height).toBeLessThan(24)
  const stackedControl = canvasElement.querySelector<HTMLElement>('[data-control-k3-stacked]')!
  await expect(stackedControl.getBoundingClientRect().height).toBeGreaterThan(parseFloat(getComputedStyle(stackedControl).lineHeight))
  await expect(stackedControl.querySelector<HTMLElement>('[data-input-title-run]')!.getClientRects().length).toBeGreaterThan(1)
  for (const lane of canvasElement.querySelectorAll('[data-lane]')) {
    const blocks = Array.from(lane.querySelectorAll<HTMLElement>('[data-turn-block]'))
    for (const [index, block] of blocks.entries()) {
      await expect(reservedMetaHeight(block)).toBe(0)
      if (lane.getAttribute('data-meta-variant') === 'M4') await expect(block.querySelectorAll('time[data-turn-time]')).toHaveLength(1)
      const next = blocks[index + 1]
      if (next === undefined) continue
      for (const label of block.querySelectorAll('time[data-turn-time]')) await expect(distance(label.getBoundingClientRect(), block.getBoundingClientRect())).toBeLessThanOrEqual(distance(label.getBoundingClientRect(), next.getBoundingClientRect()))
    }
  }
  await expect(canvasElement.querySelector('[data-control-missing-time]')!.querySelectorAll('time')).toHaveLength(0)
  await expect(reservedMetaHeight(canvasElement.querySelector<HTMLElement>('[data-control-height]')!)).toBeGreaterThan(0)
  const owner = canvasElement.querySelector('[data-control-owner]')!, next = canvasElement.querySelector('[data-control-next]')!, label = owner.querySelector('time')!
  await expect(distance(label.getBoundingClientRect(), owner.getBoundingClientRect())).toBeGreaterThan(distance(label.getBoundingClientRect(), next.getBoundingClientRect()))
  const samples: HTMLElement[] = []
  for (const lane of canvasElement.querySelectorAll('[data-lane]')) samples.push(lane.querySelector<HTMLElement>('[data-turn-kind="Seat"]')!)
  if (expandedBlock !== undefined) samples.push(expandedBlock)
  for (const block of samples) {
    const copy = block.querySelector<HTMLElement>('[data-copy]')!
    const active = canvasElement.ownerDocument.activeElement
    if (active instanceof HTMLElement) active.blur()
    await userEvent.unhover(block)
    await expect(getComputedStyle(copy).opacity).toBe('0')
    copy.focus()
    await waitFor(() => expect(getComputedStyle(copy).opacity).toBe('1'))
    const answer = block.querySelector('[data-testid="agent-message"]')
    block.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    await waitFor(() => expect(block).toHaveAttribute('data-tapped', 'true'))
    await expect(block.querySelector('[data-testid="agent-message"]')).toBe(answer)
    await expect(copy).toHaveFocus()
    block.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    await waitFor(() => expect(block).not.toHaveAttribute('data-tapped'))
    await expect(copy).toHaveFocus()
    const focusOpacity = Array.from(block.querySelectorAll('[data-affordance="reveal"]')).map(element => getComputedStyle(element).opacity)
    copy.blur()
    await userEvent.hover(block)
    await waitFor(() => expect(Array.from(block.querySelectorAll('[data-affordance="reveal"]')).map(element => getComputedStyle(element).opacity)).toEqual(focusOpacity))
    await userEvent.unhover(block)
    block.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    await waitFor(() => expect(getComputedStyle(copy).opacity).toBe('1'))
    block.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    await waitFor(() => expect(getComputedStyle(copy).opacity).toBe('0'))
    const extraStops = Array.from(block.querySelectorAll<HTMLElement>('button, a[href], [tabindex]')).filter(element => element !== copy && !element.hasAttribute('aria-expanded') && element.tabIndex >= 0 && element.getClientRects().length > 0)
    await expect(extraStops).toHaveLength(0)
  }
  const focusControl = canvasElement.querySelector<HTMLElement>('[data-control-focus]')!
  focusControl.querySelector<HTMLElement>('button')!.focus()
  await expect(getComputedStyle(focusControl.querySelector('[data-affordance]')!).opacity).toBe('0')
  const remountControl = canvasElement.querySelector<HTMLElement>('[data-control-remount]')!
  const detachedCopy = remountControl.querySelector<HTMLElement>('button')!
  detachedCopy.focus()
  remountControl.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
  await waitFor(() => expect(detachedCopy.isConnected).toBe(false))
  await expect(canvasElement.ownerDocument.activeElement).not.toBe(detachedCopy)
}
const play: NonNullable<Story['play']> = async ({ canvasElement, step }) => {
  const canvas = within(canvasElement)
  const inert = canvas.getByRole('button', { name: 'Inert transition control' })
  const control = canvasElement.querySelector('[data-control-transition]')!
  await step('Initial geometry and affordances reject footer, ownership, StyleX and hover-only controls', async () => {
    await assertProjection(canvasElement)
  })
  await step('Pick turn structure; inert picker control cannot update the lane', async () => {
    // Normalize comparison through the real controls, not updateArgs, so selected options
    // must actually replace the lane rather than merely changing a selected button.
    for (const name of ['Compare Turn structure', 'Compare Input kind', 'Compare Time and copy']) {
      const toggle = canvas.getByRole('button', { name })
      if (toggle.getAttribute('aria-pressed') === 'true') {
        const before = canvasElement.querySelectorAll('[data-lane]').length
        await userEvent.click(toggle)
        await waitFor(() => expect(canvasElement.querySelectorAll('[data-lane]').length).toBeLessThan(before))
        await assertProjection(canvasElement)
      }
    }
    const turn = canvasElement.querySelector('[data-lane]')!.getAttribute('data-turn-variant') === 'T2' ? `T${3}` : 'T2'
    await userEvent.click(inert)
    await expect(control).not.toHaveAttribute('data-turn-variant', turn)
    await userEvent.click(canvas.getByRole('button', { name: new RegExp(`^${turn}:`) }))
    await waitFor(() => expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-turn-variant', turn))
    await assertProjection(canvasElement)
  })
  await step('Pick K3 input-kind disclosure; inert kind control stays K1', async () => {
    const selected = canvas.getByRole('button', { name: /^K3:/ })
    if (selected.getAttribute('aria-pressed') === 'true') {
      await userEvent.click(canvas.getByRole('button', { name: /^K2:/ }))
      await waitFor(() => expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-kind-variant', 'K2'))
      await assertProjection(canvasElement)
    }
    await userEvent.click(inert)
    await expect(control).not.toHaveAttribute('data-kind-variant', 'K3')
    await userEvent.click(selected)
    await waitFor(() => expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-kind-variant', 'K3'))
    await assertProjection(canvasElement)
  })
  await step('Pick M4 trailing metadata; inert metadata control stays M1', async () => {
    const selected = canvas.getByRole('button', { name: /^M4:/ })
    if (selected.getAttribute('aria-pressed') === 'true') {
      await userEvent.click(canvas.getByRole('button', { name: /^M2:/ }))
      await waitFor(() => expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-meta-variant', 'M2'))
      await assertProjection(canvasElement)
    }
    await userEvent.click(inert)
    await expect(control).not.toHaveAttribute('data-meta-variant', 'M4')
    await userEvent.click(selected)
    await waitFor(() => expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-meta-variant', 'M4'))
    await assertProjection(canvasElement)
  })
  await step('Compare metadata creates four lanes; inert compare control stays one', async () => {
    await userEvent.click(inert)
    await expect(control).not.toHaveAttribute('data-lane-count', '4')
    await userEvent.click(canvas.getByRole('button', { name: 'Compare Time and copy' }))
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-lane]')).toHaveLength(dimensions.M.length))
    await assertProjection(canvasElement)
  })
  await step('Switch theme; inert theme control never changes paint', async () => {
    const initial = canvasElement.querySelector('[data-lane]')!.getAttribute('data-scheme')
    const scheme = initial === 'dark' ? 'light' : 'dark'
    const paint = getComputedStyle(canvasElement.querySelector('[data-token-probe]')!).backgroundColor
    const controlPaint = getComputedStyle(control).backgroundColor
    await userEvent.click(inert)
    await expect(getComputedStyle(control).backgroundColor).toBe(controlPaint)
    await userEvent.click(canvas.getByRole('button', { name: scheme, exact: true }))
    await waitFor(() => {
      expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-scheme', scheme)
      expect(getComputedStyle(canvasElement.querySelector('[data-token-probe]')!).backgroundColor).not.toBe(paint)
    })
    await assertProjection(canvasElement)
  })
  await step('Switch lane width; inert width control never resizes', async () => {
    const width = canvasElement.querySelector('[data-lane]')!.getAttribute('data-width') === '1440' ? 900 : 1440
    const controlWidth = control.getBoundingClientRect().width
    await userEvent.click(inert)
    await expect(control.getBoundingClientRect().width).toBe(controlWidth)
    await userEvent.click(canvas.getByRole('button', { name: `${width} lane`, exact: true }))
    await waitFor(() => {
      expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-width', `${width}`)
      expect(canvasElement.querySelector('[data-token-probe]')!.getBoundingClientRect().width).toBe(width)
    })
    await assertProjection(canvasElement)
  })
  await step('Expand K3 with M4 time inside its final paragraph; inert disclosure stays closed', async () => {
    const disclosureControl = canvas.getByRole('button', { name: 'Inert disclosure control' })
    await userEvent.click(disclosureControl)
    await expect(disclosureControl).toHaveAttribute('aria-expanded', 'false')
    await expect(control.querySelector('[data-turn-content]')).toBeNull()
    const lane = canvasElement.querySelector('[data-lane][data-meta-variant="M4"]')!
    const block = lane.querySelector<HTMLElement>('[data-turn-block="relay-hold"]')!
    const trigger = block.querySelector<HTMLElement>('button[aria-expanded]')!
    await expect(trigger).toHaveAttribute('aria-expanded', 'false')
    await userEvent.click(trigger)
    await waitFor(() => {
      expect(trigger).toHaveAttribute('aria-expanded', 'true')
      expect(block.querySelector('[data-turn-content] p [data-meta-time] time[data-turn-time]')).not.toBeNull()
    })
    await assertProjection(canvasElement, block)
  })
}
export const Exploration: Story = {}
export const ExplorationProbe: Story = { play }
export const AllStates: Story = { args: { compareT: true, compareK: true, compareM: true }, play }
export const MetaM1: Story = { args: { meta: 'M1' }, play }
export const MetaM2: Story = { args: { meta: 'M2' }, play }
export const MetaM3: Story = { args: { meta: 'M3' }, play }
export const MetaM4: Story = { args: { meta: 'M4' }, play }
export const Light900: Story = { args: { scheme: 'light', width: 900, compareM: true }, play }
export const Dark900: Story = { args: { scheme: 'dark', width: 900, compareM: true }, play }
export const Light1440: Story = { args: { scheme: 'light', width: 1440, compareM: true }, play }
