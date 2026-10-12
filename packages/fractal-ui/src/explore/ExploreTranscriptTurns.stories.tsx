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
import { SeatProse } from '../assistant-ui/composition/Transcript'
import { Icon, type IconName } from '../assistant-ui/composition/Icons'
import { EmbraceRuntimeProvider } from '../assistant-ui/EmbraceRuntime'
import type { TextItem } from '../assistant-ui/embrace-data/model'
import { WorkLogV1 } from '../assistant-ui/taste/WorkLogV1'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { lightTheme } from '../assistant-ui/composition-theme'
import { accentVars as accent, borderVars as border, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { VariantPicker } from './VariantPicker'
import { replyItems, turnBlocks, workTurn, type TurnBlock } from './turns-fixture'
import { turnsGeometry as geometry } from './turns-tokens.stylex'

const dimensions = {
  T: [{ id: 'T1', description: 'Inputs are sender Cards; the owning seat replies in unnamed Prose.' }, { id: 'T2', description: 'Inputs align right in 75% bubbles (full width below 640px); seat Prose aligns left.' }, { id: `T${3}`, description: 'Composition glyph slot and nesting-step gutter; names only when the sender changes.' }],
  K: [{ id: 'K1', description: 'One Card shape with kind word; verified is silent, Unverified is explicit.' }, { id: 'K2', description: 'Person accent rail, agent neutral rail, muted dashed relay / system rails.' }, { id: 'K3', description: 'Relay / system become expandable one-line summaries; person / agent stay cards.' }],
  M: [{ id: 'M1', description: 'Time in input header; a reserved 16px time / Copy row above each seat run, revealed on hover, focus or tap.' }, { id: 'M2', description: 'Time in the left first-line gutter; copy overlay top-right on reveal.' }, { id: 'M3', description: 'Time divider for inputs after ≥5 minutes or a day change; exact-time tooltip.' }, { id: 'M4', description: 'Time inline after the last line; copy overlay top-right on reveal.' }],
} as const
interface ExploreArgs { turn: `T${1 | 2 | 3}`; kind: 'K1' | 'K2' | 'K3'; meta: 'M1' | 'M2' | 'M3' | 'M4'; compareT: boolean; compareK: boolean; compareM: boolean; scheme: 'dark' | 'light'; width: 1440 | 900; recommended: boolean; probe: boolean }
const glyphs: Record<TurnBlock['kind'], IconName> = { Person: 'person', Agent: 'message', Relay: 'arrow-right', System: 'gear', Seat: 'thought' }
const clockFormat = new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: '2-digit' })
const exactFormat = new Intl.DateTimeFormat('en-US', { dateStyle: 'full', timeStyle: 'long' })
const dayFormat = new Intl.DateTimeFormat('en-US', { dateStyle: 'short' })
const clock = (at: string) => clockFormat.format(new Date(at))
const exact = (at: string) => exactFormat.format(new Date(at))
const isDivider = (block: TurnBlock, previous?: TurnBlock) => block.kind !== 'Seat' && (previous === undefined || Date.parse(block.at) - Date.parse(previous.at) >= 300000 || dayFormat.format(new Date(block.at)) !== dayFormat.format(new Date(previous.at)))

function ExploreAnswer({ showMeta = false }: { readonly showMeta?: boolean }) {
  const item = useAuiState(state => state.message.metadata.custom.item) as TextItem | undefined
  if (item === undefined || item.role !== 'assistant') return null
  return <SeatProse item={item as TextItem & { role: 'assistant' }} showMeta={showMeta} />
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

// A six-segment chain and a 50-character leaf: the identity must absorb all narrowing while the metadata stays whole.
const longIdentities = ['Fleet.Infrastructure.WorkspaceCoordinator.ReviewBridge.RetainedMirror.Review', 'Workspace.ReviewBridgeRetainedMirrorTypedChoicesCoordinator'] as const
const controlMeta = (yields: boolean) => <span data-input-meta {...stylex.props(styles.inputMeta, yields && styles.inputMetaYields)}><span data-input-kind {...stylex.props(yields && styles.inputKindYields)}>Agent · Unverified</span><span data-meta-time><time>11:52 AM</time></span></span>

/** Variant-only projection of production rows. No change to runtime facts or message bodies. */
function Block({ block, previous, turn, kind, meta, marker = false }: Pick<ExploreArgs, 'turn' | 'kind' | 'meta'> & { block: TurnBlock; previous?: TurnBlock; marker?: boolean }) {
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
    const adoptContent = () => {
      for (const region of element.querySelectorAll<HTMLElement>('[role="region"]')) {
        region.tabIndex = -1
        region.setAttribute('aria-label', `${block.sender} · ${block.id} · ${turn} ${kind} ${meta}`)
      }
      const paragraphs = element.querySelectorAll('[data-turn-content] p')
      setTail(paragraphs.item(paragraphs.length - 1))
    }
    adoptContent()
    const observer = new MutationObserver(adoptContent)
    observer.observe(element, { childList: true, subtree: true })
    return () => observer.disconnect()
  }, [expanded, meta, block.id, block.sender, turn, kind])
  const time = <time data-turn-time dateTime={block.at} {...stylex.props(styles.time)}>{clock(block.at)}</time>
  const copy = <TooltipTrigger delay={0}><Button data-copy data-affordance="reveal" aria-label={`${copied ? 'Copied' : 'Copy'} ${block.id}`} onPress={async () => { await navigator.clipboard.writeText(block.text); setCopied(true) }} {...stylex.props(styles.copy, meta === 'M1' && styles.inlineCopy, meta === 'M1' && seat && styles.rowCopy)}><Icon name={copied ? 'check' : 'copy'} scale="status" /></Button><Tooltip {...stylex.props(styles.tooltip)}>{meta === 'M3' ? `${copied ? 'Copied · ' : ''}${exact(block.at)}` : copied ? 'Copied' : 'Copy message'}</Tooltip></TooltipTrigger>
  const inputMeta = <span data-input-meta {...stylex.props(styles.inputMeta)}>{kind === 'K1' ? <span data-input-kind>{block.kind}{!block.verified ? ' · Unverified' : ''}</span> : null}{meta === 'M1' ? <>{copy}<span data-meta-time>{time}</span></> : null}</span>
  const content = seat
    ? <ThreadPrimitive.Unstable_MessageById messageId={block.id} components={answerComponents} />
    : <AgentMessage from={block.sender} via={{ _tag: 'SmallTalk' }} body={{ _tag: 'Full', markdown: block.text }} bodyStyles={[styles.unclampedBody]} subject={block.subject} variant={block.kind === 'Person' ? { _tag: 'Human', frame: { _tag: 'Card' } } : { _tag: 'Incoming' }} headerMeta={inputMeta} headerGlyph={turn === `T${3}` ? { _tag: 'Hidden' } : { _tag: 'Icon', name: glyphs[block.kind] }} frameStyles={[...(block.kind === 'Person' ? [styles.humanRail] : []), ...(turn === 'T2' ? [styles.bubbleFrame] : []), ...(kind === 'K2' && block.kind !== 'Person' ? [block.kind === 'Agent' ? styles.agentRail : styles.quietRail] : [])]} />
  const header = seat && turn === `T${3}` && previous?.sender !== block.sender
    ? <header data-turn-header {...stylex.props(styles.header)}><strong>{block.sender}</strong></header> : null
  const body = <div data-turn-content {...stylex.props(styles.content)}>{content}</div>
  return <div ref={root} {...hoverProps} data-turn-block={block.id} data-turn-kind={block.kind} data-turn-structure={turn} data-kind-cue={kind} data-meta={meta} data-hovered={isHovered || undefined} data-tapped={tapped || undefined} title={meta === 'M3' ? exact(block.at) : undefined} onPointerDown={event => { if (event.pointerType === 'touch') setTapped(value => !value) }} {...stylex.props(styles.block, !seat && previous !== undefined && styles.turnStart, meta === 'M1' && seat && styles.seatRow, turn === 'T2' && !seat && styles.bubble, turn === `T${3}` && styles.senderGutter, collapsedKind && styles.quiet)}>
    {turn === `T${3}` && !collapsedKind ? <span aria-hidden="true" {...stylex.props(styles.glyph)}><Icon name={glyphs[block.kind]} /></span> : null}
    {meta === 'M3' && isDivider(block, previous) ? <div data-time-divider {...stylex.props(styles.divider)}>{time}</div> : null}
    {meta === 'M1' && seat ? <div data-seat-meta-overlay data-affordance="reveal" {...stylex.props(styles.seatMeta)}><span data-meta-time>{time}</span>{copy}</div> : null}
    {collapsedKind ? <Disclosure isExpanded={expanded} onExpandedChange={setExpanded}><Button slot="trigger" {...stylex.props(styles.summary, meta === 'M4' && styles.trailingSummary, marker && styles.marker)}><span data-input-kind-icon aria-hidden="true" {...stylex.props(styles.summarySlot)}><Icon name={glyphs[block.kind]} scale="status" /></span><strong data-input-sender {...stylex.props(styles.summarySender)}>{block.sender}</strong><span data-input-title {...stylex.props(styles.summaryText)}><span data-input-title-run {...stylex.props(styles.summaryRun)}>{block.text.split('\n')[0]}</span></span><span data-input-chevron data-expanded={expanded || undefined} {...stylex.props(styles.summarySlot)}><Icon name="chevron-right" scale="disclosure" /></span>{meta === 'M4' && !expanded ? <span data-meta-time {...stylex.props(styles.trailing)}>{' '}{time}</span> : null}</Button><DisclosurePanel>{expanded ? <>{header}{body}</> : null}</DisclosurePanel></Disclosure> : <>{header}{body}</>}
    {meta === 'M1' && collapsedKind && !expanded ? <span data-collapsed-meta {...stylex.props(styles.collapsedMeta)}>{copy}<span data-meta-time>{time}</span></span> : null}
    {meta === 'M2' ? <div data-meta-time data-affordance="reveal" {...stylex.props(styles.timeOverlay, styles.gutterTime)}>{time}</div> : null}
    {meta === 'M4' && tail !== null && (!collapsedKind || expanded) ? createPortal(<span data-meta-time {...stylex.props(styles.trailing)}>{' '}{time}</span>, tail) : null}
    {meta !== 'M1' ? copy : null}
  </div>
}

