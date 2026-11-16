import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button as AriaButton, Dialog, DialogTrigger, Pressable } from 'react-aria-components'
import { interactionVars as interaction, colorVars as c, typeVars as t, radiusVars as r, geometryVars as g } from '../composition-tokens.stylex'
import { SidebarStatus as StatusGlyph } from '../sidebar/SidebarStatus'
import { Icon } from './Icons'
import { paneHeaderStyles } from './Pane'
import { ChromeControl } from './ChromeControl'
import { Popover } from './Controls'
import { NarrowLayoutContext } from './NarrowLayout'
import type { AgentStatus } from '../sidebar/model'

/**
 * Keyboard/pointer resizable separator. Arrow ±8, Shift+Arrow ±32,
 * Home/End min/max, Enter collapses, double-click resets; Escape rolls back a held drag.
 */
export function ResizableSplit({ id, value, min, max, onChange, onToggle, onReset, collapsed, label, onCommit, reverse = false, orientation = 'vertical' }: {
  onCommit?: (value: number) => void
  reverse?: boolean
  id: string
  value: number
  min: number
  max: number
  onChange: (value: number) => void
  onToggle: () => void
  onReset: () => void
  collapsed: boolean
  label: string
  orientation?: 'vertical' | 'horizontal'
}) {
  const canResize = Number.isFinite(min) && Number.isFinite(max) && max > min
  const activeDrag = React.useRef<(() => void) | undefined>(undefined)
  // Commit before any separator removal, including a frozen range while the split stays mounted.
  const separatorRef = React.useCallback((node: HTMLDivElement | null) => {
    if (node === null) activeDrag.current?.()
  }, [])
  const save = React.useCallback((next: number) => {
    const clamped = Math.min(max, Math.max(min, Math.round(next)))
    onChange(clamped)
    if (onCommit) onCommit(clamped)
    else try { window.localStorage.setItem(`composition.${id}`, String(clamped)) } catch { /* persistence is best-effort */ }
  }, [id, min, max, onChange, onCommit])
  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!canResize || collapsed || event.button !== 0) return
    event.preventDefault()
    const node = event.currentTarget
    const start = orientation === 'vertical' ? event.clientX : event.clientY
    const before = value
    let latest = before
    let frame: number | undefined
    let finished = false
    node.setPointerCapture(event.pointerId)
    const flush = () => { frame = undefined; onChange(latest) }
    const move = (moveEvent: PointerEvent) => {
      const current = orientation === 'vertical' ? moveEvent.clientX : moveEvent.clientY
      latest = Math.min(max, Math.max(min, Math.round(before + (reverse ? -1 : 1) * (current - start))))
      if (frame === undefined) frame = window.requestAnimationFrame(flush)
    }
    const finish = (cancelled: boolean) => {
      if (finished) return
      finished = true
      activeDrag.current = undefined
      if (frame !== undefined) window.cancelAnimationFrame(frame)
      node.removeEventListener('pointermove', move)
      node.removeEventListener('pointerup', up)
      node.removeEventListener('pointercancel', cancel)
      node.removeEventListener('lostpointercapture', cancel)
      node.ownerDocument.removeEventListener('keydown', escape, true)
      if (cancelled) onChange(before)
      else save(latest)
      if (node.hasPointerCapture(event.pointerId)) node.releasePointerCapture(event.pointerId)
    }
    const up = () => finish(false)
    const cancel = () => finish(true)
    const escape = (keyEvent: KeyboardEvent) => {
      if (keyEvent.key !== 'Escape') return
      keyEvent.preventDefault()
      keyEvent.stopPropagation()
      finish(true)
    }
    activeDrag.current = () => finish(false)
    node.addEventListener('pointermove', move)
    node.addEventListener('pointerup', up)
    node.addEventListener('pointercancel', cancel)
    node.addEventListener('lostpointercapture', cancel)
    // Pointer capture does not move keyboard focus; cancel even when another control is focused.
    node.ownerDocument.addEventListener('keydown', escape, true)
  }
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 32 : 8
    const delta = reverse ? -step : step
    const grow = orientation === 'vertical' ? ['ArrowRight', 'ArrowDown'] : ['ArrowDown']
    const shrink = orientation === 'vertical' ? ['ArrowLeft', 'ArrowUp'] : ['ArrowUp']
    if (grow.includes(event.key)) { event.preventDefault(); save(value + delta) }
    else if (shrink.includes(event.key)) { event.preventDefault(); save(value - delta) }
    else if (event.key === 'Home') { event.preventDefault(); save(min) }
    else if (event.key === 'End') { event.preventDefault(); save(max) }
    else if (event.key === 'Enter') { event.preventDefault(); onToggle() }
  }
  return <div data-resizer-track={id} aria-hidden={canResize ? undefined : true} {...stylex.props(orientation === 'vertical' ? styles.splitVertical : styles.splitHorizontal)}>
    {canResize && <div
    ref={separatorRef}
    role="separator"
    aria-label={label}
    aria-orientation={orientation}
    aria-valuenow={Math.round(value)}
    aria-valuemin={min}
    aria-valuemax={max}
    tabIndex={0}
    data-resizer={id}
    onPointerDown={onPointerDown}
    onKeyDown={onKeyDown}
    onDoubleClick={onReset}
    {...stylex.props(orientation === 'vertical' ? styles.resizeVertical : styles.resizeHorizontal)}
  />}
  </div>
}

