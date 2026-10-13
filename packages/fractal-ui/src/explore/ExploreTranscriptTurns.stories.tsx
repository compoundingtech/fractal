import * as React from 'react'
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
import { replyItems, turnBlocks, turnNames, workTurn, type TurnBlock } from './turns-fixture'
import { turnsGeometry as geometry } from './turns-tokens.stylex'

// The owner's pick: T1 sender Cards with unnamed seat Prose, K1 kind words (system notices as a muted Marker), M1 input
// header metadata, and GR seat metadata in the right gutter (the M1 row where the lane lacks the gutter's room).
const configuration = 'T1 · K1 · M1 · GR'
interface TurnsArgs { scheme: 'dark' | 'light'; width: 1440 | 900; probe: boolean }
const glyphs: Record<TurnBlock['kind'], IconName> = { Person: 'person', Agent: 'message', Relay: 'arrow-right', System: 'gear', Seat: 'thought' }
const clockFormat = new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: '2-digit' })
const clock = (at: string) => clockFormat.format(new Date(at))
// Container queries cannot read CSS variables. These mirror g.lane (736), s.md (8) and the 64px metaGutter: the gutter
// fits when the room beside the lane holds the gutter plus an 8px gap.
const gutterRoom = { lane: 736, gap: 8, gutter: 64 }
const gutterFit = gutterRoom.lane + 2 * (gutterRoom.gutter + gutterRoom.gap)

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

/** Projection of production rows in the chosen configuration. No change to runtime facts or message bodies. */
function Block({ block, previous }: { block: TurnBlock; previous?: TurnBlock }) {
  const root = React.useRef<HTMLDivElement>(null)
  const { hoverProps, isHovered } = useHover({})
  const [tapped, setTapped] = React.useState(false)
  const [copied, setCopied] = React.useState(false)
  const [expanded, setExpanded] = React.useState(false)
  const seat = block.kind === 'Seat'
  // A system notice is a muted one-line Marker that discloses its notice card; every other input is a full Card.
  const marker = block.kind === 'System'
  // A title that opens with its sender ('Session restored.') carries the w500 sender itself instead of repeating it.
  const title = block.text.split('\n')[0] ?? ''
  const senderLeads = title.toLowerCase().startsWith(block.sender.toLowerCase())
  React.useLayoutEffect(() => {
    const element = root.current
    if (element === null) return
    const adoptContent = () => {
      for (const region of element.querySelectorAll<HTMLElement>('[role="region"]')) {
        region.tabIndex = -1
        region.setAttribute('aria-label', turnNames[block.id]!)
      }
    }
    adoptContent()
    const observer = new MutationObserver(adoptContent)
    observer.observe(element, { childList: true, subtree: true })
    return () => observer.disconnect()
  }, [expanded, block.id, block.sender])
  const time = <time data-turn-time dateTime={block.at} {...stylex.props(styles.time)}>{clock(block.at)}</time>
  const copy = <TooltipTrigger delay={0}><Button data-copy data-affordance="reveal" aria-label={`${copied ? 'Copied' : 'Copy'} ${turnNames[block.id]}`} onPress={async () => { await navigator.clipboard.writeText(block.text); setCopied(true) }} {...stylex.props(styles.copy, styles.inlineCopy, styles.rowCopy, styles.layeredCopy)}><Icon name={copied ? 'check' : 'copy'} scale="status" /></Button><Tooltip placement="start" {...stylex.props(styles.tooltip)}>{copied ? 'Copied' : 'Copy message'}</Tooltip></TooltipTrigger>
  // Copy leads the meta side, so at rest its hidden slot reads as free space beside the identity, not a hole
  // between the kind word and the time.
  const inputMeta = <span data-input-meta {...stylex.props(styles.inputMeta)}>{copy}<span data-input-kind>{block.kind}{!block.verified ? ' · Unverified' : ''}</span><span data-meta-time>{time}</span></span>
  const content = seat
    ? <ThreadPrimitive.Unstable_MessageById messageId={block.id} components={answerComponents} />
    : <AgentMessage from={block.sender} via={{ _tag: 'SmallTalk' }} body={{ _tag: 'Full', markdown: block.text }} bodyStyles={[styles.unclampedBody]} subject={block.subject} variant={block.kind === 'Person' ? { _tag: 'Human', frame: { _tag: 'Card' } } : { _tag: 'Incoming' }} headerMeta={inputMeta} headerGlyph={{ _tag: 'Icon', name: glyphs[block.kind] }} frameStyles={block.kind === 'Person' ? [styles.humanRail] : []} />
  const body = <div data-turn-content {...stylex.props(styles.content)}>{content}</div>
  return <div ref={root} {...hoverProps} data-turn-block={block.id} data-turn-kind={block.kind} data-hovered={isHovered || undefined} data-tapped={tapped || undefined} onPointerDown={event => { if (event.pointerType === 'touch') setTapped(value => !value) }} {...stylex.props(styles.block, !seat && previous !== undefined && styles.turnStart, seat && styles.gutterBlock, marker && styles.quiet)}>
    {seat ? <div data-seat-gutter data-affordance="reveal" {...stylex.props(styles.seatMeta, styles.gutter)}><span data-meta-time>{time}</span>{copy}</div> : null}
    {marker ? <Disclosure isExpanded={expanded} onExpandedChange={setExpanded}><Button slot="trigger" {...stylex.props(styles.summary, styles.marker)}><span data-input-kind-icon aria-hidden="true" {...stylex.props(styles.summarySlot)}><Icon name={glyphs[block.kind]} scale="status" /></span>{senderLeads ? null : <strong data-input-sender {...stylex.props(styles.summarySender)}>{block.sender}</strong>}<span data-input-title {...stylex.props(styles.summaryText)}><span data-input-title-run {...stylex.props(styles.summaryRun)}>{senderLeads ? <><strong data-input-sender {...stylex.props(styles.leadSender)}>{title.slice(0, block.sender.length)}</strong>{title.slice(block.sender.length)}</> : title}</span></span><span data-input-chevron data-expanded={expanded || undefined} {...stylex.props(styles.summarySlot)}><Icon name="chevron-right" scale="disclosure" /></span></Button><DisclosurePanel>{expanded ? body : null}</DisclosurePanel></Disclosure> : body}
    {marker && !expanded ? <span data-collapsed-meta {...stylex.props(styles.collapsedMeta, styles.markerMeta)}>{copy}<span data-meta-time>{time}</span></span> : null}
  </div>
}

