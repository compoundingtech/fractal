// Explicit-clock presentation only: time leaves read the shared context clock, or the host's fixed instant when given.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Focusable, Popover } from 'react-aria-components'
import { Tooltip } from '../composition/Controls'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { accentVars as accent, borderVars as border, elevationVars as elevation, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { SidebarActions, SidebarAgentRow as Row } from './model'
import { AgentHoverCard, AgentRowActions, Highlight } from './SidebarAgentRow'
import { SidebarMissionLine, missionLineHasTrigger } from './SidebarMissionLine'
import { SidebarStatus } from './SidebarStatus'
import { SidebarTime } from './SidebarTime'
import { useSidebarHoverIntent } from './useSidebarHoverIntent'
import { useTouchLayout } from './useTouchLayout'
import { ThemePortal } from '../taste/ThemePortal'

export interface MilestoneAgentRowProps {
  readonly row: Row
  readonly active?: boolean
  /** Details shows only status values supplied by the session. */
  readonly reportedStatusOnly?: boolean
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
  /**
   * Makes the subagent count its own button (e.g. to focus the conversation's subagent tabs). Its press never reaches the
   * hosting tree item, so it neither selects nor opens the row. Omitted, the count is plain text. Pass a stable callback:
   * the row is memoized.
   */
  readonly onSubagentsPress?: (row: Row) => void
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
 * RM1 SR-2 two-line row, 48px. Line 1: status glyph · title + subagent count · trailing slot · (chevron). The trailing
 * slot (last turn, then an unread count or attention mark) takes width only for the content it holds, so the title runs to
 * the row edge when it is empty. Line 2: `[mission pill] · step` while the agent works on a mission (see
 * `SidebarMissionLine`), otherwise `status · host`; spend and duration live in the hover card and reported details.
 * On touch layouts (a coarse primary pointer, or a host that marks its subtree narrow) a row whose mission line holds a
 * button (a queue, or Stale/Failed mission info) grows line 2 to a 44px track, 68px in all, so that button's own box is
 * the 44 × 44 hit area. Width alone never enlarges targets: hosts provide `NarrowLayoutContext` only for touch surfaces.
 * The "!" appears only when needing you is the row's sole cue: a waiting status ("Needs you" and its glyph) or a waiting
 * mission question (its decision handle) already carries the attention.
 * Activation belongs to the hosting tree item, so the row is never a button. Desktop pointer hover shows the kit hover
 * card with no timers: the CSS entrance delay supplies hover intent. The shared ghost action trigger overlays the end of
 * line 2 on desktop, which fades out beneath it; touch rows reserve an always-visible touch-sized action column.
 * The subagent count sits right after the title, which truncates before the count does: "N subagents", "1 subagent",
 * "? subagents" while unreported, nothing at zero; display-only rows carry none. The unreported "?" explains itself in
 * the kit tooltip on hover and keyboard focus, and to assistive tech as its description.
 */
export const MilestoneAgentRow = React.memo(function MilestoneAgentRow({ row, active = false, reportedStatusOnly = false, displayOnly = false, frozen = false, now, query, depth = 0, focused = false, chevron, touchDisclosure, onOpen, onSubagentsPress, actions, onMarkRead }: MilestoneAgentRowProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const anchor = React.useRef<HTMLDivElement>(null)
  const touch = useTouchLayout()
  const { hoverOpen: hovered, setHoverOpen: setHovered } = useSidebarHoverIntent()
  const [countExplained, setCountExplained] = React.useState(false)
  const [menuFocused, setMenuFocused] = React.useState(false)
  const menuTrigger = React.useRef<HTMLButtonElement>(null)
  const mission = !displayOnly && row.missionContext?._tag === 'Mission' ? row.missionContext : undefined
  const unavailable = !displayOnly && row.missionContext?._tag === 'Unavailable'
  const touchMission = mission !== undefined && touch && missionLineHasTrigger(mission, row.missionInfo)
  const touchRow = touch && !displayOnly
  const queuedMission = (mission?.queued.length ?? 0) > 0
  // The desktop trigger overlays line 2's end; the text fades out beneath it while it shows.
  const menuOverlay = !displayOnly && !touch && !queuedMission && (hovered || menuFocused)
  const attentionCovered = row.status === 'waiting' || mission?.step._tag === 'Waiting'
  const signal = row.needsMe === true && !attentionCovered
    ? <span data-row-column="attention" aria-label="Needs your attention" {...stylex.props(styles.needs, frozen && styles.frozenInk)}>!</span>
    : (row.unread ?? 0) > 0 ? <span data-row-signal="unread" aria-label={`${row.unread} unread`} {...stylex.props(styles.unread, frozen && styles.frozenInk)}>{row.unread}</span> : null
  const hasTrailing = row.lastTurn._tag === 'Known' || signal !== null
  return <div ref={anchor} data-testid="taste-agent-row" data-milestone-row data-display-only={displayOnly || undefined} data-narrow={narrow || undefined} data-touch={touchRow || undefined} data-touch-mission={touchMission || undefined} data-status={row.status} data-frozen={frozen || undefined} data-depth={depth} data-observation-freshness={row.freshness} data-needs-me={row.needsMe}
    onMouseEnter={displayOnly ? undefined : () => setHovered(true)} onMouseLeave={displayOnly ? undefined : () => setHovered(false)}
    onFocus={displayOnly ? undefined : event => { if (menuTrigger.current?.contains(event.target) === true) setMenuFocused(true) }} onBlur={displayOnly ? undefined : event => { if (menuTrigger.current?.contains(event.target) === true) setMenuFocused(false) }}
    onContextMenu={displayOnly ? undefined : event => { event.preventDefault(); event.stopPropagation(); setHovered(false); menuTrigger.current?.click() }}
    style={depth > 0 ? { paddingInlineStart: `calc(${s.xl} * ${depth})` } : undefined}
    {...stylex.props(styles.row, !displayOnly && styles.interactive, !displayOnly && chevron !== undefined && styles.withChevron, touchRow && styles.touchRow, touchRow && chevron !== undefined && styles.touchWithChevron, !displayOnly && touchDisclosure !== undefined && styles.touchWithDisclosure, touchMission && styles.touchMissionRow, !displayOnly && active && styles.active, !displayOnly && focused && styles.focused)}>
    {/* One themed portal layer for the whole row: the count's tooltip mounts here, never inside the clipping name column. */}
    <ThemePortal>
    <span data-row-column="status" data-row-field="status" {...stylex.props(styles.glyph)}><SidebarStatus status={row.status} statusLabel={row.statusLabel} statusSince={row.statusSince} variant="SG-1" discRefinement="G3" freshness={row.freshness} frozen={frozen} iconOnly /></span>
    <span data-row-column="name" {...stylex.props(styles.name, !hasTrailing && styles.titleToEdge)}>
      <span data-row-column="title" {...stylex.props(styles.title)}><Highlight value={row.title} query={query} /></span>
      {!displayOnly && row.subagentObservation !== undefined && row.subagentObservation._tag !== 'NotReported' && row.subagentCount !== 0 && <SubagentCount count={row.subagentCount} frozen={frozen} onExplain={open => { setCountExplained(open); if (open) setHovered(false) }} onPress={onSubagentsPress === undefined ? undefined : () => { setHovered(false); onSubagentsPress(row) }} />}
    </span>
    {/* SidebarTime omits an instant after `now`: a frozen snapshot never shows activity newer than its observation as 0s. */}
    {hasTrailing && <span data-row-column="trailing" {...stylex.props(styles.trailing)}>
      {row.lastTurn._tag === 'Known' && <span data-row-column="time" {...stylex.props(styles.time, frozen && styles.frozenInk)}><span data-row-field="last-turn"><SidebarTime at={row.lastTurn.at} now={now} kind={row.lastTurn.kind === 'turn-completed' ? 'turn' : 'activity'} compact /></span></span>}
      {signal !== null && <span data-row-column="signal" {...stylex.props(styles.signal)}>{signal}</span>}
    </span>}
    {!displayOnly && chevron !== undefined && <span data-row-column="chevron" {...stylex.props(styles.chevronSlot)}>{chevron}</span>}
    {!displayOnly && touchDisclosure !== undefined && <span data-row-column="touch-disclosure" {...stylex.props(styles.touchDisclosureSlot)}>{touchDisclosure}</span>}
    <span data-row-column="line2" data-mission-info={mission === undefined ? undefined : (row.missionInfo?._tag ?? 'Live').toLowerCase()} data-menu-overlay={menuOverlay || undefined} {...stylex.props(styles.line2, mission !== undefined && styles.line2Mission, touchRow && styles.line2BeforeMenu, !displayOnly && !touch && queuedMission && styles.line2ClearsMenu, !displayOnly && touchDisclosure !== undefined && styles.touchDisclosureLine2, menuOverlay && styles.line2Faded)}>
      {mission !== undefined
        ? <SidebarMissionLine mission={mission} info={row.missionInfo} touch={touch} now={now} frozen={frozen} />
        : <>{unavailable
          ? <span data-mission-unavailable {...stylex.props(styles.unavailable)}><svg aria-hidden="true" width="10" height="10" viewBox="0 0 12 12" {...stylex.props(styles.warningIcon)}><path d="M6 1 11 10H1Z" fill="none" stroke="currentColor" /><path d="M6 4v3m0 1v1" stroke="currentColor" /></svg> Mission info unavailable</span>
          : <span data-row-field="status-label">{row.statusLabel}</span>}<span aria-hidden="true" {...stylex.props(styles.sep)}>·</span><span data-row-field="host">{row.host}</span></>}
    </span>
    {/* The hover card and the actions' overlays portal into the row's themed surface, so a light root gets light overlays. */}
    {!displayOnly && <>
      <AgentRowActions row={row} now={now} reportedStatusOnly={reportedStatusOnly} actions={actions} onOpen={onOpen} onMarkRead={onMarkRead} anchorRef={anchor} menuTriggerRef={menuTrigger} onDetailsPress={() => setHovered(false)}
        menuButtonStyle={({ isHovered, isPressed, isFocusVisible }) => [styles.menuButton, (hovered || isHovered || isPressed || isFocusVisible) && styles.menuVisible, touchDisclosure !== undefined && styles.touchMenuButton]} />
      {hovered && !countExplained && <Popover isNonModal isOpen onOpenChange={open => { if (!open) setHovered(false) }} triggerRef={anchor} placement="right top" offset={8} {...stylex.props(styles.hovercard)}>
        <AgentHoverCard row={row} now={now} reportedStatusOnly={reportedStatusOnly} />
      </Popover>}
    </>}
    </ThemePortal>
  </div>
})

const unreportedLabel = 'Unreported subagents'
const unreportedReason = 'Not reported yet'

/**
 * Running subagents; `undefined` when the harness does not report them. A known count is its own accessible text, with
 * its full "N subagents" label in the kit tooltip when it is a button. The unreported "?" is spoken as "Unreported
 * subagents", which keeps the visible word in the name, and is described by why it is unknown; that reason is its
 * tooltip on hover and keyboard focus, so as static metadata it takes a focus stop of its own. Opening a count tooltip
 * dismisses the row's hover card. A narrow name column (the 240px sidebar, deep nesting) shows the bare count with the
 * word kept for assistive tech, then drops the count so the title keeps a readable stretch; the count never paints past
 * the name column.
 */
function SubagentCount({ count, frozen, onPress, onExplain }: { readonly count: number | undefined; readonly frozen: boolean; readonly onPress: (() => void) | undefined; readonly onExplain: (open: boolean) => void }) {
  const reasonId = React.useId()
  const unknown = count === undefined
  const label = unknown ? unreportedLabel : `${count} ${count === 1 ? 'subagent' : 'subagents'}`
  const id = unknown ? 'unknown' : String(count)
  const tag = <span aria-hidden={unknown || undefined} data-count-text {...stylex.props(styles.countTag, frozen && styles.frozenInk)}>{unknown ? '?' : count}<span data-count-word {...stylex.props(styles.countWord)}>{count === 1 ? ' subagent' : ' subagents'}</span></span>
  const reason = unknown ? <span id={reasonId} hidden>{unreportedReason}</span> : null
  if (onPress !== undefined) return <Tooltip label={unknown ? unreportedReason : label} placement="top" onOpenChange={onExplain}>
    <Button data-subagent-count={id} aria-label={label} aria-describedby={unknown ? reasonId : undefined} onPress={onPress} {...stylex.props(styles.countHit, styles.countButton)}>{tag}{reason}</Button>
  </Tooltip>
  if (!unknown) return <span data-subagent-count={id} {...stylex.props(styles.countHit)}>{tag}</span>
  return <span data-subagent-count={id} {...stylex.props(styles.countHit)}>
    <Tooltip label={unreportedReason} placement="top" onOpenChange={onExplain}>
      <Focusable><span tabIndex={0} role="img" aria-label={label} aria-describedby={reasonId} data-count-trigger {...stylex.props(styles.countFocus)}>{tag}</span></Focusable>
    </Tooltip>
    {reason}
  </span>
}

// Keep ~96px for the title, 6px for the gap and ~90px for a full two-digit count. Tighter name columns, including touch
// rows at 320px, use the bare count. Below `hiddenCount` even that would leave too little title, so it leaves line 1.
const compactName = '@container agent-name (width < 192px)'
const hiddenCount = '@container agent-name (width < 48px)'

const styles = stylex.create({
  // Relative: the containing block for the visually hidden details trigger, so it never escapes the scroll clip.
  row: { position: 'relative', display: 'grid', gridTemplateColumns: `${g.icon} minmax(0, 1fr) auto`, gridTemplateRows: `${g.controlSm} ${g.controlSm}`, alignItems: 'center', columnGap: s.sm, height: g.sidebarRow, minWidth: 0, width: '100%', boxSizing: 'border-box', paddingInline: s.sm, borderRadius: r.sm, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  interactive: { ':hover': { backgroundColor: surface.rowHover } },
  // Ghost trigger overlaying the end of line 2 (no track of its own); line 2 fades out beneath it while it shows.
  menuButton: { gridColumn: '-2 / -1', gridRow: '2', justifySelf: 'end', zIndex: 1, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, padding: 0, boxSizing: 'border-box', borderWidth: 0, borderRadius: r.sm, backgroundColor: { default: surface.transparent, ':hover': surface.rowActive, ':is([data-pressed])': surface.rowActive }, color: { default: ink.fgMuted, ':hover': ink.fg }, cursor: 'pointer', opacity: 0, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted } },
  menuVisible: { opacity: 1 },
  withChevron: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) auto ${g.icon}` },
  touchRow: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) auto ${g.touchTarget}` },
  touchWithChevron: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) auto ${g.icon} ${g.touchTarget}` },
  touchWithDisclosure: { gridTemplateColumns: `${g.icon} minmax(0, 1fr) auto ${g.touchTarget} ${g.touchTarget}` },
  touchDisclosureSlot: { gridColumn: '4', gridRow: '1 / -1', display: 'flex', alignItems: 'center', justifyContent: 'center', width: g.touchTarget },
  // Touch row with a mission button: line 2 becomes the button's 44px track; desktop rows stay 48px.
  touchMissionRow: { gridTemplateRows: `${g.controlSm} ${g.touchTarget}`, height: `calc(${g.controlSm} + ${g.touchTarget})` },
  touchDisclosureLine2: { gridColumn: '2 / -3' },
  touchMenuButton: { gridRow: '1 / -1', width: g.touchTarget, height: g.touchTarget, opacity: 1 },
  active: { backgroundColor: surface.rowActive },
  // Sidebar rows ring keyboard focus in neutral fgMuted, never the accent, so focus does not read as selection.
  focused: { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` },
  glyph: { gridColumn: '1', gridRow: '1', display: 'flex', alignItems: 'center', justifyContent: 'center' },
  // Line 1's name column: the title truncates first; the count fits the column (see `SubagentCount`) and the column
  // clips, so the count can never cover the trailing slot.
  name: { gridColumn: '2', gridRow: '1', display: 'flex', alignItems: 'center', columnGap: s.sm, minWidth: 0, overflow: 'hidden', containerType: 'inline-size', containerName: 'agent-name' },
  title: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  // No last turn and no signal: the name column also takes the empty trailing track and its gap, reaching the row edge.
  titleToEdge: { gridColumn: '2 / 4' },
  // The hit area is line 1's full 24px track; the visible tag inside it is an outlined 11px label with no fill.
  countHit: { flexShrink: 0, display: { default: 'inline-flex', [hiddenCount]: 'none' }, alignItems: 'center', height: g.controlSm, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans },
  // Inset ring: the clipping name column would cut an outset one.
  countButton: { cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  // The static unreported count's own focus stop, for its tooltip: line 1's full track, ringed inset like the button.
  countFocus: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, borderRadius: r.sm, outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  // fgSoft, not fgMuted: 11px text must clear 4.5:1 on the hover and active-row washes too. Inline-block, not flex, so the
  // space before the word survives in the text and the accessible name. Relative: the compacted word stays inside the tag.
  countTag: { position: 'relative', display: 'inline-block', paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, fontSize: t.denseSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  // Compact: visually hidden, still in the accessible name.
  countWord: {
    position: { default: 'static', [compactName]: 'absolute' }, width: { default: 'auto', [compactName]: '1px' }, height: { default: 'auto', [compactName]: '1px' },
    overflow: { default: 'visible', [compactName]: 'hidden' }, clipPath: { default: 'none', [compactName]: 'inset(50%)' }, whiteSpace: 'nowrap',
  },
  // Sized by its content only: an absent time or signal reserves nothing.
  trailing: { gridColumn: '3', gridRow: '1', display: 'flex', alignItems: 'center', justifyContent: 'end', gap: s.sm },
  time: { color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  signal: { display: 'flex', justifyContent: 'center' },
  // Bold: at semibold the thin amber glyph anti-aliases below the 3:1 non-text minimum on the light sidebar.
  needs: { color: tone.attention, fontSize: t.denseSize, fontWeight: t.weightBold },
  unread: { color: accent.primary, fontSize: t.microSize, fontVariantNumeric: 'tabular-nums' },
  // Last-observed values: neutral ink, no live accent. fgSoft (not fgMuted) so it still clears 4.5:1 on the active-row wash.
  frozenInk: { color: ink.fgSoft },
  chevronSlot: { gridColumn: '4', gridRow: '1', display: 'flex', justifyContent: 'center' },
  line2: { gridColumn: '2 / -1', gridRow: '2', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgSoft, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  // The pill sits at line 2's first pixel; its slots truncate, so the line itself does not clip the mission buttons' focus rings.
  line2Mission: { display: 'flex', alignItems: 'center', overflow: 'visible' },
  // Touch rows give the action trigger its own column.
  line2BeforeMenu: { gridColumn: '2 / -2' },
  // A desktop "+N" queue row stops short of the trigger's overlay, so the queue button stays pressable.
  line2ClearsMenu: { paddingInlineEnd: `calc(${g.icon} + ${s.xs})` },
  // The trigger shows over line 2's end: the text fades out before it instead of reserving a column.
  line2Faded: { maskImage: `linear-gradient(to right, black calc(100% - ${g.icon} - ${s.lg}), transparent calc(100% - ${g.icon}))` },
  sep: { paddingInline: s.xs, color: ink.fgMuted },
  unavailable: { color: ink.fgMuted },
  warningIcon: { display: 'inline-block', verticalAlign: 'middle' },
  // The card mounts on pointer enter; the delayed entrance is the 150ms hover intent, so a pass across rows never flashes cards.
  hovercard: { backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, padding: s.lg, boxSizing: 'border-box', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, maxWidth: 'calc(100vw - 32px)', boxShadow: elevation.popover, outline: 'none', animationName: stylex.keyframes({ from: { opacity: 0 }, to: { opacity: 1 } }), animationDuration: m.fast, animationDelay: m.fast, animationTimingFunction: 'ease-out', animationFillMode: 'backwards' },
})
