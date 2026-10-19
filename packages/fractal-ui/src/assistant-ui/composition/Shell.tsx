import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { interactionVars as interaction, colorVars as c, typeVars as t, radiusVars as r, geometryVars as g } from '../composition-tokens.stylex'
import { SidebarStatus as StatusGlyph } from '../sidebar/SidebarStatus'
import { Icon } from './Icons'
import { paneHeaderStyles } from './Pane'
import { ChromeControl } from './ChromeControl'
import type { AgentStatus } from '../sidebar/model'

/**
 * Keyboard/pointer resizable separator. Arrow ±8, Shift+Arrow ±32,
 * Home/End min/max, Enter collapses, double-click resets; size persists.
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
  const save = React.useCallback((next: number) => {
    const clamped = Math.min(max, Math.max(min, Math.round(next)))
    onChange(clamped)
    if (onCommit) onCommit(clamped)
    else try { window.localStorage.setItem(`composition.${id}`, String(clamped)) } catch { /* persistence is best-effort */ }
  }, [id, min, max, onChange, onCommit])
  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (collapsed || event.button !== 0) return
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
      if (frame !== undefined) window.cancelAnimationFrame(frame)
      node.removeEventListener('pointermove', move)
      node.removeEventListener('pointerup', up)
      node.removeEventListener('pointercancel', cancel)
      node.removeEventListener('lostpointercapture', cancel)
      if (cancelled) onChange(before)
      else save(latest)
      if (node.hasPointerCapture(event.pointerId)) node.releasePointerCapture(event.pointerId)
    }
    const up = () => finish(false)
    const cancel = () => finish(true)
    node.addEventListener('pointermove', move)
    node.addEventListener('pointerup', up)
    node.addEventListener('pointercancel', cancel)
    node.addEventListener('lostpointercapture', cancel)
  }
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 32 : 8
    const grow = orientation === 'vertical' ? ['ArrowRight', 'ArrowDown'] : ['ArrowDown']
    const shrink = orientation === 'vertical' ? ['ArrowLeft', 'ArrowUp'] : ['ArrowUp']
    if (grow.includes(event.key)) { event.preventDefault(); save(value + step) }
    else if (shrink.includes(event.key)) { event.preventDefault(); save(value - step) }
    else if (event.key === 'Home') { event.preventDefault(); save(min) }
    else if (event.key === 'End') { event.preventDefault(); save(max) }
    else if (event.key === 'Enter') { event.preventDefault(); onToggle() }
  }
  return <div
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
    {...stylex.props(orientation === 'vertical' ? styles.splitVertical : styles.splitHorizontal)}
  />
}

/** Restores a persisted size once, client-side only. */
export const usePersistedSize = (id: string, fallback: number) => {
  const [size, setSize] = React.useState(() => {
    try { const value = Number(window.localStorage.getItem(`composition.${id}`)); return Number.isFinite(value) && value > 0 ? value : fallback } catch { return fallback }
  })
  return [size, setSize] as const
}

// Controlled header: the host supplies every displayed fact.
export function ThreadHeader({
  folder,
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
  drawerOpen,
  onTogglePanel,
  onToggleDrawer,
  sidebarCollapsed = false,
  onToggleSidebar,
  sidebarSheet,
  narrow = false,
}: {
  /** Reported folder for the breadcrumb; omitted entirely when the host knows none. */
  folder?: string
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
  /** Why the terminal is unavailable. With `terminalAvailable` false, a set reason shows the toggle disabled and described by it; without one the toggle stays hidden. */
  terminalDisabledReason?: string
  onOpen?: () => void
  onCommit?: () => void
  panelOpen: boolean
  drawerOpen: boolean
  onTogglePanel: () => void
  onToggleDrawer: () => void
  /** When the sidebar is collapsed its separator is gone; the header carries the expand toggle. */
  sidebarCollapsed?: boolean
  onToggleSidebar?: () => void
  /** The expand toggle opens the sidebar as a sheet (narrow hosts): its open state and dialog id, for `aria-expanded`/`aria-controls`. */
  sidebarSheet?: { readonly open: boolean; readonly id: string }
  /** The host is below its drawer breakpoint: the header keeps its 52px band, the breadcrumb and status, the host crumb may shrink, and one Agent details button opens `panelOpen`'s sheet. Wide hosts get the shared 32px pane header. */
  narrow?: boolean
}) {
  return (
    <header data-testid="thread-header" {...stylex.props(styles.header, !narrow && paneHeaderStyles.header, !narrow && styles.laneContainer)}>
      {sidebarCollapsed && onToggleSidebar !== undefined ? (
        <ChromeControl icon="chevron-right" label="Expand sidebar" onPress={onToggleSidebar} {...(sidebarSheet === undefined ? {} : { popup: 'dialog' as const, expanded: sidebarSheet.open, ...(sidebarSheet.open ? { controls: sidebarSheet.id } : {}) })} />
      ) : null}
      <nav aria-label="Breadcrumb" {...stylex.props(styles.breadcrumb)}>
        {folder !== undefined ? <>
          <span {...stylex.props(styles.crumbFolder, narrow && styles.crumbFolderShrink, !narrow && styles.desktopFolder)}>{folder}</span>
          <span aria-hidden="true" {...stylex.props(styles.crumbSlash, !narrow && styles.desktopFolder)}>/</span>
        </> : null}
        <span role={narrow ? undefined : 'heading'} aria-level={narrow ? undefined : 2} {...stylex.props(styles.crumbTitle)}>{title}</span>
      </nav>
      <div {...stylex.props(styles.actions)}>
        {status !== undefined ? <StatusGlyph status={status} statusLabel={statusLabel} statusSince={statusSince} freshness={freshness} now={now} /> : null}
        {elapsed !== undefined ? <span role="timer" aria-label="Current conversation turn elapsed">{elapsed}</span> : null}
        <div data-testid="native-action-slot" ref={actionPortalRef} {...stylex.props(styles.actionSlot)}>{nativeActions}</div>
        {onOpen !== undefined && <button type="button" onClick={onOpen} {...stylex.props(styles.outlineXs)}>Open <Icon name="chevron-down" size={12} /></button>}
        {onCommit !== undefined && <button type="button" onClick={onCommit} {...stylex.props(styles.outlineXs)}>Commit <Icon name="chevron-down" size={12} /></button>}
        {!narrow && terminalAvailable && <ChromeControl icon="drawer" label="Toggle terminal drawer" pressed={drawerOpen} onPress={onToggleDrawer} />}
        {!narrow && !terminalAvailable && terminalDisabledReason !== undefined && <ChromeControl icon="drawer" label="Toggle terminal drawer" disabledReason={terminalDisabledReason} />}
        {narrow
          ? <ChromeControl icon="panel" label="Agent details" popup="dialog" expanded={panelOpen} active={panelOpen} onPress={onTogglePanel} />
          : <ChromeControl icon="panel" label="Toggle right panel" pressed={panelOpen} onPress={onTogglePanel} />}
      </div>
    </header>
  )
}


