// Explicit-clock presentation only: time leaves read the shared context clock, or the host's fixed instant when given.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Popover } from 'react-aria-components'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { accentVars as accent, borderVars as border, elevationVars as elevation, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { usageScopeLabel, type SidebarActions, type SidebarAgentRow as Row } from './model'
import { AgentHoverCard, AgentRowActions, Highlight } from './SidebarAgentRow'
import { SidebarStatus } from './SidebarStatus'
import { SidebarTime } from './SidebarTime'
import { useSidebarHoverIntent } from './useSidebarHoverIntent'
import { SidebarDuration } from './SidebarDuration'

const spendCount = Intl.NumberFormat('en', { style: 'currency', currency: 'USD', notation: 'compact', maximumSignificantDigits: 2 })
const tokenCount = Intl.NumberFormat('en', { notation: 'compact', maximumSignificantDigits: 2 })

export interface MilestoneAgentRowProps {
  readonly row: Row
  readonly active?: boolean
  /** Readable observation only: no menu, hover affordance, selection or activation. */
  readonly displayOnly?: boolean
  /**
   * The roster is a last-observed snapshot: glyphs go muted with shape cues, accents go neutral, and `now` should be the
   * observation instant. Independent of the row's own `status` and `freshness`, which render unchanged.
   */
  readonly frozen?: boolean
  /** Fixed instant for every time leaf; omitted, the leaves follow the shared context clock. */
  readonly now?: number
  /** Overrides `SidebarQueryContext` for the title highlight. */
  readonly query?: string
  /** Standalone tree hosts only: nesting below the root agent. `SidebarTree` owns indentation and guides, so its rows leave this 0. */
  readonly depth?: number
  /** Standalone tree hosts only: paint the keyboard focus ring (`SidebarTree` rings its own items). */
  readonly focused?: boolean
  /** Standalone tree hosts only: the item's disclosure button, in a fixed trailing column. `SidebarTree` draws its own. */
  readonly chevron?: React.ReactNode
  /** Touch tree hosts reserve a separate 44px disclosure track before the row-action button. */
  readonly touchDisclosure?: React.ReactNode
  /** Host callbacks enable the corresponding actions; reported details are always available. */
  readonly onOpen?: (row: Row) => void
  readonly actions?: SidebarActions
  readonly onMarkRead?: (ref: string) => void
}

/**
 * Opens the row menu inside `element`, for keyboard context menus (ContextMenu key, Shift+F10) that the focused tree item
 * receives instead of the row; pass it from `SidebarTree`'s `onRowContextMenu`. Pointer context menus open it directly.
 */
export const openAgentRowMenu = (element: Element): boolean => {
  const trigger = element.querySelector<HTMLButtonElement>('[data-agent-menu]')
  trigger?.click()
  return trigger !== null
}

/**
 * RM1 SR-2 two-line row, 48px. Line 1: status glyph · title · last turn · attention · (chevron), each trailing column a
 * fixed width so every time ends on one edge. Line 2: status word · host · spend and tokens · duration.
 * Activation belongs to the hosting tree item, so the row is never a button. Desktop pointer hover shows the kit hover
 * card with no timers: the CSS entrance delay supplies hover intent. The shared action trigger overlays line 2 on
 * desktop; narrow rows reserve an always-visible touch-sized action column.
 */