// Controlled header: the host supplies every displayed fact.
export function ThreadHeader({
  folder,
  breadcrumbLabel = 'Breadcrumb',
  title,
  status,
  statusLabel,
  elapsed,
  now,
  nativeActions,
  actionPortalRef,
  terminalAvailable = false,
  terminalDisabledReason,
  onOpen,
  onCommit,
  statusSince,
  freshness,
  panelOpen,
  terminalOpen,
  onTogglePanel,
  onToggleTerminal,
  sidebarCollapsed = false,
  onToggleSidebar,
  sidebarToggleRef,
  panelToggleRef,
  sidebarSheet,
  narrow = false,
}: {
  /** Reported folder for the breadcrumb; omitted entirely when the host knows none. */
  folder?: string
  breadcrumbLabel?: string
  title: string
  /** Observed agent status, never inferred from conversation lifecycle. */
  status?: AgentStatus
  /** Exact reported native label; the glyph shape does not establish attention or freshness. */
  statusLabel?: string
  statusSince?: number
  freshness?: 'live' | 'stale' | 'unobserved'
  /** Elapsed label; a ReactNode so an isolated ticker can be passed in. */
  elapsed?: React.ReactNode
  now?: number
  nativeActions?: React.ReactNode
  actionPortalRef?: React.Ref<HTMLDivElement>
  terminalAvailable?: boolean
  /** Why the terminal is unavailable. With `terminalAvailable` false, a set reason shows the toggle disabled and described by it; without one the toggle stays hidden. A narrow header has no terminal control: the phone host places `PhoneTerminalToggle` beside its workspace tabs. */
  terminalDisabledReason?: string
  onOpen?: () => void
  onCommit?: () => void
  panelOpen: boolean
  /** The main pane shows the terminal in place of the conversation; the toggle then offers the way back. */
  terminalOpen: boolean
  onTogglePanel: () => void
  onToggleTerminal: () => void
  /** The wide header's Toggle right panel button: the opener a host returns focus to when an explicit close unmounts the focused panel control. */
  panelToggleRef?: React.Ref<HTMLButtonElement>
  /** When the sidebar is collapsed its separator is gone; the header carries the expand toggle. */
  sidebarCollapsed?: boolean
  onToggleSidebar?: () => void
  sidebarToggleRef?: React.Ref<HTMLButtonElement>
  /** The expand toggle opens the sidebar as a sheet (narrow hosts): its open state and dialog id, for `aria-expanded`/`aria-controls`. */
  sidebarSheet?: { readonly open: boolean; readonly id: string }
  /** The host is below its drawer breakpoint: the header keeps the breadcrumb and status, the host crumb may shrink, the terminal control leaves the header (the host renders `PhoneTerminalToggle` in its tab row), and one Agent details button opens `panelOpen`'s sheet. Every header control becomes a 44px touch target and the header clears the left/right safe-area insets; the host owns the top inset. */
  narrow?: boolean
}) {
  // The `narrow` prop is authoritative for the header's own controls: they size as 44px touch targets even when the host did not provide the context.
  const inheritedNarrow = React.useContext(NarrowLayoutContext)
  return (
    <NarrowLayoutContext.Provider value={narrow === true || inheritedNarrow}>
    <header data-testid="thread-header" {...stylex.props(styles.header, !narrow && paneHeaderStyles.header, !narrow && styles.laneContainer, narrow && styles.headerNarrow)}>
      {sidebarCollapsed && onToggleSidebar !== undefined ? (
        <ChromeControl ref={sidebarToggleRef} icon="chevron-right" label="Expand sidebar" tooltip="Expand sidebar (⌘/Ctrl+B)" edge="lead" onPress={onToggleSidebar} {...(sidebarSheet === undefined ? {} : { popup: 'dialog' as const, expanded: sidebarSheet.open, ...(sidebarSheet.open ? { controls: sidebarSheet.id } : {}) })} />
      ) : null}
      {/* The status mark sits beside the title, outside the breadcrumb landmark so the crumb reads as names only. */}
      <div {...stylex.props(styles.titleGroup)}>
      <nav aria-label={breadcrumbLabel} {...stylex.props(styles.breadcrumb, styles.breadcrumbFit)}>
        {folder !== undefined ? <>
          <span {...stylex.props(styles.crumbFolder, narrow && styles.crumbFolderShrink, !narrow && styles.desktopFolder)}>{folder}</span>
          <span aria-hidden="true" {...stylex.props(styles.crumbSlash, !narrow && styles.desktopFolder)}>/</span>
        </> : null}
        <span role={narrow ? undefined : 'heading'} aria-level={narrow ? undefined : 2} {...stylex.props(styles.crumbTitle)}>{title}</span>
      </nav>
      {status !== undefined ? <StatusGlyph status={status} statusLabel={statusLabel} statusSince={statusSince} freshness={freshness} now={now} /> : null}
      </div>
      <div {...stylex.props(styles.actions)}>
        {elapsed !== undefined ? <span role="timer" aria-label="Current conversation turn elapsed">{elapsed}</span> : null}
        <div data-testid="native-action-slot" ref={actionPortalRef} {...stylex.props(styles.actionSlot)}>{nativeActions}</div>
        {onOpen !== undefined && <button type="button" onClick={onOpen} {...stylex.props(styles.outlineXs, narrow && styles.outlineTouch)}>Open <Icon name="chevron-down" size={12} /></button>}
        {onCommit !== undefined && <button type="button" onClick={onCommit} {...stylex.props(styles.outlineXs, narrow && styles.outlineTouch)}>Commit <Icon name="chevron-down" size={12} /></button>}
        {!narrow && terminalAvailable && <ChromeControl icon={terminalOpen ? 'message' : 'terminal'} label={terminalOpen ? 'Show conversation' : 'Show terminal'} active={terminalOpen} onPress={onToggleTerminal} />}
        {!narrow && !terminalAvailable && terminalDisabledReason !== undefined && <ChromeControl icon="terminal" label="Show terminal" disabledReason={terminalDisabledReason} />}
        {narrow
          ? <ChromeControl icon="panel" label="Agent details" edge="trail" popup="dialog" expanded={panelOpen} active={panelOpen} onPress={onTogglePanel} />
          : <ChromeControl ref={panelToggleRef} icon="panel" label="Toggle right panel" pressed={panelOpen} onPress={onTogglePanel} />}
      </div>
    </header>
    </NarrowLayoutContext.Provider>
  )
}