function Lane({ args }: { args: ExploreArgs }) {
  return <section data-lane data-recommended={args.recommended || undefined} data-turn-variant={args.turn} data-kind-variant={args.kind} data-meta-variant={args.meta} data-scheme={args.scheme} data-width={args.width} aria-label={`${args.turn} ${args.kind} ${args.meta} · ${args.scheme} · ${args.width}`} {...stylex.props(styles.lane)}>
    <h2 {...stylex.props(styles.laneTitle)}>{args.recommended ? 'Recommended · pending ratification · ' : ''}{args.turn} · {args.kind} · {args.meta} / {args.scheme} / {args.width}</h2>
    <EmbraceRuntimeProvider options={{ messages: replyItems, isRunning: false, onNew: async () => {} }}><ThreadPrimitive.Root {...stylex.props(styles.thread)}>{turnBlocks.map((block, index) => <React.Fragment key={block.id}><Block block={block} previous={turnBlocks[index - 1]} turn={args.turn} kind={args.recommended && block.kind === 'System' ? 'K3' : args.kind} meta={args.meta} marker={args.recommended && block.kind === 'System'} />{index === 7 ? <div data-work-fixture {...stylex.props(styles.work)}><WorkLogV1 turn={workTurn} ariaLabel={`Work log · ${args.turn} ${args.kind} ${args.meta}`} /></div> : null}</React.Fragment>)}</ThreadPrimitive.Root></EmbraceRuntimeProvider>
  </section>
}
function Explorer(initialArgs: ExploreArgs) {
  const [, updateStoryArgs] = useArgs<ExploreArgs>()
  const [probeArgs, setProbeArgs] = React.useState(initialArgs)
  const args = initialArgs.probe ? probeArgs : initialArgs
  // Probe interactions belong to their own mount, not Storybook's arg lifecycle.
  const updateArgs = initialArgs.probe
    ? (patch: Partial<ExploreArgs>) => setProbeArgs(current => ({ ...current, ...patch }))
    : updateStoryArgs
  const turns = args.compareT ? dimensions.T.map(choice => choice.id) : [args.turn]
  const kinds = args.compareK ? dimensions.K.map(choice => choice.id) : [args.kind]
  const metas = args.compareM ? dimensions.M.map(choice => choice.id) : [args.meta]
  return <main data-turns-explorer data-token-probe {...stylex.props(...baselineTheme, args.scheme === 'light' && lightTheme, styles.root, args.width === 1440 ? styles.wide : styles.narrow)}>
    <style>{projectionCss}</style>
    <h1 {...stylex.props(styles.title)}>Transcript turns</h1><p {...stylex.props(styles.explanation)}>Inputs trigger the turn. The owning seat replies. Compare language, input cues and metadata without adding a footer.</p>
    <div {...stylex.props(styles.pickers)}>
    <Button onPress={() => updateArgs({ ...recommendedArgs, scheme: args.scheme, width: args.width })} aria-pressed={args.recommended} {...stylex.props(styles.control, styles.preset)}>Recommended preset</Button>
    <VariantPicker label="Turn structure" choices={dimensions.T} value={args.turn} compare={args.compareT} onChange={turn => updateArgs({ turn: turn as ExploreArgs['turn'], recommended: false })} onCompare={compareT => updateArgs({ compareT, recommended: false })} />
    <VariantPicker label="Input kind" choices={dimensions.K} value={args.kind} compare={args.compareK} onChange={kind => updateArgs({ kind: kind as ExploreArgs['kind'], recommended: false })} onCompare={compareK => updateArgs({ compareK, recommended: false })} />
    <VariantPicker label="Time and copy" choices={dimensions.M} value={args.meta} compare={args.compareM} onChange={meta => updateArgs({ meta: meta as ExploreArgs['meta'], recommended: false })} onCompare={compareM => updateArgs({ compareM, recommended: false })} />
    </div>
    <div {...stylex.props(styles.controls)}>{(['dark', 'light'] as const).map(scheme => <Button key={scheme} aria-pressed={args.scheme === scheme} onPress={() => updateArgs({ scheme })} {...stylex.props(styles.control)}>{scheme}</Button>)}{([1440, 900] as const).map(width => <Button key={width} aria-pressed={args.width === width} onPress={() => updateArgs({ width })} {...stylex.props(styles.control)}>{width} lane</Button>)}</div>
    <div {...stylex.props(styles.comparisons)}>{turns.flatMap(turn => kinds.flatMap(kind => metas.map(meta => <Lane key={`${turn}-${kind}-${meta}-${args.recommended}`} args={{ ...args, turn, kind, meta } as ExploreArgs} />)))}</div>
    {args.probe ? <section aria-label="Deliberately failing controls" data-controls {...stylex.props(styles.negativeControls)}>
      <div data-control-height {...stylex.props(styles.block)}><p>Reply with a legacy footer.</p><div data-meta-time {...stylex.props(styles.footerControl)}><time>6:32 AM</time></div></div>
      <div data-control-owner {...stylex.props(styles.block)}><p>Owning reply.</p><time data-turn-time {...stylex.props(styles.wrongOwner)}>6:32 AM</time></div><div data-control-next {...stylex.props(styles.block)}><p>Next input.</p></div>
      <div data-control-focus {...stylex.props(styles.block)}><span data-affordance="reveal" {...stylex.props(styles.hoverOnly)}>Hover-only time</span><Button {...stylex.props(styles.control)}>Control copy</Button></div>
      <div data-control-style>Unstyled token probe</div>
      <div data-control-transition data-turn-variant="T1" data-kind-variant="K1" data-meta-variant="M1" data-scheme="dark" data-width="1440" data-lane-count="1"><Button {...stylex.props(styles.control)}>Inert transition control</Button><Button aria-expanded={false} {...stylex.props(styles.control)}>Inert disclosure control</Button></div>
      <EmbraceRuntimeProvider options={{ messages: replyItems, isRunning: false, onNew: async () => {} }}><ThreadPrimitive.Root data-control-production-meta><ThreadPrimitive.Unstable_MessageById messageId={replyItems[0]!.id} components={metaControlComponents} /></ThreadPrimitive.Root></EmbraceRuntimeProvider>
      <div data-control-k3 {...stylex.props(styles.quietInputControl)}><span data-input-title>Quiet summary with no sender or kind cue</span></div>
      <div data-control-k3-stacked {...stylex.props(styles.stackedInputControl)}><span data-input-title><span data-input-title-run>Sender<br />Wrapped title</span></span></div>
      <FocusRemountControl />
      <div data-control-missing-time>Reply without its own timestamp.</div>
      <div data-control-nested-card data-fractal-card {...stylex.props(styles.inputFrame)}><header data-message-header>Projected sender/time</header><article data-fractal-card {...stylex.props(styles.inputFrame)}><header data-message-header>Duplicate card header</header>Nested input.</article></div>
      <div data-control-covered-line {...stylex.props(styles.block)}><p>First line under a toolbar.</p><div data-seat-meta-overlay {...stylex.props(styles.seatMeta, styles.coveredLine)}>Covering metadata</div></div>
      <div data-control-long-final-line {...stylex.props(styles.longLineControl)}><div {...stylex.props(styles.block)}><p {...stylex.props(styles.controlLine)}>A preceding input whose final line runs to the right edge.</p></div><div {...stylex.props(styles.block, styles.seatRow)}><div data-seat-meta-overlay {...stylex.props(styles.seatMeta)}>Meta</div><p {...stylex.props(styles.controlLine)}>Reply.</p></div></div>
      <div data-control-proximity {...stylex.props(styles.longLineControl)}><div {...stylex.props(styles.block, styles.inputFrame)}><p {...stylex.props(styles.controlLine)}>Trigger input.</p></div><div {...stylex.props(styles.block)}><div {...stylex.props(styles.seatMeta, styles.stackedRow)}>Meta</div><p data-control-reply {...stylex.props(styles.controlLine)}>Reply under the stacked 8px gap and 24px row.</p></div><div {...stylex.props(styles.block, styles.turnStart, styles.inputFrame)}><p {...stylex.props(styles.controlLine)}>Next turn.</p></div></div>
      <article data-control-header-wrap><header data-message-header {...stylex.props(styles.wrappedHeaderControl)}><span>From Workspace</span><span>Review · Small Talk</span></header><h2 data-message-subject {...stylex.props(styles.controlLine)}>Wrapped sender path</h2></article>
      <div data-control-identity-clip {...stylex.props(styles.identityClipControl)}><span {...stylex.props(styles.ellipsisLine)}>From <strong><span data-agent-id-trigger {...stylex.props(styles.atomicRun)}><span data-agent-segment="parent">Workspace</span>›<span data-agent-segment="leaf">Review</span></span></strong> · Small Talk</span></div>
      <span data-control-unicode-glyph>↪</span>
      <div data-control-long-identities {...stylex.props(styles.longIdentityControl)}>{longIdentities.map(id => <AgentMessage key={id} from={id} via={{ _tag: 'SmallTalk' }} body={{ _tag: 'Full', markdown: 'Long identity.' }} headerMeta={controlMeta(false)} />)}</div>
      <div data-control-meta-yields {...stylex.props(styles.longIdentityControl)}><AgentMessage from={longIdentities[0]} via={{ _tag: 'SmallTalk' }} body={{ _tag: 'Full', markdown: 'Metadata that yields.' }} headerMeta={controlMeta(true)} /></div>
      <div data-control-copy-hit {...stylex.props(styles.block)}><div {...stylex.props(styles.seatMeta, styles.flatRow)}><Button data-copy aria-label="Copy hit-area control" {...stylex.props(styles.copy, styles.inlineCopy, styles.rowCopy)}><Icon name="copy" scale="status" /></Button></div><p {...stylex.props(styles.controlLine)}>Reply line under the Copy overhang.</p></div>
      <strong data-control-heavy-sender>Heavy sender</strong>
    </section> : null}
  </main>
}
// Scoped variant projections keep the production message / Markdown / work-log renderers.
// Every geometry value comes from composition tokens (including the locked hierarchy step).
const projectionCss = `
[data-turns-explorer] [data-turn-content] p:last-child { margin-bottom: 0; }
[data-turns-explorer] [data-meta="M4"] [data-turn-content] p:last-child { padding-inline-end: ${geometry.trailingWidth}; }
[data-turns-explorer] [data-turn-block] [data-affordance="reveal"] { opacity: 0; }
[data-turns-explorer] [data-turn-block][data-hovered] [data-affordance="reveal"],
[data-turns-explorer] [data-turn-block]:focus-within [data-affordance="reveal"],
[data-turns-explorer] [data-turn-block][data-tapped] [data-affordance="reveal"] { opacity: 1; }
[data-turns-explorer] [data-input-chevron] svg { transition: transform ${m.fast} ${m.ease}; }
[data-turns-explorer] [data-input-chevron][data-expanded] svg { transform: rotate(90deg); }
@media (prefers-reduced-motion: reduce) { [data-turns-explorer] [data-input-chevron] svg { transition: none; } }
`
const styles = stylex.create({
  root: { padding: s.section, maxWidth: '100%', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, boxSizing: 'border-box' },
  wide: { width: g.polishWide }, narrow: { width: geometry.narrow },
  title: { margin: 0, fontSize: t.headingSize, lineHeight: t.headingLeading }, explanation: { color: ink.fgMuted, marginBlock: s.md },
  pickers: { display: 'flex', flexDirection: 'column', gap: s.md }, preset: { alignSelf: 'start' },
  controls: { display: 'flex', gap: s.md, marginBlock: s.md }, control: { padding: s.xs, color: ink.fg, backgroundColor: surface.rowActive, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, cursor: 'pointer', fontFamily: t.fontSans, ':focus-visible': { outlineColor: accent.primary, outlineStyle: 'solid', outlineWidth: g.focusRing } },
  comparisons: { display: 'flex', alignItems: 'start', gap: s.section, overflowX: 'auto' },
  lane: { flex: '0 0 auto', width: '100%', minWidth: 0, boxSizing: 'border-box' }, laneTitle: { marginBlock: s.lg, fontSize: t.metaSize, color: ink.fgMuted, fontWeight: t.weightMedium },
  thread: { width: g.lane, maxWidth: '100%', marginInline: 'auto', paddingLeft: g.textColumn, boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.md },
  block: { position: 'relative', minWidth: 0, color: ink.fg, fontSize: t.bodySize, lineHeight: t.bodyLeading },
  turnStart: { marginTop: s.xl },
  inputFrame: { paddingBlock: s.md, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.raised },
  // Below 640px a 75% bubble leaves the header no room for the sender and its metadata, so bubbles take the lane width.
  bubble: { alignSelf: 'end', maxWidth: { default: '75%', '@media (max-width: 640px)': '100%' } }, senderGutter: { marginLeft: g.textColumn }, glyph: { position: 'absolute', right: '100%', top: s.md, width: g.icon, color: ink.fgMuted, textAlign: 'center' },
  bubbleFrame: { borderRadius: r.lg },
  humanRail: { borderLeftWidth: g.focusRing, borderLeftColor: accent.primary }, agentRail: { borderLeftWidth: g.focusRing, borderLeftColor: ink.fgMuted }, quietRail: { borderLeftWidth: g.focusRing, borderLeftColor: ink.fgMuted, borderLeftStyle: 'dashed' },
  unclampedBody: { maxHeight: 'none', overflowY: 'visible' },
  quiet: { paddingBlock: 0, borderWidth: 0, backgroundColor: surface.transparent },
  header: { display: 'flex', alignItems: 'center', gap: s.sm, minHeight: t.metaLeading, paddingRight: geometry.metaRoom, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, marginBottom: s.xs2 },
  content: { minWidth: 0 },
  time: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, whiteSpace: 'nowrap', fontVariantNumeric: 'tabular-nums' },
  timeOverlay: { position: 'absolute', top: s.md, right: g.gutter40, lineHeight: t.metaLeading }, gutterTime: { right: '100%', marginRight: s.md, top: s.md },
  trailing: { display: 'inline-block', width: geometry.trailingWidth, marginInlineEnd: geometry.trailingOverlap, whiteSpace: 'nowrap' },
  copy: { position: 'absolute', top: s.xs, right: 0, minHeight: g.controlSm, padding: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.raised, color: ink.fgMuted, cursor: 'pointer', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  inlineCopy: { position: 'static', minHeight: 0, padding: 0, backgroundColor: surface.transparent },
  // The seat row is one 16px meta line; Copy keeps a 24px hit area by overhanging it equally above and below.
  rowCopy: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minWidth: g.controlSm, minHeight: g.controlSm, marginBlock: `calc((${t.metaLeading} - ${g.controlSm}) / 2)` },
  // Header metadata never yields: the kind word (Unverified is the only non-silent state), Copy and time keep their
  // width, and only the sender's identity run narrows.
  inputMeta: { marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: s.md, flex: '0 0 auto', minWidth: 'max-content' },
  inputMetaYields: { flex: '0 1 auto', minWidth: 0 },
  inputKindYields: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  // The width of a card at a 440px viewport.
  longIdentityControl: { width: '360px', maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: s.md },
  // M1 reserves one 16px meta row above each seat run at every width. The row replaces the 8px block gap rather than
  // stacking on it, so a reply stays closer to its own trigger than to the next turn; the toolbar never overlays text.
  // The row is layered above the reply so Copy's overhang stays hittable over the reply's first line box.
  seatMeta: { position: 'relative', zIndex: 1, width: 'max-content', maxWidth: '100%', marginLeft: 'auto', height: t.metaLeading, boxSizing: 'border-box', display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, backgroundColor: surface.canvas },
  flatRow: { position: 'static', zIndex: 'auto' },
  seatRow: { marginTop: `calc(-1 * ${s.md})` },
  stackedRow: { height: g.toolRow },
  identityClipControl: { width: g.controlLg, maxWidth: '100%', fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap' },
  ellipsisLine: { display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  atomicRun: { display: 'inline-flex', alignItems: 'center' },
  coveredLine: { position: 'absolute', top: 0, bottom: 'auto', left: 0, right: 'auto' },
  longLineControl: { width: g.previewMax, maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: s.md },
  controlLine: { margin: 0, textAlign: 'right' },
  wrappedHeaderControl: { display: 'flex', flexDirection: 'column', height: g.icon, fontSize: t.metaSize, lineHeight: t.metaLeading },
  collapsedMeta: { position: 'absolute', right: 0, top: 0, display: 'flex', gap: s.md, alignItems: 'center', height: g.toolRow },
  divider: { position: 'absolute', bottom: '100%', left: 0, right: 0, textAlign: 'center', color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  summary: { display: 'flex', flexWrap: 'nowrap', alignItems: 'center', gap: s.sm, width: '100%', padding: 0, paddingRight: `min(${geometry.metaRoom}, 25%)`, minHeight: g.toolRow, lineHeight: g.toolRow, whiteSpace: 'nowrap', borderWidth: 0, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, textAlign: 'left', cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, summaryText: { flex: '1 1 auto', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 },
  trailingSummary: { paddingRight: 0 },
  summaryRun: { display: 'inline-block', whiteSpace: 'nowrap' },
  summarySender: { fontWeight: t.weightMedium, flexShrink: 0, maxWidth: '40%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  summarySlot: { width: g.icon, height: g.icon, flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' },
  marker: { minHeight: t.uiLeading, lineHeight: t.uiLeading, fontSize: t.metaSize, color: ink.fgMuted },
  stackedInputControl: { width: geometry.metaRoom, fontSize: t.uiSize, lineHeight: g.toolRow },
  quietInputControl: { color: ink.fgMuted, height: g.iconSm, overflow: 'hidden', fontSize: t.metaSize, lineHeight: t.metaLeading },
  work: { marginTop: s.md }, tooltip: { padding: s.md, backgroundColor: surface.raised, color: ink.fg, fontSize: t.metaSize, borderRadius: r.sm },
  negativeControls: { marginTop: s.section, display: 'flex', flexDirection: 'column', gap: s.section }, footerControl: { height: g.toolRow }, wrongOwner: { position: 'absolute', top: '100%', marginTop: s.section, fontSize: t.metaSize }, hoverOnly: { opacity: { default: 0, ':hover': 1 } },
})

const recommendedArgs = { turn: 'T1', kind: 'K1', meta: 'M1', compareT: false, compareK: false, compareM: false, recommended: true } satisfies Partial<ExploreArgs>
const meta = { title: 'Fractal UI/Explore/Transcript Turns', component: Explorer, render: Explorer, parameters: { layout: 'fullscreen' }, args: { turn: 'T1', kind: 'K1', meta: 'M1', compareT: false, compareK: false, compareM: false, scheme: 'dark', width: 1440, recommended: false, probe: false }, argTypes: { turn: { options: dimensions.T.map(choice => choice.id), control: 'radio' }, kind: { options: dimensions.K.map(choice => choice.id), control: 'radio' }, meta: { options: dimensions.M.map(choice => choice.id), control: 'radio' }, scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [1440, 900], control: 'radio' }, probe: { table: { disable: true }, control: false } } } satisfies Meta<typeof Explorer>
export default meta
type Story = StoryObj<typeof meta>
const distance = (label: DOMRect, block: DOMRect) => Math.max(block.top - label.bottom, label.top - block.bottom, 0)
const intersects = (a: DOMRect, b: DOMRect) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
function visibleTextRects(block: HTMLElement) {
  const content = Array.from(block.querySelectorAll<HTMLElement>('[data-message-content], [data-testid="agent-message"], [data-input-title], p')).find(element => element.getClientRects().length > 0) ?? block
  const bounds = content.getBoundingClientRect()
  const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT)
  const rects: DOMRect[] = []
  while (walker.nextNode()) {
    const text = walker.currentNode
    if (text.textContent?.trim() === '' || text.parentElement?.closest('[data-meta-time], [data-copy], [data-seat-meta-overlay]')) continue
    const range = document.createRange()
    range.selectNodeContents(text)
    for (const rect of range.getClientRects()) {
      const left = Math.max(rect.left, bounds.left), right = Math.min(rect.right, bounds.right)
      const top = Math.max(rect.top, bounds.top), bottom = Math.min(rect.bottom, bounds.bottom)
      if (right > left && bottom > top) rects.push(new DOMRect(left, top, right - left, bottom - top))
    }
  }
  return rects
}
function lastLineRects(block: HTMLElement) {
  const rects = visibleTextRects(block)
  const bottom = Math.max(...rects.map(rect => rect.bottom))
  return rects.filter(rect => rect.bottom > bottom - 1)
}
/** Visible header text that leaves its 16px header box or reaches into the subject line. */
function headerCollisions(header: Element, subject: Element) {
  const box = header.getBoundingClientRect(), subjectBox = subject.getBoundingClientRect()
  const walker = document.createTreeWalker(header, NodeFilter.SHOW_TEXT)
  const collisions: DOMRect[] = []
  while (walker.nextNode()) {
    const parent = walker.currentNode.parentElement!
    if (walker.currentNode.textContent?.trim() === '' || parent.getBoundingClientRect().width <= 1) continue
    const range = document.createRange()
    range.selectNodeContents(walker.currentNode)
    for (const rect of range.getClientRects()) if (rect.bottom > box.bottom + 0.5 || rect.top < box.top - 0.5 || intersects(rect, subjectBox)) collisions.push(rect)
  }
  return collisions
}
/** Text extent of a subtree, excluding its seat meta row: top of its first glyph row, bottom of its last. */
function textExtent(root: Element) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  let top = Infinity, bottom = -Infinity
  while (walker.nextNode()) {
    if (walker.currentNode.textContent?.trim() === '' || walker.currentNode.parentElement?.closest('[data-seat-meta-overlay]')) continue
    const range = document.createRange()
    range.selectNodeContents(walker.currentNode)
    for (const rect of range.getClientRects()) if (rect.width > 0 && rect.height > 0) { top = Math.min(top, rect.top); bottom = Math.max(bottom, rect.bottom) }
  }
  return { top, bottom }
}
/** A reply's text distance to its own trigger and to the next turn. */
function proximity(trigger: Element, reply: Element, nextTurn: Element) {
  const text = textExtent(reply)
  return { toTrigger: text.top - trigger.getBoundingClientRect().bottom, toNext: nextTurn.getBoundingClientRect().top - text.bottom }
}
/** A dotted identity must narrow inside its own run: the leaf stays whole until every ancestor is folded into '…',
 * no glyph sits outside the run, and no ancestor clips the run or lays an ellipsis over it. */