export const MilestoneAgentRow = React.memo(function MilestoneAgentRow({ row, active = false, displayOnly = false, frozen = false, now, query, depth = 0, focused = false, chevron, touchDisclosure, onOpen, actions, onMarkRead }: MilestoneAgentRowProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const anchor = React.useRef<HTMLDivElement>(null)
  const { hoverOpen: hovered, setHoverOpen: setHovered } = useSidebarHoverIntent()
  const menuTrigger = React.useRef<HTMLButtonElement>(null)
  const fields: React.ReactNode[] = [<span key="status" data-row-field="status-label">{row.statusLabel}</span>, <span key="host" data-row-field="host">{row.host}</span>]
  if (row.usage._tag === 'Known') {
    const { usd, tokens, scope } = row.usage
    fields.push(<span key="spend" data-row-field="spend" aria-label={`${usageScopeLabel(scope)}: ${usd === undefined ? '' : `$${usd}, `}${tokens} tokens`}>{usd === undefined ? `${tokenCount.format(tokens)} tokens` : `${spendCount.format(usd)} · ${tokenCount.format(tokens)}`}</span>)
  }
  if (row.duration._tag === 'Known') fields.push(<span key="duration" data-row-field="duration"><SidebarDuration duration={row.duration} /></span>)
  const signal = row.needsMe
    ? <span data-row-column="attention" aria-label="Needs your attention" {...stylex.props(styles.needs, frozen && styles.frozenInk)}>!</span>
    : (row.unread ?? 0) > 0 ? <span data-row-signal="unread" aria-label={`${row.unread} unread`} {...stylex.props(styles.unread, frozen && styles.frozenInk)}>{row.unread}</span> : null
  return <div ref={anchor} data-testid="taste-agent-row" data-milestone-row data-display-only={displayOnly || undefined} data-narrow={narrow || undefined} data-status={row.status} data-frozen={frozen || undefined} data-depth={depth} data-observation-freshness={row.freshness} data-needs-me={row.needsMe}
    onMouseEnter={displayOnly ? undefined : () => setHovered(true)} onMouseLeave={displayOnly ? undefined : () => setHovered(false)}
    onContextMenu={displayOnly ? undefined : event => { event.preventDefault(); event.stopPropagation(); setHovered(false); menuTrigger.current?.click() }}
    style={depth > 0 ? { paddingInlineStart: `calc(${s.xl} * ${depth})` } : undefined}
    {...stylex.props(styles.row, !displayOnly && styles.interactive, !displayOnly && chevron !== undefined && styles.withChevron, narrow && !displayOnly && styles.narrowRow, narrow && !displayOnly && chevron !== undefined && styles.narrowWithChevron, !displayOnly && touchDisclosure !== undefined && styles.touchWithDisclosure, !displayOnly && active && styles.active, !displayOnly && focused && styles.focused)}>
    <span data-row-column="status" data-row-field="status" {...stylex.props(styles.glyph)}><SidebarStatus status={row.status} statusLabel={row.statusLabel} statusSince={row.statusSince} variant="SG-1" discRefinement="G3" freshness={row.freshness} frozen={frozen} iconOnly /></span>
    <span data-row-column="title" {...stylex.props(styles.title)}><Highlight value={row.title} query={query} /></span>
    {/* SidebarTime omits an instant after `now`: a frozen snapshot never shows activity newer than its observation as 0s. */}
    <span data-row-column="time" {...stylex.props(styles.time, frozen && styles.frozenInk)}>{row.lastTurn._tag === 'Known' && <span data-row-field="last-turn"><SidebarTime at={row.lastTurn.at} now={now} kind={row.lastTurn.kind === 'turn-completed' ? 'turn' : 'activity'} compact /></span>}</span>
    <span data-row-column="signal" {...stylex.props(styles.signal)}>{signal}</span>
    {!displayOnly && chevron !== undefined && <span data-row-column="chevron" {...stylex.props(styles.chevronSlot)}>{chevron}</span>}
    {!displayOnly && touchDisclosure !== undefined && <span data-row-column="touch-disclosure" {...stylex.props(styles.touchDisclosureSlot)}>{touchDisclosure}</span>}
    <span data-row-column="line2" {...stylex.props(styles.line2, narrow && !displayOnly && styles.narrowLine2, !displayOnly && touchDisclosure !== undefined && styles.touchDisclosureLine2)}>{fields.flatMap((field, index) => index === 0 ? [field] : [<span key={`sep-${index}`} aria-hidden="true" {...stylex.props(styles.sep)}>·</span>, field])}</span>
    {!displayOnly && <AgentRowActions row={row} now={now} actions={actions} onOpen={onOpen} onMarkRead={onMarkRead} anchorRef={anchor} menuTriggerRef={menuTrigger} onDetailsPress={() => setHovered(false)}
      menuButtonStyle={({ isHovered, isPressed, isFocusVisible }) => [styles.menuButton, (hovered || isHovered || isPressed || isFocusVisible) && styles.menuVisible, touchDisclosure !== undefined && styles.touchMenuButton]} />}
    {!displayOnly && hovered && <Popover isNonModal isOpen onOpenChange={open => { if (!open) setHovered(false) }} triggerRef={anchor} placement="right top" offset={8} {...stylex.props(styles.hovercard)}>
      <AgentHoverCard row={row} now={now} />
    </Popover>}
  </div>
})