/**
 * The phone terminal toggle, placed by the host at the trailing end of its workspace tab row: a 44px button whose
 * visible **Terminal** label names it, so its meaning never depends on a tooltip. Unavailable, it stays focusable and
 * `aria-disabled`, keeps `aria-pressed="false"`, is described by the reason, and opens that reason in a popover on
 * press instead of toggling.
 */
export function PhoneTerminalToggle({ pressed, onPress, disabledReason }: { readonly pressed: boolean; readonly onPress: () => void; readonly disabledReason?: string }) {
  const reasonId = React.useId()
  // The name says what a press shows next; the fill says the terminal is showing now. `aria-pressed` would read "Show conversation, pressed".
  const content = <><Icon name={pressed ? 'message' : 'terminal'} /><span>{pressed ? 'Conversation' : 'Terminal'}</span></>
  if (disabledReason === undefined) {
    return <AriaButton aria-label={pressed ? 'Show conversation' : 'Show terminal'} onPress={onPress} data-narrow-terminal {...stylex.props(styles.terminalTouch, pressed && styles.terminalPressed)}>{content}</AriaButton>
  }
  return <DialogTrigger>
    <Pressable>
      <button type="button" aria-label="Show terminal" aria-disabled="true" aria-describedby={reasonId} data-narrow-terminal {...stylex.props(styles.terminalTouch, styles.terminalDisabled)}>{content}</button>
    </Pressable>
    <span id={reasonId} hidden>{disabledReason}</span>
    <Popover placement="bottom end">
      <Dialog aria-label="Terminal unavailable" {...stylex.props(styles.terminalReason)}>{disabledReason}</Dialog>
    </Popover>
  </DialogTrigger>
}