function Lane({ args }: { args: TurnsArgs }) {
  return <section data-lane data-scheme={args.scheme} data-width={args.width} aria-label={`${configuration} · ${args.scheme} · ${args.width}`} {...stylex.props(styles.lane)}>
    <h2 {...stylex.props(styles.laneTitle)}>{configuration} / {args.scheme} / {args.width}</h2>
    <EmbraceRuntimeProvider options={{ messages: replyItems, isRunning: false, onNew: async () => {} }}><ThreadPrimitive.Root {...stylex.props(styles.thread)}>{turnBlocks.map((block, index) => <React.Fragment key={block.id}>
      <Block block={block} previous={turnBlocks[index - 1]} />
      {index === 7 ? <div data-work-fixture {...stylex.props(styles.work)}><WorkLogV1 turn={workTurn} ariaLabel="Work log" /></div> : null}
    </React.Fragment>)}</ThreadPrimitive.Root></EmbraceRuntimeProvider>
  </section>
}
function Turns(initialArgs: TurnsArgs) {
  const [, updateStoryArgs] = useArgs<TurnsArgs>()
  const [probeArgs, setProbeArgs] = React.useState(initialArgs)
  const args = initialArgs.probe ? probeArgs : initialArgs
  // Probe interactions belong to their own mount, not Storybook's arg lifecycle.
  const updateArgs = initialArgs.probe
    ? (patch: Partial<TurnsArgs>) => setProbeArgs(current => ({ ...current, ...patch }))
    : updateStoryArgs
  return <main data-turns-explorer data-token-probe {...stylex.props(...baselineTheme, args.scheme === 'light' && lightTheme, styles.root, args.width === 1440 ? styles.wide : styles.narrow)}>
    <style>{projectionCss}</style>
    <h1 {...stylex.props(styles.title)}>Transcript turns</h1><p {...stylex.props(styles.explanation)}>Inputs trigger the turn; the owning seat replies. Time and Copy sit in each input's header and in the gutter right of each reply, revealed on hover, focus or tap, without a footer.</p>
    <div {...stylex.props(styles.controls)}>{(['dark', 'light'] as const).map(scheme => <Button key={scheme} aria-pressed={args.scheme === scheme} onPress={() => updateArgs({ scheme })} {...stylex.props(styles.control)}>{scheme}</Button>)}{([1440, 900] as const).map(width => <Button key={width} aria-pressed={args.width === width} onPress={() => updateArgs({ width })} {...stylex.props(styles.control)}>{width} lane</Button>)}</div>
    <Lane args={args} />
    {args.probe ? <section aria-label="Deliberately failing controls" data-controls {...stylex.props(styles.negativeControls)}>
      <div data-control-height {...stylex.props(styles.block)}><p>Reply with a legacy footer.</p><div data-meta-time {...stylex.props(styles.footerControl)}><time>6:32 AM</time></div></div>
      <div data-control-owner {...stylex.props(styles.block)}><p>Owning reply.</p><time data-turn-time {...stylex.props(styles.wrongOwner)}>6:32 AM</time></div><div data-control-next {...stylex.props(styles.block)}><p>Next input.</p></div>
      <div data-control-focus {...stylex.props(styles.block)}><span data-affordance="reveal" {...stylex.props(styles.hoverOnly)}>Hover-only time</span><Button {...stylex.props(styles.control)}>Control copy</Button></div>
      <div data-control-style>Unstyled token probe</div>
      <div data-control-transition><Button {...stylex.props(styles.control)}>Inert transition control</Button><Button aria-expanded={false} {...stylex.props(styles.control)}>Inert disclosure control</Button></div>
      <EmbraceRuntimeProvider options={{ messages: replyItems, isRunning: false, onNew: async () => {} }}><ThreadPrimitive.Root data-control-production-meta><ThreadPrimitive.Unstable_MessageById messageId={replyItems[0]!.id} components={metaControlComponents} /></ThreadPrimitive.Root></EmbraceRuntimeProvider>
      <div data-control-k3 {...stylex.props(styles.quietInputControl)}><span data-input-title>Quiet summary with no sender or kind cue</span></div>
      <div data-control-k3-stacked {...stylex.props(styles.stackedInputControl)}><span data-input-title><span data-input-title-run>Sender<br />Wrapped title</span></span></div>
      <FocusRemountControl />
      <div data-control-missing-time>Reply without its own timestamp.</div>
      <div data-control-nested-card data-fractal-card {...stylex.props(styles.inputFrame)}><header data-message-header>Projected sender/time</header><article data-fractal-card {...stylex.props(styles.inputFrame)}><header data-message-header>Duplicate card header</header>Nested input.</article></div>
      <div data-control-covered-line {...stylex.props(styles.block)}><p>First line under a toolbar.</p><div data-seat-gutter {...stylex.props(styles.seatMeta, styles.coveredLine)}>Covering metadata</div></div>
      <div data-control-long-final-line {...stylex.props(styles.longLineControl)}><div {...stylex.props(styles.block)}><p {...stylex.props(styles.controlLine)}>A preceding input whose final line runs to the right edge.</p></div><div {...stylex.props(styles.block, styles.seatRow)}><div data-seat-gutter {...stylex.props(styles.seatMeta)}>Meta</div><p {...stylex.props(styles.controlLine)}>Reply.</p></div></div>
      <div data-control-proximity {...stylex.props(styles.longLineControl)}><div {...stylex.props(styles.block, styles.inputFrame)}><p {...stylex.props(styles.controlLine)}>Trigger input.</p></div><div {...stylex.props(styles.block)}><div {...stylex.props(styles.seatMeta, styles.stackedRow)}>Meta</div><p data-control-reply {...stylex.props(styles.controlLine)}>Reply under the stacked 8px gap and 24px row.</p></div><div {...stylex.props(styles.block, styles.turnStart, styles.inputFrame)}><p {...stylex.props(styles.controlLine)}>Next turn.</p></div></div>
      <article data-control-header-wrap><header data-message-header {...stylex.props(styles.wrappedHeaderControl)}><span>From Workspace</span><span>Review · Small Talk</span></header><h2 data-message-subject {...stylex.props(styles.controlLine)}>Wrapped sender path</h2></article>
      <div data-control-identity-clip {...stylex.props(styles.identityClipControl)}><span {...stylex.props(styles.ellipsisLine)}>From <strong><span data-agent-id-trigger {...stylex.props(styles.atomicRun)}><span data-agent-segment="parent">Workspace</span>›<span data-agent-segment="leaf">Review</span></span></strong> · Small Talk</span></div>
      <span data-control-unicode-glyph>↪</span>
      <div data-control-gutter-over-text {...stylex.props(styles.block)}><p>First line under a gutter.</p><div data-seat-gutter {...stylex.props(styles.seatMeta, styles.coveredLine)}><span>6:32 AM</span></div></div>
      <div data-control-long-identities {...stylex.props(styles.longIdentityControl)}>{longIdentities.map(id => <AgentMessage key={id} from={id} via={{ _tag: 'SmallTalk' }} body={{ _tag: 'Full', markdown: 'Long identity.' }} headerMeta={controlMeta(false)} />)}</div>
      <div data-control-meta-yields {...stylex.props(styles.longIdentityControl)}><AgentMessage from={longIdentities[0]} via={{ _tag: 'SmallTalk' }} body={{ _tag: 'Full', markdown: 'Metadata that yields.' }} headerMeta={controlMeta(true)} /></div>
      <div data-control-copy-hit {...stylex.props(styles.block)}><div {...stylex.props(styles.seatMeta, styles.flatRow)}><Button data-copy aria-label="Copy hit-area control" {...stylex.props(styles.copy, styles.inlineCopy, styles.rowCopy)}><Icon name="copy" scale="status" /></Button></div><p {...stylex.props(styles.controlLine)}>Reply line under the Copy overhang.</p></div>
      <strong data-control-heavy-sender>Heavy sender</strong>
    </section> : null}
  </main>
}
// Scoped projections keep the production message / Markdown / work-log renderers.
// Every geometry value comes from composition tokens (including the locked hierarchy step).
const projectionCss = `
[data-turns-explorer] [data-turn-content] p:last-child { margin-bottom: 0; }
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
  controls: { display: 'flex', gap: s.md, marginBlock: s.md }, control: { padding: s.xs, color: ink.fg, backgroundColor: surface.rowActive, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, cursor: 'pointer', fontFamily: t.fontSans, ':focus-visible': { outlineColor: accent.primary, outlineStyle: 'solid', outlineWidth: g.focusRing } },
  // The lane is the size container for the seat gutter; the caption starts on the thread's text column.
  lane: { width: '100%', minWidth: 0, boxSizing: 'border-box', containerType: 'inline-size' }, laneTitle: { width: g.lane, maxWidth: '100%', marginInline: 'auto', paddingLeft: g.textColumn, boxSizing: 'border-box', marginBlock: s.lg, fontSize: t.metaSize, color: ink.fgMuted, fontWeight: t.weightMedium },
  thread: { width: g.lane, maxWidth: '100%', marginInline: 'auto', paddingLeft: g.textColumn, boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.md },
  block: { position: 'relative', minWidth: 0, color: ink.fg, fontSize: t.bodySize, lineHeight: t.bodyLeading },
  turnStart: { marginTop: s.xl },
  inputFrame: { paddingBlock: s.md, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.raised },
  humanRail: { borderLeftWidth: g.focusRing, borderLeftColor: accent.primary },
  unclampedBody: { maxHeight: 'none', overflowY: 'visible' },
  quiet: { paddingBlock: 0, borderWidth: 0, backgroundColor: surface.transparent },
  content: { minWidth: 0 },
  time: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, whiteSpace: 'nowrap', fontVariantNumeric: 'tabular-nums' },
  copy: { position: 'absolute', top: s.xs, right: 0, minHeight: g.controlSm, padding: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.raised, color: ink.fgMuted, cursor: 'pointer', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  inlineCopy: { position: 'static', minHeight: 0, padding: 0, backgroundColor: surface.transparent },
  // The seat row is one 16px meta line; Copy keeps a 24px hit area by overhanging it equally above and below.
  rowCopy: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minWidth: g.controlSm, minHeight: g.controlSm, marginBlock: `calc((${t.metaLeading} - ${g.controlSm}) / 2)` },
  // Header and gutter Copy draw a 14px glyph in a 16px slot but hit across 24x24: the box overhangs the slot equally on
  // every side and is layered above its neighbours so the overhang stays hittable. Relative positioning must not pick up
  // the overlay Copy's top offset.
  layeredCopy: { position: 'relative', zIndex: 1, top: 0, marginInline: `calc(-1 * ${s.xs})` },
  // Header metadata never yields: the kind word (Unverified is the only non-silent state), Copy and time keep their
  // width, and only the sender's identity run narrows.
  // The extra 2px on top of the header's 6px gap keeps 8px between the identity and the metadata.
  inputMeta: { marginLeft: 'auto', paddingLeft: s.xs2, display: 'flex', alignItems: 'center', gap: s.md, flex: '0 0 auto', minWidth: 'max-content' },
  inputMetaYields: { flex: '0 1 auto', minWidth: 0 },
  inputKindYields: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  // The width of a card at a 440px viewport.
  longIdentityControl: { width: '360px', maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: s.md },
  // The seat gutter: time over Copy beside the run's first line, right of the text column, once the lane holds the 64px
  // gutter plus an 8px gap; it takes no row, so the block keeps the plain 8px gap. Narrower lanes keep it in flow as one
  // plain 16px meta line, right-aligned above the run, that takes 4px of the 8px block gap instead of stacking on it, so
  // a reply stays closer to its own trigger than to the next turn and the line clears the trigger card's bottom edge.
  // The gutter is layered above the reply so Copy's overhang stays hittable over the reply's first line box.
  // Container queries cannot read variables, so the width is a literal that gutterFit mirrors for the plays.
  seatMeta: { position: 'relative', zIndex: 1, width: 'max-content', maxWidth: '100%', marginLeft: 'auto', height: t.metaLeading, boxSizing: 'border-box', display: 'flex', alignItems: 'center', gap: s.md },
  flatRow: { position: 'static', zIndex: 'auto' },
  seatRow: { marginTop: `calc(-1 * ${s.xs})` },
  gutter: { position: { default: 'relative', '@container (min-width: 880px)': 'absolute' }, top: { default: 'auto', '@container (min-width: 880px)': `calc((${t.bodyLeading} - ${t.metaLeading}) / 2)` }, left: { default: 'auto', '@container (min-width: 880px)': `calc(100% + ${s.md})` }, marginLeft: { default: 'auto', '@container (min-width: 880px)': 0 }, width: { default: 'max-content', '@container (min-width: 880px)': geometry.metaGutter }, height: { default: t.metaLeading, '@container (min-width: 880px)': 'auto' }, flexDirection: { default: 'row', '@container (min-width: 880px)': 'column' }, alignItems: { default: 'center', '@container (min-width: 880px)': 'flex-start' }, gap: { default: s.md, '@container (min-width: 880px)': s.xs } },
  gutterBlock: { marginTop: { default: `calc(-1 * ${s.xs})`, '@container (min-width: 880px)': 0 } },
  leadSender: { fontWeight: t.weightMedium },
  stackedRow: { height: g.toolRow },
  identityClipControl: { width: g.controlLg, maxWidth: '100%', fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap' },
  ellipsisLine: { display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  atomicRun: { display: 'inline-flex', alignItems: 'center' },
  coveredLine: { position: 'absolute', top: 0, bottom: 'auto', left: 0, right: 'auto' },
  longLineControl: { width: g.previewMax, maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: s.md },
  controlLine: { margin: 0, textAlign: 'right' },
  wrappedHeaderControl: { display: 'flex', flexDirection: 'column', height: g.icon, fontSize: t.metaSize, lineHeight: t.metaLeading },
  collapsedMeta: { position: 'absolute', right: 0, top: 0, display: 'flex', gap: s.md, alignItems: 'center', height: g.toolRow },
  markerMeta: { height: t.uiLeading },
  summary: { display: 'flex', flexWrap: 'nowrap', alignItems: 'center', gap: s.sm, width: '100%', padding: 0, paddingRight: `min(${geometry.metaRoom}, 25%)`, minHeight: g.toolRow, lineHeight: g.toolRow, whiteSpace: 'nowrap', borderWidth: 0, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, textAlign: 'left', cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, summaryText: { flex: '0 1 auto', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 },
  // The run is its own ellipsis box; an inline-block run inside an ellipsis wrapper overflowed it and clipped mid-glyph.
  summaryRun: { display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  summarySender: { fontWeight: t.weightMedium, flexShrink: 0, maxWidth: '40%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  summarySlot: { width: g.icon, height: g.icon, flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' },
  marker: { minHeight: t.uiLeading, lineHeight: t.uiLeading, fontSize: t.metaSize, color: ink.fgMuted },
  stackedInputControl: { width: geometry.metaRoom, fontSize: t.uiSize, lineHeight: g.toolRow },
  quietInputControl: { color: ink.fgMuted, height: g.iconSm, overflow: 'hidden', fontSize: t.metaSize, lineHeight: t.metaLeading },
  work: { marginTop: s.md }, tooltip: { padding: s.md, backgroundColor: surface.raised, color: ink.fg, fontSize: t.metaSize, borderRadius: r.sm },
  negativeControls: { marginTop: s.section, display: 'flex', flexDirection: 'column', gap: s.section }, footerControl: { height: g.toolRow }, wrongOwner: { position: 'absolute', top: '100%', marginTop: s.section, fontSize: t.metaSize }, hoverOnly: { opacity: { default: 0, ':hover': 1 } },
})

const meta = { title: 'Fractal UI/Explore/Transcript Turns', component: Turns, render: Turns, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', width: 1440, probe: false }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: [1440, 900], control: 'radio' }, probe: { table: { disable: true }, control: false } } } satisfies Meta<typeof Turns>
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
    if (text.textContent?.trim() === '' || text.parentElement?.closest('[data-meta-time], [data-copy], [data-seat-gutter]')) continue
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
    if (walker.currentNode.textContent?.trim() === '' || walker.currentNode.parentElement?.closest('[data-seat-gutter]')) continue
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
  // A truncated ancestor shows at least two letters before its ellipsis; a one-letter stub folds into '…' instead.
  for (const segment of trigger.querySelectorAll<HTMLElement>('[data-agent-segment="parent"]')) {
    const text = segment.firstChild
    if (text?.nodeType !== Node.TEXT_NODE || (text.textContent ?? '').length < 2 || segment.scrollWidth <= segment.clientWidth) continue
    const range = segment.ownerDocument.createRange()
    range.setStart(text, 0)
    range.setEnd(text, 2)
    if (segment.clientWidth < range.getBoundingClientRect().width + parseFloat(getComputedStyle(segment).fontSize) / 2) faults.push('one-letter ancestor stub')
  }
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
  // The channel shows whole or not at all: no '·…' stub.
  const channel = header.querySelector<HTMLElement>('[data-message-channel]')
  if (channel !== null && getComputedStyle(channel).visibility !== 'hidden' && channel.scrollWidth > channel.clientWidth + 0.5) faults.push('channel truncated')
  // 8px between the identity and the metadata; header Copy hits across 24x24.
  const meta = header.querySelector<HTMLElement>(':scope > [data-input-meta]')
  const beforeMeta = meta?.previousElementSibling
  if (meta && beforeMeta && meta.getBoundingClientRect().left + parseFloat(getComputedStyle(meta).paddingLeft) - beforeMeta.getBoundingClientRect().right < 7.5) faults.push('identity to meta gap under 8px')
  for (const copy of header.querySelectorAll<HTMLElement>('[data-copy]')) {
    const box = copy.getBoundingClientRect()
    if (box.width < 24 || box.height < 24) faults.push('header Copy under 24x24')
    const row = (meta ?? header).getBoundingClientRect()
    if (Math.abs(box.top + box.height / 2 - (row.top + row.height / 2)) > 1) faults.push('header Copy off its row centre')
  }
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
    // Chrome hit-tests the corner arc on device pixels, so the last pixel ring along each arc may fall either side of it.
    if (dx > 0 && dy > 0 && Math.hypot(dx, dy) > radius - 1) continue
    total++
    const target = control.ownerDocument.elementFromPoint(box.left + x, box.top + y)
    if (target !== null && control.contains(target)) hit++
  }
  return { width: box.width, height: box.height, total, hit }
}
function reservedMetaHeight(block: HTMLElement) {
  const height = block.getBoundingClientRect().height
  const metadata = Array.from(block.querySelectorAll<HTMLElement>('[data-meta-time], [data-copy], [data-seat-gutter], time'))
  const previous = metadata.map(element => element.style.display)
  metadata.forEach(element => { element.style.display = 'none' })
  const bare = block.getBoundingClientRect().height
  metadata.forEach((element, index) => { element.style.display = previous[index] ?? '' })
  return height - bare
}
/** The seat gutter never lays over the run's text, and the lane never clips it. */
function gutterFaults(block: HTMLElement, gutter: HTMLElement, bounds: DOMRect) {
  const faults: string[] = []
  const box = gutter.getBoundingClientRect()
  if (visibleTextRects(block).some(line => intersects(box, line))) faults.push('gutter over text')
  if (box.left < bounds.left - 0.5 || box.right > bounds.right + 0.5) faults.push('gutter clipped')
  return faults
}
/** GR: time over Copy beside the run's first line when the lane holds the gutter, else the in-flow 16px row above the run. */
async function assertGutter(lane: HTMLElement) {
  const bounds = lane.getBoundingClientRect()
  const beside = bounds.width >= gutterFit
  const seats = Array.from(lane.querySelectorAll<HTMLElement>('[data-turn-block][data-turn-kind="Seat"]'))
  await expect(seats.length).toBeGreaterThan(0)
  for (const seat of seats) {
    const gutters = seat.querySelectorAll<HTMLElement>('[data-seat-gutter]')
    await expect(gutters).toHaveLength(1)
    const gutter = gutters[0]!
    await expect(gutter.querySelectorAll('[data-copy]')).toHaveLength(1)
    await expect(gutterFaults(seat, gutter, bounds)).toEqual([])
    const box = gutter.getBoundingClientRect(), run = seat.getBoundingClientRect()
    const firstLineRange = lane.ownerDocument.createRange()
    firstLineRange.selectNodeContents(seat.querySelector('[data-turn-content] p')!)
    const firstLine = firstLineRange.getClientRects()[0]!
    const previous = (seat.previousElementSibling as HTMLElement).getBoundingClientRect()
    await expect(getComputedStyle(gutter).position === 'absolute', `gutter beside the text at ${bounds.width}px`).toBe(beside)
    if (beside) {
      const copy = gutter.querySelector<HTMLElement>('[data-copy]')!
      const time = gutter.querySelector<HTMLElement>('[data-turn-time]')!
      const tips = () => Array.from(lane.ownerDocument.querySelectorAll<HTMLElement>('[role="tooltip"]')).filter(tip => tip.textContent === 'Copy message')
      const assertClearTime = async (modality: string) => {
        await waitFor(() => {
          expect(tips(), `Copy tooltip opens on ${modality}`).toHaveLength(1)
          expect(intersects(tips()[0]!.getBoundingClientRect(), time.getBoundingClientRect()), `Copy tooltip clears time on ${modality}`).toBe(false)
        })
        // Move the real tooltip over the time: the same geometry predicate must reject it.
        const tip = tips()[0]!
        const original = tip.style.cssText
        const rect = time.getBoundingClientRect()
        Object.assign(tip.style, { position: 'fixed', inset: 'auto', left: `${rect.left}px`, top: `${rect.top}px`, transform: 'none' })
        await expect(intersects(tip.getBoundingClientRect(), rect), 'covered-time negative control').toBe(true)
        tip.style.cssText = original
        await expect(intersects(tip.getBoundingClientRect(), rect)).toBe(false)
      }
      await userEvent.keyboard('{Escape}')
      await userEvent.tab()
      copy.focus()
      await assertClearTime('keyboard focus')
      copy.blur()
      await waitFor(() => expect(tips()).toHaveLength(0))
      copy.scrollIntoView({ block: 'center' })
      await new Promise<void>(resolve => lane.ownerDocument.defaultView!.requestAnimationFrame(() => lane.ownerDocument.defaultView!.requestAnimationFrame(() => resolve())))
      await userEvent.pointer({ keys: '[MouseLeft]', target: lane.ownerDocument.body })
      await userEvent.hover(copy)
      await assertClearTime('hover')
      await userEvent.unhover(copy)
      await waitFor(() => expect(tips()).toHaveLength(0))
      // Right of the run beside its first line, taking no row: the block keeps the plain 8px gap.
      await expect(box.left).toBeGreaterThanOrEqual(run.right + 8 - 0.5)
      await expect(box.top).toBeGreaterThanOrEqual(firstLine.top - 0.5)
      await expect(box.top).toBeLessThan(firstLine.bottom)
      const originalTransform = gutter.style.transform
      gutter.style.transform = `translateY(${getComputedStyle(seat.querySelector('[data-turn-content] p')!).lineHeight})`
      try {
        await expect(gutter.getBoundingClientRect().top, 'one-line-offset gutter must fail first-line alignment').toBeGreaterThanOrEqual(firstLine.bottom)
      } finally { gutter.style.transform = originalTransform }
      await expect(gutter.getBoundingClientRect().top).toBeLessThan(firstLine.bottom)
      await expect(Math.abs(run.top - previous.bottom - 8)).toBeLessThan(0.5)
    } else {
      // Fallback: one 16px row above the run, ending on the run's right edge.
      await expect(box.height).toBe(16)
      await expect(box.bottom).toBeLessThanOrEqual(firstLine.top + 0.5)
      await expect(Math.abs(box.right - run.right)).toBeLessThan(0.5)
    }
  }
}
async function assertProjection(canvasElement: HTMLElement, expandedBlock?: HTMLElement) {
  await waitFor(() => {
    const lanes = canvasElement.querySelectorAll('[data-lane]')
    expect(lanes).toHaveLength(1)
    for (const lane of lanes) expect(lane.querySelectorAll('[data-turn-content] [data-testid="agent-message"]')).toHaveLength(replyItems.length)
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
        await expect(header.querySelector('svg')).not.toBeNull()
        for (const glyph of header.querySelectorAll(':scope > span[aria-hidden="true"]')) await expect(glyph.querySelector('svg')).not.toBeNull()
        const subject = header.parentElement?.querySelector(':scope > [data-message-subject]')
        if (subject) await expect(headerCollisions(header, subject)).toEqual([])
        await expect(headerFaults(header as HTMLElement)).toEqual([])
      }
      const overlay = Array.from(block.querySelectorAll<HTMLElement>('[data-seat-gutter]')).find(row => getComputedStyle(row).position !== 'absolute')
      if (overlay !== undefined) {
        const firstLine = block.querySelector('[data-turn-content] p')!
        await expect(overlay.getBoundingClientRect().bottom).toBeLessThanOrEqual(firstLine.getBoundingClientRect().top)
        await expect(overlay.getBoundingClientRect().height).toBe(16)
        const previous = block.previousElementSibling as HTMLElement
        // The row takes 4px of the 8px gap, clear of the trigger card's bottom edge.
        await expect(Math.abs(block.getBoundingClientRect().top - previous.getBoundingClientRect().bottom - 4)).toBeLessThan(0.5)
        if (previous.getAttribute('data-turn-kind') !== 'Seat') {
          const nextTurn = Array.from(lane.querySelectorAll<HTMLElement>('[data-turn-block]:not([data-turn-kind="Seat"])')).find(candidate => candidate.getBoundingClientRect().top > block.getBoundingClientRect().bottom - 0.5)
          if (nextTurn !== undefined) {
            // The reply starts at its own text; the meta row above it is not reply text.
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
    // Person, agent mail and relays are full native Cards; the system notice is a Marker; seat prose has no sender line.
    for (const input of lane.querySelectorAll('[data-turn-kind="Person"], [data-turn-kind="Relay"], [data-turn-kind="Agent"]')) {
      await expect(input.querySelectorAll('[data-message-header]')).toHaveLength(1)
      await expect(input.querySelector('[data-input-kind]')).not.toBeNull()
      await expect(input.querySelector('[data-message-content]')).not.toBeNull()
      await expect(input.querySelector('[data-input-title]')).toBeNull()
    }
    await expect(lane.querySelectorAll('[data-turn-kind="Seat"] [data-message-header]')).toHaveLength(0)
    for (const block of lane.querySelectorAll<HTMLElement>('[data-turn-kind="System"]')) {
      const trigger = block.querySelector<HTMLElement>('button[aria-expanded="false"]')
      if (trigger === null) continue
      await expect(getComputedStyle(trigger.querySelector('[data-input-title]')!).color).toBe(getComputedStyle(canvasElement.querySelector('[data-control-k3]')!).color)
      await expect(trigger.querySelector('[data-input-sender]')!.textContent?.trim().length).toBeGreaterThan(0)
      await expect(getComputedStyle(trigger.querySelector('[data-input-sender]')!).fontWeight).toBe('500')
      // A title that opens with its sender carries it once.
      await expect(trigger.querySelectorAll('[data-input-sender]')).toHaveLength(1)
      await expect(trigger.querySelector('[data-input-kind-icon] svg')).not.toBeNull()
      await expect(trigger.getBoundingClientRect().height).toBeGreaterThanOrEqual(20)
      await expect(trigger.getBoundingClientRect().height).toBeLessThanOrEqual(parseFloat(getComputedStyle(trigger).lineHeight))
      await expect(trigger.querySelector<HTMLElement>('[data-input-title-run]')!.getClientRects()).toHaveLength(1)
    }
    await assertGutter(lane as HTMLElement)
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
  await expect(coveredControl.querySelector('[data-seat-gutter]')!.getBoundingClientRect().bottom).toBeGreaterThan(coveredControl.querySelector('p')!.getBoundingClientRect().top)
  await expect(lastLineRects(coveredControl as HTMLElement).some(line => intersects(coveredControl.querySelector('[data-seat-gutter]')!.getBoundingClientRect(), line))).toBe(true)
  // Long final line: the preceding block's last line reaches the right edge. The row toolbar clears it;
  // the rejected zero-height overhang (absolute, 32px past the run) erases its end.
  const longLineControl = canvasElement.querySelector<HTMLElement>('[data-control-long-final-line]')!
  const longLineToolbar = longLineControl.querySelector<HTMLElement>('[data-seat-gutter]')!
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
  // A gutter laid over the run's first line fails.
  const gutterControl = canvasElement.querySelector<HTMLElement>('[data-control-gutter-over-text]')!
  await expect(gutterFaults(gutterControl, gutterControl.querySelector<HTMLElement>('[data-seat-gutter]')!, gutterControl.getBoundingClientRect())).toContain('gutter over text')
  // Header and gutter Copy are hittable across their whole 24px box; without layering the reply's line box takes the overhang.
  for (const lane of canvasElement.querySelectorAll('[data-lane]')) {
    const headerCopy = lane.querySelector<HTMLElement>('[data-message-header] [data-copy]')
    if (headerCopy !== null) {
      const header = hitArea(headerCopy)
      await expect([header.width, header.height]).toEqual([24, 24])
      await expect(header.hit, 'every point of the header Copy box hits Copy').toBe(header.total)
    }
    const rowCopy = lane.querySelector<HTMLElement>('[data-seat-gutter] [data-copy]')
    if (rowCopy === null) continue
    const area = hitArea(rowCopy)
    await expect([area.width, area.height]).toEqual([24, 24])
    await expect(area.hit, 'every point of the gutter Copy box hits Copy').toBe(area.total)
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
      const seatToolbar = Array.from(block.querySelectorAll('[data-seat-gutter]')).find(row => getComputedStyle(row).position !== 'absolute')
      const inFlow = seatToolbar !== undefined
      await expect(reservedMetaHeight(block)).toBe(inFlow ? 16 : 0)
      // Every block carries exactly its own time.
      const name = turnNames[block.getAttribute('data-turn-block')!]!
      await expect(block.querySelector('[data-copy]')).toHaveAccessibleName(`Copy ${name}`)
      for (const region of block.querySelectorAll('[role="region"]')) await expect(region).toHaveAccessibleName(name)
      await expect(block.querySelectorAll('time[data-turn-time]')).toHaveLength(1)
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
    await waitFor(() => expect(Array.from(block.querySelectorAll('[data-affordance="reveal"]')).map(element => getComputedStyle(element).opacity), `hover reveals ${block.getAttribute('data-turn-block')} (hovered=${block.hasAttribute('data-hovered')})`).toEqual(focusOpacity))
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
    // At 440 the parent folds rather than show one letter; the visible '…' is decorative and the hidden text keeps the identity.
    await expect(identity.textContent?.replace(/…/g, '')).toBe('Workspace.Review')
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
  await step('Expand the system Marker into its notice card; inert disclosure stays closed', async () => {
    const disclosureControl = canvas.getByRole('button', { name: 'Inert disclosure control' })
    await userEvent.click(disclosureControl)
    await expect(disclosureControl).toHaveAttribute('aria-expanded', 'false')
    await expect(control.querySelector('[data-turn-content]')).toBeNull()
    const block = canvasElement.querySelector<HTMLElement>('[data-lane] [data-turn-kind="System"]')!
    const trigger = block.querySelector<HTMLElement>('button[aria-expanded]')!
    await expect(trigger).toHaveAttribute('aria-expanded', 'false')
    await userEvent.click(trigger)
    await waitFor(() => {
      expect(trigger).toHaveAttribute('aria-expanded', 'true')
      expect(block.querySelector('[data-turn-content] [data-message-header] time[data-turn-time]')).not.toBeNull()
      expect(block.querySelector('[data-collapsed-meta]')).toBeNull()
    })
    await assertProjection(canvasElement, block)
  })
}
// One public story; scheme and lane width are URL args. Probe stories carry the plays; the narrow harness also runs
// the 1440 probes at a 440 viewport.
export const Exploration: Story = {}
export const Dark1440Probe: Story = { args: { scheme: 'dark', width: 1440, probe: true }, play }
export const Light1440Probe: Story = { args: { scheme: 'light', width: 1440, probe: true }, play }
export const Dark900Probe: Story = { args: { scheme: 'dark', width: 900, probe: true }, play }
export const Light900Probe: Story = { args: { scheme: 'light', width: 900, probe: true }, play }