const styles = stylex.create({
  // Relative: the containing block for the visually hidden details trigger, so it never escapes the scroll clip.
  row: { position: 'relative', display: 'grid', gridTemplateColumns: `${g.icon} minmax(0, 1fr) ${g.sidebarMetric} ${g.status}`, gridTemplateRows: `${g.controlSm} ${g.controlSm}`, alignItems: 'center', columnGap: s.sm, height: g.sidebarRow, minWidth: 0, width: '100%', boxSizing: 'border-box', paddingInline: s.sm, borderRadius: r.sm, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  interactive: { ':hover': { backgroundColor: surface.rowHover } },
  // Overlays the end of line 2 (same grid area, no track of its own); filled so truncated text never shows through.
  menuButton: { gridColumn: '-2 / -1', gridRow: '2', justifySelf: 'end', zIndex: 1, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, padding: 0, boxSizing: 'border-box', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fgSoft, cursor: 'pointer', opacity: 0, ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted } },
  menuVisible: { opacity: 1 },
  withChevron: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) ${g.sidebarMetric} ${g.status} ${g.icon}` },
  narrowRow: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) ${g.sidebarMetric} ${g.status} ${g.touchTarget}` },
  narrowWithChevron: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) ${g.sidebarMetric} ${g.status} ${g.icon} ${g.touchTarget}` },
  narrowLine2: { gridColumn: '2 / -2' },
  touchWithDisclosure: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) ${g.sidebarMetric} ${g.status} ${g.touchTarget} ${g.touchTarget}` },
  touchDisclosureSlot: { gridColumn: '5', gridRow: '1 / -1', display: 'flex', alignItems: 'center', justifyContent: 'center', width: g.touchTarget },
  touchDisclosureLine2: { gridColumn: '2 / -3' },
  touchMenuButton: { gridRow: '1 / -1', width: g.touchTarget, height: g.touchTarget, opacity: 1 },
  active: { backgroundColor: surface.rowActive },
  // Sidebar rows ring keyboard focus in neutral fgMuted, never the accent, so focus does not read as selection.
  focused: { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` },
  glyph: { gridColumn: '1', gridRow: '1', display: 'flex', alignItems: 'center', justifyContent: 'center' },
  title: { gridColumn: '2', gridRow: '1', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  time: { gridColumn: '3', gridRow: '1', justifySelf: 'end', color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  signal: { gridColumn: '4', gridRow: '1', display: 'flex', justifyContent: 'center' },
  needs: { color: tone.attention, fontSize: t.denseSize, fontWeight: t.weightSemibold },
  unread: { color: accent.primary, fontSize: t.microSize, fontVariantNumeric: 'tabular-nums' },
  // Last-observed values: neutral ink, no live accent. fgSoft (not fgMuted) so it still clears 4.5:1 on the active-row wash.
  frozenInk: { color: ink.fgSoft },
  chevronSlot: { gridColumn: '5', gridRow: '1', display: 'flex', justifyContent: 'center' },
  line2: { gridColumn: '2 / -1', gridRow: '2', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  sep: { paddingInline: s.xs, color: ink.fgMuted },
  // The card mounts on pointer enter; the delayed entrance is the 150ms hover intent, so a pass across rows never flashes cards.
  hovercard: { backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, padding: s.lg, boxSizing: 'border-box', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, maxWidth: 'calc(100vw - 32px)', boxShadow: elevation.popover, outline: 'none', animationName: stylex.keyframes({ from: { opacity: 0 }, to: { opacity: 1 } }), animationDuration: m.fast, animationDelay: m.fast, animationTimingFunction: 'ease-out', animationFillMode: 'backwards' },
})