const styles = stylex.create({
  // Full 1px layout rules; fixed chrome owns the last raster row and the sidebar owns the crossing hit area.
  splitVertical: { width: 1, alignSelf: 'stretch', backgroundColor: c.borderStrong, position: 'relative', overflow: 'visible', flexShrink: 0 },
  splitHorizontal: { height: 1, width: '100%', backgroundColor: c.borderStrong, position: 'relative', overflow: 'visible', flexShrink: 0 },
  resizeVertical: { position: 'absolute', insetBlockStart: 0, insetBlockEnd: 1, insetInlineStart: 0, width: 1, cursor: 'col-resize', zIndex: 2, touchAction: 'none', '::after': { content: '""', position: 'absolute', insetInline: -3, insetBlock: 0 }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: -1 } },
  resizeHorizontal: { position: 'absolute', insetInlineStart: 0, insetInlineEnd: 4, insetBlockStart: 0, height: 1, cursor: 'row-resize', zIndex: 2, touchAction: 'none', '::after': { content: '""', insetInline: 0, insetBlock: -3, position: 'absolute' }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: -1 } },
  header: { height: g.band, display: 'flex', alignItems: 'center', gap: g.gutter, paddingInline: g.gutter, flexShrink: 0, minWidth: 0 },
  laneContainer: { containerType: 'inline-size', containerName: 'thread-header' },
  desktopFolder: { display: { default: 'inline', '@container thread-header (max-width: 560px)': 'none' } },
  // A landscape phone's notch sits beside the header; the gutter stays the minimum.
  headerNarrow: { paddingInlineStart: `max(${g.gutter}, env(safe-area-inset-left, 0px))`, paddingInlineEnd: `max(${g.gutter}, env(safe-area-inset-right, 0px))` },
  titleGroup: { display: 'flex', flex: '1 1 0', alignItems: 'center', gap: 6, minWidth: 0 },
  breadcrumbFit: { flex: '0 1 auto' },
  breadcrumb: { display: 'flex', flex: '1 1 0', alignItems: 'baseline', gap: 6, minWidth: 0, fontSize: t.uiSize, lineHeight: t.uiLeading },
  crumbFolderShrink: { flexShrink: 1, minWidth: 0 },
  crumbFolder: { color: c.fgMuted, maxWidth: 160, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', flexShrink: 0 },
  crumbSlash: { color: c.fgMuted },
  crumbTitle: { color: c.fg, fontWeight: t.weightMedium, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  actionSlot: { display: 'flex', alignItems: 'center', gap: 4, flexShrink: 1, minWidth: 0 },
  actions: { marginInlineStart: 'auto', display: 'flex', alignItems: 'center', gap: 4, flexShrink: 1, minWidth: 0, maxWidth: '100%' },
  outlineXs: { height: g.controlSm, display: 'inline-flex', alignItems: 'center', whiteSpace: 'nowrap', flexShrink: 0, paddingInline: 6, borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, backgroundColor: 'rgba(255, 255, 255, 0.025)', color: c.fg, borderRadius: r.control, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: 1 } },
  outlineTouch: { height: g.touchTarget },
  // Same quiet chrome as ChromeControl (borderless, filled on hover and while pressed), widened by its visible label.
  terminalTouch: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: 4, flexShrink: 0, boxSizing: 'border-box', minWidth: g.touchTarget, height: g.touchTarget, paddingInline: 8, borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: c.fgMuted, fontFamily: 'inherit', fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, whiteSpace: 'nowrap', cursor: 'pointer', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  terminalPressed: { backgroundColor: c.rowActive, color: c.fg, ':hover': { backgroundColor: c.rowActive, color: c.fg } },
  terminalDisabled: { opacity: 0.64, cursor: 'default', ':hover': { backgroundColor: 'transparent', color: c.fgMuted } },
  terminalReason: { maxWidth: 280, padding: 8, fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fg, outline: 'none' },
})