export function TerminalDrawer({ lines, open, height, onHeight }: { lines: readonly string[]; open: boolean; height: number; onHeight: (value: number) => void }) {
  if (!open) return null
  return <section aria-label="Terminal drawer" data-testid="terminal-drawer" style={{ height }} {...stylex.props(styles.drawer)}>
    <div {...stylex.props(styles.drawerHead)}><span {...stylex.props(styles.drawerLabel)}>terminal</span><span {...stylex.props(styles.drawerHint)}>Drag the header edge, or focus it and use ↑/↓</span></div>
    <div {...stylex.props(styles.drawerBody)}>
      {lines.map((line, index) => <div key={index} {...stylex.props(styles.drawerLine)}>{line}</div>)}
    </div>
    <label {...stylex.props(styles.drawerRangeLabel)}>height<input type="range" aria-label="Drawer height" min={120} max={480} value={height} onChange={event => onHeight(Number(event.target.value))} {...stylex.props(styles.drawerRange)} /></label>
  </section>
}

const styles = stylex.create({
  splitVertical: { width: 1, alignSelf: 'stretch', backgroundColor: c.borderStrong, cursor: 'col-resize', position: 'relative', flexShrink: 0, touchAction: 'none', '::after': { content: '""', position: 'absolute', insetInline: -3, insetBlock: 0 }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: -1 } },
  splitHorizontal: { height: 1, width: '100%', backgroundColor: c.borderStrong, cursor: 'row-resize', position: 'relative', flexShrink: 0, touchAction: 'none', '::after': { content: '""', insetInline: 0, insetBlock: -3, position: 'absolute' }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: -1 } },
  header: { height: g.band, display: 'flex', alignItems: 'center', gap: g.gutter, paddingInline: g.gutter, flexShrink: 0, minWidth: 0 },
  laneContainer: { containerType: 'inline-size', containerName: 'thread-header' },
  desktopFolder: { display: { default: 'inline', '@container thread-header (max-width: 560px)': 'none' } },
  breadcrumb: { display: 'flex', flex: '1 1 0', alignItems: 'baseline', gap: 6, minWidth: 0, fontSize: t.uiSize, lineHeight: t.uiLeading },
  crumbFolderShrink: { flexShrink: 1, minWidth: 0 },
  crumbFolder: { color: c.fgMuted, maxWidth: 160, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', flexShrink: 0 },
  crumbSlash: { color: c.fgMuted },
  crumbTitle: { color: c.fg, fontWeight: t.weightMedium, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  actionSlot: { display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 },
  actions: { marginInlineStart: 'auto', display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 },
  outlineXs: { height: g.controlSm, display: 'inline-flex', alignItems: 'center', whiteSpace: 'nowrap', flexShrink: 0, paddingInline: 6, borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, backgroundColor: 'rgba(255, 255, 255, 0.025)', color: c.fg, borderRadius: r.control, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: 1 } },
  drawer: { display: 'flex', flexDirection: 'column', borderTopWidth: 1, borderTopStyle: 'solid', borderTopColor: c.borderStrong, backgroundColor: c.canvas, flexShrink: 0, minHeight: 0 },
  drawerHead: { height: 32, display: 'flex', alignItems: 'center', gap: 12, paddingInline: 12 },
  drawerLabel: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fgMuted, fontWeight: t.weightMedium },
  drawerHint: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fgMuted, marginInlineStart: 'auto' },
  drawerBody: { paddingInline: 12, paddingBottom: 4, overflowY: 'auto', fontFamily: t.fontMono, fontSize: t.codeSize, lineHeight: '20px', whiteSpace: 'pre-wrap' },
  drawerLine: { color: c.fgMuted },
  drawerRangeLabel: { display: 'flex', alignItems: 'center', gap: 8, marginInline: 12, marginBottom: 8, fontSize: t.metaSize, color: c.fgMuted },
  drawerRange: { accentColor: c.primary, flexGrow: 1 },
})