function identityFaults(trigger: HTMLElement, boundary: Element) {
  const faults: string[] = []
  const run = trigger.getBoundingClientRect()
  const leaf = trigger.querySelector<HTMLElement>('[data-agent-segment="leaf"]')
  const ancestorsShown = trigger.querySelector('[data-agent-segment="parent"]') !== null
  if (leaf === null || (ancestorsShown && leaf.scrollWidth > leaf.clientWidth)) faults.push('leaf truncated')
  if (trigger.scrollWidth > trigger.clientWidth) faults.push('run overflows')
  for (const segment of trigger.querySelectorAll('[data-agent-segment]')) {
    const box = segment.getBoundingClientRect()
    if (box.left < run.left - 0.5 || box.right > run.right + 0.5) faults.push('glyph outside run')
  }
  for (let element = trigger.parentElement; element !== null && element !== boundary.parentElement; element = element.parentElement) {
    const style = getComputedStyle(element), clip = element.getBoundingClientRect()
    if (style.textOverflow === 'ellipsis') faults.push('ellipsis over the run')
    if (style.overflowX !== 'visible' && (run.left < clip.left - 0.5 || run.right > clip.right + 0.5)) faults.push('run clipped')
  }
  return faults
}
/** Header metadata never yields to the sender: the kind word is whole, and nothing renders past the card text edge. */
function headerFaults(header: HTMLElement) {
  const faults: string[] = []
  const edge = header.getBoundingClientRect()
  const kind = header.querySelector<HTMLElement>('[data-input-kind]')
  if (kind !== null && (kind.clientWidth === 0 || kind.scrollWidth > kind.clientWidth)) faults.push('kind truncated')
  for (const item of header.querySelectorAll<HTMLElement>('[data-input-kind], [data-meta-time], [data-agent-segment], [data-copy]')) {
    const box = item.getBoundingClientRect()
    if (box.width > 0 && (box.left < edge.left - 0.5 || box.right > edge.right + 0.5)) faults.push('item past the card text edge')
  }
  // The channel yields before any identity segment, so it is gone once an ancestor folds.
  if (header.querySelector('[data-agent-id-trigger]:not([data-collapsed="0"])') !== null && header.querySelector('[data-message-channel]') !== null) faults.push('channel beside a folded identity')
  for (const trigger of header.querySelectorAll<HTMLElement>('[data-agent-id-trigger]')) faults.push(...identityFaults(trigger, header))
  return faults
}
/** Hittable share of a control's box, sampled at pixel centres inside its rounded corners. */
function hitArea(control: HTMLElement) {
  control.scrollIntoView({ block: 'center' })
  const box = control.getBoundingClientRect(), radius = parseFloat(getComputedStyle(control).borderTopLeftRadius) || 0
  let total = 0, hit = 0
  for (let y = 0.5; y < box.height; y++) for (let x = 0.5; x < box.width; x++) {
    const dx = Math.max(radius - x, x - (box.width - radius), 0), dy = Math.max(radius - y, y - (box.height - radius), 0)
    if (dx * dx + dy * dy > radius * radius) continue
    total++
    const target = control.ownerDocument.elementFromPoint(box.left + x, box.top + y)
    if (target !== null && control.contains(target)) hit++
  }
  return { width: box.width, height: box.height, total, hit }
}
function reservedMetaHeight(block: HTMLElement) {
  const height = block.getBoundingClientRect().height
  const metadata = Array.from(block.querySelectorAll<HTMLElement>('[data-meta-time], [data-time-divider], [data-copy], [data-seat-meta-overlay], time'))
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
    for (const block of lane.querySelectorAll<HTMLElement>('[data-turn-block]')) {
      await expect(parseFloat(getComputedStyle(block).borderTopWidth)).toBe(0)
      await expect(block.querySelector('[data-fractal-card] [data-fractal-card]')).toBeNull()
      if (block.querySelector('[data-fractal-card]') !== null) await expect(block.querySelectorAll('[data-message-header]')).toHaveLength(1)
      for (const prose of block.querySelectorAll('[data-message-content] p')) await expect(getComputedStyle(prose).color).toBe(getComputedStyle(block).color)
      for (const header of block.querySelectorAll('[data-message-header]')) {
        if (block.getAttribute('data-turn-structure') !== `T${3}`) await expect(header.querySelector('svg')).not.toBeNull()
        for (const glyph of header.querySelectorAll(':scope > span[aria-hidden="true"]')) await expect(glyph.querySelector('svg')).not.toBeNull()
        const subject = header.parentElement?.querySelector(':scope > [data-message-subject]')
        if (subject) await expect(headerCollisions(header, subject)).toEqual([])
        await expect(headerFaults(header as HTMLElement)).toEqual([])
      }
      const overlay = block.querySelector('[data-seat-meta-overlay]')
      if (overlay !== null) {
        const firstLine = block.querySelector('[data-turn-content] p')!
        await expect(overlay.getBoundingClientRect().bottom).toBeLessThanOrEqual(firstLine.getBoundingClientRect().top)
        await expect(overlay.getBoundingClientRect().height).toBe(16)
        const previous = block.previousElementSibling as HTMLElement
        await expect(Math.abs(block.getBoundingClientRect().top - previous.getBoundingClientRect().bottom)).toBeLessThan(0.5)
        if (previous.getAttribute('data-turn-kind') !== 'Seat') {
          const nextTurn = Array.from(lane.querySelectorAll<HTMLElement>('[data-turn-block]:not([data-turn-kind="Seat"])')).find(candidate => candidate.getBoundingClientRect().top > block.getBoundingClientRect().bottom - 0.5)
          if (nextTurn !== undefined) {
            // The reply starts at its own text, including a sender-gutter label; the meta row above it is not reply text.
            const { toTrigger, toNext } = proximity(previous, block, nextTurn)
            await expect(toTrigger, 'reply is closer to its own trigger than to the next turn').toBeLessThan(toNext)
          }
        }
        const previousLastLine = lastLineRects(previous)
        await expect(previousLastLine.length).toBeGreaterThan(0)
        for (const line of visibleTextRects(previous)) await expect(intersects(overlay.getBoundingClientRect(), line)).toBe(false)
        await expect(overlay.getBoundingClientRect().right).toBeLessThanOrEqual(lane.getBoundingClientRect().right)
        await expect(overlay.getBoundingClientRect().left).toBeGreaterThanOrEqual(lane.getBoundingClientRect().left)
      }
    }
    const work = lane.querySelector<HTMLElement>('[data-work-fixture]')!
    await expect(reservedMetaHeight(work)).toBe(0)
    for (const label of work.querySelectorAll('time')) {
      await expect(distance(label.getBoundingClientRect(), work.getBoundingClientRect())).toBeLessThanOrEqual(distance(label.getBoundingClientRect(), work.nextElementSibling!.getBoundingClientRect()))
    }
    if (lane.hasAttribute('data-recommended')) {
      for (const input of lane.querySelectorAll('[data-turn-kind="Relay"], [data-turn-kind="Agent"]')) {
        await expect(input).toHaveAttribute('data-kind-cue', 'K1')
        await expect(input.querySelectorAll('[data-message-header]')).toHaveLength(1)
        await expect(input.querySelector('[data-message-content]')).not.toBeNull()
      }
      await expect(lane.querySelector('[data-turn-kind="System"]')).toHaveAttribute('data-kind-cue', 'K3')
      await expect(lane.querySelector('[data-turn-kind="Seat"] [data-turn-header]')).toBeNull()
    }
    for (const block of lane.querySelectorAll<HTMLElement>('[data-kind-cue="K3"]')) {
      const trigger = block.querySelector<HTMLElement>('button[aria-expanded="false"]')
      if (trigger === null) continue
      const isMarker = lane.hasAttribute('data-recommended') && block.getAttribute('data-turn-kind') === 'System'
      await expect(getComputedStyle(trigger.querySelector('[data-input-title]')!).color).toBe(isMarker ? getComputedStyle(canvasElement.querySelector('[data-control-k3]')!).color : getComputedStyle(block).color)
      await expect(trigger.querySelector('[data-input-sender]')!.textContent?.trim().length).toBeGreaterThan(0)
      await expect(getComputedStyle(trigger.querySelector('[data-input-sender]')!).fontWeight).toBe('500')
      await expect(trigger.querySelector('[data-input-kind-icon] svg')).not.toBeNull()
      await expect(trigger.getBoundingClientRect().height).toBeGreaterThanOrEqual(lane.hasAttribute('data-recommended') && block.getAttribute('data-turn-kind') === 'System' ? 20 : 24)
      await expect(trigger.getBoundingClientRect().height).toBeLessThanOrEqual(parseFloat(getComputedStyle(trigger).lineHeight))
      await expect(trigger.querySelector<HTMLElement>('[data-input-title-run]')!.getClientRects()).toHaveLength(1)
    }
  }
  await waitFor(() => expect(canvasElement.querySelector('[data-control-production-meta] [data-testid="answer-meta"]')).not.toBeNull())
  const quietControl = canvasElement.querySelector<HTMLElement>('[data-control-k3]')!
  await expect(getComputedStyle(quietControl.querySelector('[data-input-title]')!).color).not.toBe(styled.color)
  await expect(quietControl.querySelector('[data-input-sender], [data-input-kind-icon]')).toBeNull()
  await expect(quietControl.getBoundingClientRect().height).toBeLessThan(24)
  await expect(getComputedStyle(canvasElement.querySelector('[data-control-heavy-sender]')!).fontWeight).not.toBe('500')
  const stackedControl = canvasElement.querySelector<HTMLElement>('[data-control-k3-stacked]')!
  await expect(stackedControl.getBoundingClientRect().height).toBeGreaterThan(parseFloat(getComputedStyle(stackedControl).lineHeight))
  await expect(stackedControl.querySelector<HTMLElement>('[data-input-title-run]')!.getClientRects().length).toBeGreaterThan(1)
  const nestedControl = canvasElement.querySelector<HTMLElement>('[data-control-nested-card]')!
  await expect(parseFloat(getComputedStyle(nestedControl).borderTopWidth)).toBeGreaterThan(0)
  await expect(nestedControl.querySelector('[data-fractal-card] [data-fractal-card]')).not.toBeNull()
  await expect(nestedControl.querySelectorAll('[data-message-header]')).toHaveLength(2)
  const coveredControl = canvasElement.querySelector('[data-control-covered-line]')!
  await expect(coveredControl.querySelector('[data-seat-meta-overlay]')!.getBoundingClientRect().bottom).toBeGreaterThan(coveredControl.querySelector('p')!.getBoundingClientRect().top)
  await expect(lastLineRects(coveredControl as HTMLElement).some(line => intersects(coveredControl.querySelector('[data-seat-meta-overlay]')!.getBoundingClientRect(), line))).toBe(true)
  // Long final line: the preceding block's last line reaches the right edge. The row toolbar clears it;
  // the rejected zero-height overhang (absolute, 32px past the run) erases its end.
  const longLineControl = canvasElement.querySelector<HTMLElement>('[data-control-long-final-line]')!
  const longLineToolbar = longLineControl.querySelector<HTMLElement>('[data-seat-meta-overlay]')!
  const preceding = longLineControl.firstElementChild as HTMLElement
  const overlapsPreceding = () => visibleTextRects(preceding).some(line => intersects(longLineToolbar.getBoundingClientRect(), line))
  await expect(lastLineRects(preceding).at(-1)!.right).toBeGreaterThan(preceding.getBoundingClientRect().right - 2)
  await expect(overlapsPreceding()).toBe(false)
  Object.assign(longLineToolbar.style, { position: 'absolute', bottom: '100%', right: `calc(-1 * ${s.panel})` })
  await expect(overlapsPreceding()).toBe(true)
  Object.assign(longLineToolbar.style, { position: '', bottom: '', right: '' })
  await expect(overlapsPreceding()).toBe(false)
  const wrappedHeader = canvasElement.querySelector('[data-control-header-wrap]')!
  await expect(headerCollisions(wrappedHeader.querySelector('[data-message-header]')!, wrappedHeader.querySelector('[data-message-subject]')!).length).toBeGreaterThan(0)
  // The previous structure (an atomic inline-flex run inside an ellipsis line) clips the identity instead of truncating it.
  const clipControl = canvasElement.querySelector('[data-control-identity-clip]')!
  await expect(identityFaults(clipControl.querySelector<HTMLElement>('[data-agent-id-trigger]')!, clipControl)).not.toEqual([])
  // The previous stack (8px gap plus a 24px row) puts the reply nearer the next turn than its own trigger.
  const proximityControl = canvasElement.querySelector('[data-control-proximity]')!
  const stacked = proximity(proximityControl.children[0]!, proximityControl.querySelector('[data-control-reply]')!, proximityControl.children[2]!)
  await expect(stacked.toTrigger).toBeGreaterThanOrEqual(stacked.toNext)
  for (const header of canvasElement.querySelectorAll('[data-lane] [data-message-header]')) await expect(header.querySelector('[title]')).toBeNull()
  // A six-segment chain and a 50-character leaf in a 440-width card: only the identity narrows (ancestors fold into
  // '…', the long leaf truncates last); the previous yielding metadata truncates the kind word.
  for (const header of canvasElement.querySelectorAll<HTMLElement>('[data-control-long-identities] [data-message-header]')) await waitFor(() => expect(headerFaults(header)).toEqual([]))
  await expect(Number(canvasElement.querySelector('[data-control-long-identities] [data-agent-id-trigger]')!.getAttribute('data-collapsed'))).toBeGreaterThan(0)
  await expect(headerFaults(canvasElement.querySelector<HTMLElement>('[data-control-meta-yields] [data-message-header]')!)).toContain('kind truncated')
  // M1 Copy is hittable across its whole 24px box; without the layered row the reply's line box takes the overhang.
  for (const lane of canvasElement.querySelectorAll('[data-lane]')) {
    const rowCopy = lane.querySelector<HTMLElement>('[data-seat-meta-overlay] [data-copy]')
    if (rowCopy === null) continue
    const area = hitArea(rowCopy)
    await expect([area.width, area.height]).toEqual([24, 24])
    await expect(area.hit, 'every point of the M1 Copy box hits Copy').toBe(area.total)
  }
  const flatCopy = hitArea(canvasElement.querySelector<HTMLElement>('[data-control-copy-hit] [data-copy]')!)
  await expect(flatCopy.hit).toBeLessThan(flatCopy.total)
  if (canvasElement.ownerDocument.defaultView!.innerWidth <= 440) {
    await expect(canvasElement.ownerDocument.documentElement.scrollWidth).toBeLessThanOrEqual(canvasElement.ownerDocument.documentElement.clientWidth)
  }
  await expect(canvasElement.querySelector('[data-control-unicode-glyph] svg')).toBeNull()
  for (const lane of canvasElement.querySelectorAll('[data-lane]')) {
    const blocks = Array.from(lane.querySelectorAll<HTMLElement>('[data-turn-block]'))
    for (const [index, block] of blocks.entries()) {
      const seatToolbar = block.querySelector('[data-seat-meta-overlay]')
      const inFlow = seatToolbar !== null && getComputedStyle(seatToolbar).position !== 'absolute'
      await expect(reservedMetaHeight(block)).toBe(inFlow ? 16 : 0)
      if (lane.getAttribute('data-meta-variant') === 'M4') await expect(block.querySelectorAll('time[data-turn-time]')).toHaveLength(1)
      const next = blocks[index + 1]
      if (next === undefined) continue
      for (const label of block.querySelectorAll('time[data-turn-time]')) await expect(distance(label.getBoundingClientRect(), block.getBoundingClientRect())).toBeLessThanOrEqual(distance(label.getBoundingClientRect(), next.getBoundingClientRect()))
    }
  }
  await expect(canvasElement.querySelector('[data-control-missing-time]')!.querySelectorAll('time')).toHaveLength(0)
  await expect(reservedMetaHeight(canvasElement.querySelector<HTMLElement>('[data-control-height]')!)).toBeGreaterThan(0)
  await expect(reservedMetaHeight(canvasElement.querySelector<HTMLElement>('[data-control-missing-time]')!)).not.toBe(16)
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
    await waitFor(() => expect(Array.from(block.querySelectorAll('[data-affordance="reveal"]')).map(element => getComputedStyle(element).opacity), `hover reveals ${block.getAttribute('data-turn-block')} (${block.getAttribute('data-turn-structure')}${block.getAttribute('data-meta')}, hovered=${block.hasAttribute('data-hovered')})`).toEqual(focusOpacity))
    await userEvent.unhover(block)
    block.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    await waitFor(() => expect(getComputedStyle(copy).opacity).toBe('1'))
    block.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    await waitFor(() => expect(getComputedStyle(copy).opacity).toBe('0'))
    const extraStops = Array.from(block.querySelectorAll<HTMLElement>('button, a[href], [tabindex]')).filter(element => element !== copy && !element.hasAttribute('aria-expanded') && !element.hasAttribute('data-agent-id-trigger') && element.tabIndex >= 0 && element.getClientRects().length > 0)
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
  // The dotted identity is one focus stop named by its full identity; the kit tooltip shows it once on keyboard focus,
  // hover and touch tap.
  const identity = canvasElement.querySelector<HTMLElement>('[data-lane] [data-agent-id="Workspace.Review"] [data-agent-id-trigger]')
  if (identity !== null) {
    const view = canvasElement.ownerDocument.defaultView!
    const settle = () => new Promise<void>(resolve => view.requestAnimationFrame(() => view.requestAnimationFrame(() => resolve())))
    const tooltips = () => Array.from(canvasElement.ownerDocument.querySelectorAll('[role="tooltip"]')).filter(tip => tip.textContent === 'Workspace.Review')
    const exactlyOne = (label: string) => waitFor(() => {
      expect(tooltips(), label).toHaveLength(1)
      expect(canvasElement.ownerDocument.querySelectorAll('[role="tooltip"]').length, label).toBe(1)
    }, { timeout: 2000 })
    // Hidden dots join the flex parts; name computation may pad them with spaces.
    await expect(identity).toHaveAccessibleName(/^Workspace\s*\.\s*Review$/)
    await expect(identity.textContent).toBe('Workspace.Review')
    // Keyboard focus on an off-screen trigger scrolls it into view; that scroll must not close the tooltip it opened.
    view.scrollTo(0, identity.getBoundingClientRect().top + view.scrollY + view.innerHeight)
    await settle()
    await userEvent.keyboard('{Escape}')
    identity.focus()
    await exactlyOne('identity tooltip on keyboard focus')
    await settle()
    await exactlyOne('identity tooltip after the focus scroll settles')
    identity.blur()
    await waitFor(() => expect(tooltips()).toHaveLength(0))
    // Hover: the trigger is settled in view and the pointer is the input modality.
    identity.scrollIntoView({ block: 'center' })
    await settle()
    await userEvent.pointer({ keys: '[MouseLeft]', target: canvasElement.ownerDocument.body })
    await userEvent.hover(identity)
    await exactlyOne('identity tooltip on hover')
    await userEvent.unhover(identity)
    await waitFor(() => expect(tooltips()).toHaveLength(0))
    // Touch has no hover: a tap opens the tooltip; leaving the trigger closes it.
    await userEvent.pointer({ keys: '[TouchA]', target: identity })
    await exactlyOne('identity tooltip on touch tap')
    identity.blur()
    await waitFor(() => expect(tooltips()).toHaveLength(0))
    // React Aria's useHover ignores mouse pointerenter for 500ms after any touch pointerup (an iOS emulation guard), so a
    // later hover check would see no hover. Outlast that window; the package lib is ES2022 (no Promise.withResolvers).
    await new Promise(resolve => view.setTimeout(resolve, 600))
  }
}
const play: NonNullable<Story['play']> = async ({ canvasElement, step }) => {
  const canvas = within(canvasElement)
  const inert = canvas.getByRole('button', { name: 'Inert transition control' })
  const control = canvasElement.querySelector('[data-control-transition]')!
  await step('Initial geometry and affordances reject footer, ownership, StyleX and hover-only controls', async () => {
    await assertProjection(canvasElement)
  })
  await step('Apply Recommended preset; inert preset control cannot change the composition', async () => {
    await userEvent.click(inert)
    await expect(control).not.toHaveAttribute('data-recommended')
    await userEvent.click(canvas.getByRole('button', { name: 'Recommended preset' }))
    await waitFor(() => {
      expect(canvasElement.querySelectorAll('[data-lane]')).toHaveLength(1)
      const lane = canvasElement.querySelector('[data-lane]')!
      expect(lane).toHaveAttribute('data-recommended', 'true')
      expect(lane).toHaveAttribute('data-turn-variant', 'T1')
      expect(lane).toHaveAttribute('data-kind-variant', 'K1')
      expect(lane).toHaveAttribute('data-meta-variant', 'M1')
    })
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
    await userEvent.click(canvas.getByRole('button', { name: scheme }))
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
    await userEvent.click(canvas.getByRole('button', { name: `${width} lane` }))
    await waitFor(() => {
      expect(canvasElement.querySelector('[data-lane]')).toHaveAttribute('data-width', `${width}`)
      expect(canvasElement.querySelector('[data-token-probe]')!.getBoundingClientRect().width).toBe(Math.min(width, canvasElement.ownerDocument.documentElement.clientWidth))
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
export const ExplorationProbe: Story = { args: { probe: true }, play }
// One public Exploration; every pick (and the Recommended preset) is a URL arg. Probe stories carry the plays.
export const AllStatesProbe: Story = { args: { compareT: true, compareK: true, compareM: true, probe: true }, play }
export const MetaM1Probe: Story = { args: { meta: 'M1', probe: true }, play }
export const MetaM2Probe: Story = { args: { meta: 'M2', probe: true }, play }
export const MetaM3Probe: Story = { args: { meta: 'M3', probe: true }, play }
export const MetaM4Probe: Story = { args: { meta: 'M4', probe: true }, play }
export const Light900Probe: Story = { args: { scheme: 'light', width: 900, compareM: true, probe: true }, play }
export const Dark900Probe: Story = { args: { scheme: 'dark', width: 900, compareM: true, probe: true }, play }
export const Light1440Probe: Story = { args: { scheme: 'light', width: 1440, compareM: true, probe: true }, play }
export const RecommendedProbe: Story = { args: { ...recommendedArgs, probe: true }, play }
