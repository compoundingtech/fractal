// Explicit-clock presentation only.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Icon } from '../composition/Icons'
import { Button, Tooltip, TooltipTrigger, VisuallyHidden } from 'react-aria-components'
import { accentVars as accent, borderVars as border, elevationVars as elevation, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { usageScopeLabel, type SidebarActions, type SidebarAgentRow as Row } from './model'
import { Highlight, sidebarRowTitle, sidebarSubagentObservationText, usageText, tokenCount, spendCount, openRow } from './rowPresentation.tsx'
import { AgentHoverCard, AgentRowActions } from './DeferredRowActions.tsx'
import { SidebarStatus, type DiscRefinement, type GlyphVariant } from './SidebarStatus'
import { SidebarTime, type SidebarClock } from './SidebarTime'
import { SidebarDuration } from './SidebarDuration'
import { formatDuration } from '../../time/human-time'
import { SidebarRowSignals, attachSidebarLine1Fit } from './SidebarRowSignals'
import { useSidebarHoverIntent } from './useSidebarHoverIntent'
import { useTouchLayout } from './useTouchLayout'

export type RowVariant = 'SR-1' | 'SR-2' | 'SR-3'
export type RowLayout = 'SR2-A' | 'SR2-B' | 'SR2-C'
export type ExtraSignal = 'X-needs' | 'X-unread' | 'X-work' | 'X-model' | 'X-pr' | 'X-subs'
export const defaultExtraSignals: readonly ExtraSignal[] = ['X-needs', 'X-unread', 'X-work', 'X-subs']
export const rowDescriptions: Record<RowVariant, string> = { 'SR-1': '32px compact row; complete reported facts in accessible details', 'SR-2': '48px shared two-line layouts A/B/C; B shows current work and retains spend in accessible details', 'SR-3': '68px three-line row with extra work metadata; complete details remain accessible' }
export type SidebarAgentRowProps = SidebarClock & {
  readonly item: Row
  readonly variant?: RowVariant
  readonly layout?: RowLayout
  readonly glyph?: GlyphVariant
  readonly extraSignals?: readonly ExtraSignal[]
  readonly discRefinement?: DiscRefinement
  /** Details omits unreported status values without changing sidebar defaults. */
  readonly reportedStatusOnly?: boolean
  readonly active?: boolean
  /** Overrides `SidebarQueryContext` for this row and its nested rows. A per-row prop re-renders every row on each keystroke; prefer the context. */
  readonly query?: string
  readonly actions?: SidebarActions
  readonly onOpen?: (row: Row) => void
  readonly onMarkRead?: (ref: string) => void
  readonly onToggleChildren?: (ref: string) => void
  readonly inTree?: boolean
  readonly treeExpanded?: boolean
}
/** Reported metadata stays accessible; fields the source never reported are omitted, never described as unknown. */
export function sidebarRowDescription(row: Row, reportedStatusOnly = false): string {
  const parts: string[] = [row.title]
  if (row.description !== undefined) parts.push(row.description)
  parts.push(`Host: ${row.host}.${reportedStatusOnly && row.status === 'unobserved' ? '' : ` Status: ${row.statusLabel};`} ${row.freshness} observation`)
  if (row.usage._tag === 'Known') parts.push(usageText(row.usage))
  if (row.duration._tag === 'Known') parts.push(`${row.duration.scope === '24h-activity-span' ? '24h activity span' : 'Lifetime duration'}: ${formatDuration(row.duration.ms)}`)
  if (row.lastTurn._tag === 'Known') parts.push(`${row.lastTurn.kind === 'activity' ? 'Since activity' : 'Last completed turn'}: ${new Date(row.lastTurn.at).toISOString()}`)
  if (row.harness !== undefined) parts.push(`Harness: ${row.harness}`)
  if (row.model !== undefined) parts.push(`Model: ${row.model}`)
  if (row.needsMe !== undefined) parts.push(row.needsMe ? 'Needs your attention' : 'No attention requested')
  if (row.unread !== undefined) parts.push(`Unread: ${row.unread}`)
  if (row.statusSince !== undefined) parts.push(`Status boundary: ${new Date(row.statusSince).toISOString()}`)
  if (row.lastActivityAt !== undefined) parts.push(`Last activity: ${new Date(row.lastActivityAt).toISOString()}`)
  if (row.branch !== undefined) parts.push(`Branch: ${row.branch}`)
  if (row.worktree !== undefined) parts.push(`Worktree: ${row.worktree}`)
  if (row.pullRequest !== undefined) parts.push(`Pull request: #${row.pullRequest.number}, ${row.pullRequest.title}, ${row.pullRequest.state}, ${row.pullRequest.ref}`)
  if (row.terminal !== undefined) parts.push(`Terminal: ${row.terminal}`)
  if (row.mission !== undefined) parts.push(`Mission: ${row.mission}`)
  if (row.missionContext?._tag === 'Mission') {
    const { progress, step, queued } = row.missionContext
    if (progress._tag === 'Known') parts.push(`${progress.done} of ${progress.total} steps`)
    if (step._tag === 'Step') parts.push(`Step: ${step.name}`)
    if (step._tag === 'Waiting') parts.push(`Waiting for you: ${step.handle === undefined ? '' : `${step.handle} `}${step.question}`)
    if (queued.length > 0) parts.push(`Queued missions: ${queued.map(mission => mission.slug).join(', ')}`)
  }
  parts.push(row.subagentObservation?._tag === 'Reported' ? `Subagents: ${row.children.length}` : sidebarSubagentObservationText(row.subagentObservation))
  parts.push(`ID: ${row.ref}`)
  return `${parts.join('. ')}.`
}
export const SidebarAgentRow = React.memo(function SidebarAgentRow({ item, variant = 'SR-2', layout = 'SR2-A', glyph = 'SG-1', extraSignals = defaultExtraSignals, discRefinement = 'G1', reportedStatusOnly = false, active = false, query, now, actions, onOpen, onMarkRead, onToggleChildren, inTree = false, treeExpanded }: SidebarAgentRowProps) {
  const title = sidebarRowTitle(item)
  // Touch (coarse pointer or touch host): the actions menu and the subagent disclosure leave the dense row's corners for their own trailing 44px columns; the title truncates earlier instead.
  const touch = useTouchLayout()
  const host = item.host
  const showStatus = !reportedStatusOnly || item.status !== 'unobserved'
  const metadata = React.useMemo(() => sidebarRowDescription(item, reportedStatusOnly), [item, reportedStatusOnly])
  const titleId = React.useId()
  const statusWordId = React.useId()
  const descriptionId = React.useId()
  const clock: SidebarClock = { now: now! }
  const callbacks = actions
  const usd = item.usage._tag === 'Known' ? item.usage.usd : undefined
  const tokens = item.usage._tag === 'Known' ? item.usage.tokens : undefined
  const usageScope = item.usage._tag === 'Known' ? usageScopeLabel(item.usage.scope) : undefined
  const expanded = treeExpanded ?? false
  const canOpen = callbacks?.select !== undefined || onOpen !== undefined
  const rowLayout = variant === 'SR-2' ? layout : 'SR2-A'
  const rowGlyph = variant === 'SR-2' ? 'SG-1' : glyph
  const menuTrigger = React.useRef<HTMLButtonElement>(null)
  const menuOpen = React.useRef(false)
  const tooltipAnchor = React.useRef<HTMLDivElement>(null)
  const { hoverEnabled, hoverOpen, setHoverOpen, hoverIntent: scheduleHover } = useSidebarHoverIntent()
  const [rowHovered, setRowHovered] = React.useState(false)
  // The hover card anchors to the measured row frame so it follows the row, not its hidden trigger.
  const rowAnchor = React.useCallback((node: HTMLDivElement | null) => { tooltipAnchor.current = node; return attachSidebarLine1Fit(node) }, [])
  const hoverIntent = (open: boolean) => {
    // Portalled menu focus bubbles through the row; it must not open a second overlay that consumes Escape.
    if (open && menuOpen.current) return
    scheduleHover(open)
  }
  const prefetch = () => { if (!inTree) callbacks?.prefetch?.(item.ref) }
  const open = () => openRow(item, callbacks, onOpen)
  const quickOpen = hoverEnabled && canOpen && rowHovered
  const spendFields = item.usage._tag === 'Unknown' ? null : <span {...stylex.props(styles.metric)}>{usd !== undefined && <span data-row-field="total-usd" aria-label={`${usageScope}: $${usd}`}>{spendCount.format(usd)}</span>}<span data-row-field="total-tokens" data-line1-drop={usd === undefined ? undefined : 'tokens'} aria-label={`${usageScope}: ${tokens} tokens`}>{tokenCount.format(item.usage.tokens)}</span></span>
  const durationField = item.duration._tag === 'Unknown' ? null : <span data-row-field="total-duration"><SidebarDuration duration={item.duration} /></span>
  const lastTurnField = item.lastTurn._tag === 'Unknown' ? null : <span data-row-field="last-turn"><SidebarTime at={item.lastTurn.at} {...clock} kind={item.lastTurn.kind === 'turn-completed' ? 'turn' : 'activity'} compact /></span>
  const content = <>
    <span data-row-column="title" {...stylex.props(styles.title, !touch && variant === 'SR-2' && item.children.length > 0 && styles.hasChildren)}>{showStatus && rowGlyph === 'SG-2' && <span id={statusWordId} data-row-column="status-word" {...stylex.props(styles.statusWord)}>{item.statusLabel}</span>}<span id={titleId} data-row-column="title-text" {...stylex.props(styles.name)}><Highlight value={title} query={query} /></span></span>
  </>
  // The subtitle and SR-3 metadata are painted siblings of the row button, not its children: the button's visible text stays exactly its name (label-in-name) and every fact remains in the description.
  const subtitle = <>
    {variant !== 'SR-1' && <SidebarRowSignals>
      {showStatus && rowLayout === 'SR2-C' && <span data-row-field="status-label" data-row-retention="7">{item.statusLabel}</span>}
      {extraSignals.includes('X-work') && item.description !== undefined && item.description.trim()!==title.trim() && <span data-row-field="current-work" data-row-signal="X-work" data-row-retention="2">{item.description}</span>}
      <span data-row-field="host" data-row-retention="3" aria-label={`Host ${host}`}>{host}</span>
      {rowLayout === 'SR2-A' && <span data-row-retention="5">{spendFields}</span>}
      {rowLayout !== 'SR2-B' && <span data-row-retention="4">{durationField}</span>}
      {rowLayout !== 'SR2-A' && <span data-row-retention="6">{lastTurnField}</span>}
      {extraSignals.includes('X-model') && (item.model ?? item.harness) !== undefined && <span data-row-signal="X-model" data-row-retention="1">{item.model ?? item.harness}</span>}
      {extraSignals.includes('X-pr') && (item.pullRequest ?? item.branch ?? item.worktree) !== undefined && <span data-row-signal="X-pr" data-row-retention="0">{item.pullRequest === undefined ? item.branch ?? item.worktree : `#${item.pullRequest.number}`}</span>}
    </SidebarRowSignals>}
    <span {...stylex.props(styles.metadata, touch && styles.metadataNarrow, variant !== 'SR-3' && styles.hidden)}>{[item.description, item.harness, item.model, item.branch, item.worktree, item.pullRequest === undefined ? undefined : `PR #${item.pullRequest.number}`].filter((part): part is string => part !== undefined).join(' / ')}</span>
  </>
  const rowStyle = stylex.props(styles.row, touch && styles.rowNarrow)
  return <div data-observation-freshness={item.freshness} data-wf-agent-ref={inTree ? undefined : item.ref} {...stylex.props(styles.wrapper)} onMouseEnter={() => { setRowHovered(true); prefetch(); hoverIntent(true) }} onMouseLeave={() => { setRowHovered(false); hoverIntent(false) } } onFocusCapture={() => { setRowHovered(true); prefetch(); hoverIntent(true) }} onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) { setRowHovered(false); hoverIntent(false) } }} onContextMenu={event => { event.preventDefault(); event.stopPropagation(); setHoverOpen(false); menuTrigger.current?.click() }} onKeyDown={event => { if (event.key === 'Escape') setHoverOpen(false); if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) { event.preventDefault(); event.stopPropagation(); setHoverOpen(false); menuTrigger.current?.click() } }}>
    <div ref={rowAnchor} data-testid="taste-agent-row" data-density={variant} data-layout={rowLayout} data-glyph={rowGlyph} data-disc-refinement={discRefinement} data-extra-signals={extraSignals.join(' ')} data-needs-me={item.needsMe} data-narrow={touch || undefined} data-unknown-row-fields={[...(item.usage._tag === 'Unknown' ? ['total-usd', 'total-tokens'] : []), ...(item.duration._tag === 'Unknown' ? ['total-duration'] : []), ...(item.lastTurn._tag === 'Unknown' ? ['last-turn'] : []), ...(item.description === undefined ? ['current-work'] : [])].join(' ')} data-required-row-fields={variant === 'SR-2' ? 'true' : undefined} {...stylex.props(styles.rowWrap, variant === 'SR-2' ? item.children.length === 0 ? styles.rowWrapLeaf : styles.rowWrapParent : undefined, styles[variant], touch && (item.children.length === 0 ? styles.rowWrapNarrowLeaf : styles.rowWrapNarrowParent), touch && variant === 'SR-1' && styles.singleNarrow, active && styles.active)}>
      <span data-row-column="status" data-row-field="status" {...stylex.props(styles.glyph)}>{showStatus && <SidebarStatus status={item.status} statusLabel={item.statusLabel} statusSince={item.statusSince} now={clock.now} variant={rowGlyph} discRefinement={discRefinement} freshness={item.freshness} iconOnly />}</span>
      <span data-row-trailing-signals {...stylex.props(styles.trailingSignals)}>{extraSignals.includes('X-unread') && (item.unread ?? 0) > 0 && <span data-row-signal="X-unread" aria-label={`${item.unread} unread`} {...stylex.props(styles.unread)}>{item.unread}</span>}{extraSignals.includes('X-needs') && item.needsMe && <span data-row-signal="X-needs" data-row-column="attention" aria-label="Needs your attention" {...stylex.props(styles.needsYou)}>!</span>}</span>
      <span data-row-column="time" data-quick-open={quickOpen ? 'true' : undefined} {...stylex.props(styles.statusTime, quickOpen && styles.timeCovered)}>{rowLayout === 'SR2-A' ? lastTurnField : rowLayout === 'SR2-C' ? spendFields : durationField}{quickOpen && <Button data-row-action="open" aria-label={`Open ${title}`} onPress={open} {...stylex.props(styles.quickOpen)}><Icon name="message" size={12} /></Button>}</span>
      {/* The visible title is the whole name (label-in-name); status and every reported fact live in the hidden description. The custom hover card is the only hover disclosure, so no native title appears in this subtree. */}
      {inTree ? <div aria-describedby={descriptionId} aria-current={active ? 'page' : undefined} {...rowStyle}>{content}</div> : <Button aria-labelledby={showStatus && rowGlyph === 'SG-2' ? `${statusWordId} ${titleId}` : titleId} aria-describedby={descriptionId} aria-current={active ? 'page' : undefined} isDisabled={!canOpen} onPress={open} {...rowStyle}>{content}</Button>}
      {subtitle}
      <VisuallyHidden><span id={descriptionId} data-row-description>{metadata}</span></VisuallyHidden>
      <div {...stylex.props(styles.actions)}>
        <AgentRowActions row={item} reportedStatusOnly={reportedStatusOnly} {...clock} actions={callbacks} onOpen={onOpen} onMarkRead={onMarkRead} anchorRef={tooltipAnchor} menuTriggerRef={menuTrigger} onDetailsPress={() => setHoverOpen(false)} onMenuOpenChange={isOpen => { menuOpen.current = isOpen; setHoverOpen(false) }}
          menuButtonStyle={({ isHovered, isPressed, isFocusVisible }) => [styles.iconButton, styles.menuButton, (hoverOpen || isHovered || isPressed || isFocusVisible) && styles.menuVisible, variant === 'SR-1' && styles.menuSingle]}
          wrapDetailsTrigger={trigger => <TooltipTrigger delay={150} closeDelay={300} isOpen={hoverOpen} onOpenChange={setHoverOpen}>{trigger}<Tooltip triggerRef={tooltipAnchor} placement="right" {...stylex.props(styles.hovercard, styles.passiveHovercard)}><AgentHoverCard row={item} reportedStatusOnly={reportedStatusOnly} {...clock} /></Tooltip></TooltipTrigger>} />
        {item.children.length > 0 && <Button data-row-column="children" slot={inTree ? 'chevron' : undefined} isDisabled={!inTree && onToggleChildren === undefined} aria-labelledby="" aria-label={`${expanded ? 'Collapse' : 'Expand'} ${item.children.length} subagents`} aria-expanded={expanded} onPress={inTree ? undefined : () => onToggleChildren?.(item.ref)} {...stylex.props(styles.iconButton, styles.childrenToggle, variant === 'SR-2' && styles.childrenTop, variant === 'SR-1' && styles.childrenSingle, touch && styles.childrenNarrow)}><Icon name={expanded ? 'chevron-down' : 'chevron-right'} />{extraSignals.includes('X-subs') && <span data-row-signal="X-subs" data-line1-drop="subs">{item.children.length}</span>}</Button>}
      </div>
    </div>
    {!inTree && expanded && item.children.length > 0 && <div {...stylex.props(styles.children)}>{item.children.map(child => <SidebarAgentRow key={child.ref} item={child} variant={variant} layout={layout} glyph={glyph} extraSignals={extraSignals} discRefinement={discRefinement} query={query} {...clock} actions={actions} onOpen={onOpen} onMarkRead={onMarkRead} onToggleChildren={onToggleChildren} />)}</div>}
  </div>
})
const styles = stylex.create({
  wrapper: { minWidth: 0, width: '100%', containerType: 'inline-size' },
  rowWrapLeaf: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) 0px var(--sidebar-metric-track, ${g.sidebarMetric}) max-content` },
  rowWrapParent: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) ${g.icon} var(--sidebar-metric-track, ${g.sidebarMetric}) max-content` },
  // Phone grid: the desktop tracks keep their order (the col-3 slot collapses to 0px), then a 44px disclosure column when the row has subagents and a 44px menu column.
  rowWrapNarrowLeaf: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) 0px var(--sidebar-metric-track, ${g.sidebarMetric}) max-content ${g.touchTarget}` },
  rowWrapNarrowParent: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) 0px var(--sidebar-metric-track, ${g.sidebarMetric}) max-content ${g.touchTarget} ${g.touchTarget}` },
  singleNarrow: { height: g.touchTarget },
  rowNarrow: { gridColumn: '2 / 6' },
  metadataNarrow: { gridColumn: '2 / 6' },
  // Touch has no hover, so the phone menu is always visible; both controls fill their own column across the row's lines.
  childrenNarrow: { gridColumn: '-3 / -2', gridRow: '1 / -1', width: g.touchTarget, minWidth: g.touchTarget, height: g.touchTarget },
  // The row is the containing block for its visually hidden details trigger; without it the
  // absolutely positioned trigger escapes every scroll clip and extends the document.
  rowWrap: { position: 'relative', display: 'grid', gridTemplateColumns: `${g.icon} minmax(0, 1fr) ${g.controlSm} var(--sidebar-metric-track, ${g.sidebarMetric}) max-content`, alignContent: 'center', alignItems: 'center', columnGap: s.xs2, rowGap: s.xs2, minWidth: 0, width: '100%', boxSizing: 'border-box', paddingInline: s.sm, borderRadius: r.sm, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg, ':hover': { backgroundColor: surface.rowHover } },
  // Rows and menu items show keyboard focus with the neutral fgMuted ring, never the accent: an accent ring reads as a selection state.
  row: { display: 'grid', gridColumn: '2 / -1', gridRow: '1 / -1', gridTemplateColumns: 'subgrid', gridTemplateRows: 'subgrid', alignItems: 'center', minWidth: 0, width: '100%', height: '100%', padding: s.zero, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', cursor: 'pointer', ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  'SR-1': { height: g.controlLg, gridTemplateRows: '1fr' }, 'SR-2': { height: g.sidebarRow, gridTemplateRows: `${g.controlSm} ${g.controlSm}`, rowGap: s.zero }, 'SR-3': { height: g.sidebarThree, gridTemplateRows: `${t.uiLeading} ${g.controlSm} ${t.metaLeading}` },
  active: { backgroundColor: surface.rowActive },
  glyph: { gridColumn: '1', gridRow: '1', display: 'flex', alignItems: 'center', justifyContent: 'center' },
  title: { gridColumn: '1 / 3', gridRow: '1', display: 'flex', alignItems: 'center', gap: s.xs2, whiteSpace: 'nowrap', fontWeight: t.weightMedium, minWidth: 0 },
  name: { flex: '1 1 0', minWidth: g.icon, overflow: 'hidden', textOverflow: 'ellipsis' },
  statusWord: { flexShrink: 0, color: ink.fgSoft, fontSize: t.denseSize, fontWeight: t.weightRegular },
  statusTime: { display: 'inline-flex', alignItems: 'center', justifyContent: 'end', gap: s.xs, gridColumn: '4', gridRow: '1', color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', overflow: 'hidden' },
  timeCovered: { position: 'relative', visibility: 'hidden' },
  quickOpen: { position: 'absolute', insetInlineEnd: 0, insetBlock: 0, zIndex: 2, visibility: 'visible', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, boxSizing: 'border-box', padding: s.zero, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fg, cursor: 'pointer', ':hover': { backgroundColor: surface.rowActive }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  metric: { display: 'inline-flex', alignItems: 'baseline', gap: s.sm, flexShrink: 0 }, scope: { fontSize: t.microSize, color: ink.fgSoft },
  metadata: { gridColumn: '2 / -1', gridRow: '3', pointerEvents: 'none', overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', color: ink.fgSoft, fontSize: t.denseSize },
  actions: { display: 'contents' }, iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, minHeight: g.controlSm, borderWidth: 0, padding: s.zero, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowActive }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  menuButton: { gridColumn: '1', gridRow: '2', width: g.icon, opacity: 0 }, menuVisible: { opacity: 1 }, menuSingle: { gridColumn: '3', gridRow: '1' },
  childrenToggle: { gridColumn: '3', gridRow: '2', gap: s.xs2, width: g.controlSm }, childrenSingle: { gridRow: '1' }, childrenTop: { gridRow: '1', width: 'max-content', minWidth: g.icon },
  hidden: { display: 'none' },
  unread: { flexShrink: 0, color: accent.primary, fontSize: t.microSize, textAlign: 'center', fontVariantNumeric: 'tabular-nums' },
  needsYou: { flexShrink: 0, color: tone.attention, fontSize: t.denseSize, fontWeight: t.weightSemibold },
  hasChildren: { paddingInlineEnd: g.icon },
  trailingSignals: { gridColumn: '5', gridRow: '1', display: 'inline-flex', alignItems: 'center', justifyContent: 'end', gap: s.xs, width: `var(--sidebar-signal-track, ${g.status})`, marginInlineStart: s.xs, minWidth: g.status },
  hovercard: { backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, padding: s.lg, width: g.specimenAside, boxSizing: 'border-box', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, maxWidth: 'calc(100vw - 32px)', maxHeight: g.commandMaxHeight, overflowY: 'auto', boxShadow: elevation.popover, zIndex: 30 },
  // The informational tooltip never owns the divider underneath; explicit details popovers remain interactive.
  passiveHovercard: { pointerEvents: 'none' },
  children: { borderInlineStartWidth: g.hairline, borderInlineStartStyle: 'solid', borderInlineStartColor: border.border },
})
