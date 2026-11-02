import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, DialogTrigger, MenuTrigger, Popover, VisuallyHidden } from 'react-aria-components'
import { Icon } from '../composition/Icons'
import { deferredLoader, DeferredSurfaceBoundary } from '../composition/DeferredSurfaceBoundary.tsx'
import { ThemePortal } from '../taste/ThemePortal'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import type { SidebarActions, SidebarAgentRow as Row } from './model'
import type { SidebarClock } from './SidebarTime'
import { useTouchLayout } from './useTouchLayout'

// Only overlay contents cross this boundary; their real triggers never suspend or remount.
const loadAgentRowMenu = deferredLoader(() => import('./RowOverlays.tsx').then(module => ({ default: module.AgentRowMenu })))
const loadAgentReportedDetails = deferredLoader(() => import('./RowOverlays.tsx').then(module => ({ default: module.AgentReportedDetails })))
const loadHoverCard = deferredLoader(() => import('./RowOverlays.tsx').then(module => ({ default: module.AgentHoverCard })))
export function AgentHoverCard(props: { readonly row: Row; readonly reportedStatusOnly?: boolean } & SidebarClock) {
  return <DeferredSurfaceBoundary load={loadHoverCard} surfaceProps={props} fallback={<span role="alert">This agent hover card could not load.</span>} />
}

export interface AgentRowActionsProps extends SidebarClock {
  readonly row: Row
  readonly reportedStatusOnly?: boolean
  readonly actions?: SidebarActions
  readonly onOpen?: (row: Row) => void
  readonly onMarkRead?: (ref: string) => void
  readonly anchorRef: React.RefObject<HTMLElement | null>
  readonly menuTriggerRef: React.RefObject<HTMLButtonElement | null>
  readonly menuButtonStyle: (state: { readonly isHovered: boolean; readonly isPressed: boolean; readonly isFocusVisible: boolean }) => stylex.StyleXStyles
  readonly wrapDetailsTrigger?: (trigger: React.ReactElement) => React.ReactNode
  readonly onDetailsPress?: () => void
  readonly onMenuOpenChange?: (isOpen: boolean) => void
}

/** The eager trigger shell keeps collection focus, geometry and focus restoration stable during import. */
export const AgentRowActions = (props: AgentRowActionsProps) => {
  const touch = useTouchLayout()
  const [menuOpen, setMenuOpen] = React.useState(false)
  const [detailsOpen, setDetailsOpen] = React.useState(false)
  const trigger = <VisuallyHidden isFocusable><Button aria-label={`Reported details for ${props.row.title}`} onPress={props.onDetailsPress} {...stylex.props(styles.iconButton, touch && styles.detailsTouchTarget)}><Icon name="message" /></Button></VisuallyHidden>
  return <ThemePortal>
    <DialogTrigger isOpen={detailsOpen} onOpenChange={open => {
      setDetailsOpen(open)
      if (!open) requestAnimationFrame(() => {
        const button = props.menuTriggerRef.current
        const active = button?.ownerDocument.activeElement
        if (active === button?.ownerDocument.body || active === props.anchorRef.current || active === props.anchorRef.current?.closest('[role="row"]')) button?.focus()
      })
    }}>{props.wrapDetailsTrigger === undefined ? trigger : props.wrapDetailsTrigger(trigger)}
      {detailsOpen && <DeferredSurfaceBoundary load={loadAgentReportedDetails} surfaceProps={props} fallback={<Popover triggerRef={props.menuTriggerRef} placement="bottom end" {...stylex.props(styles.failureSurface)}><Dialog aria-label={`Reported details for ${props.row.title}`}><p role="alert">These agent details could not load.</p><Button onPress={() => setDetailsOpen(false)}>Close</Button></Dialog></Popover>} />}
    </DialogTrigger>
    <MenuTrigger isOpen={menuOpen} onOpenChange={open => { setMenuOpen(open); props.onMenuOpenChange?.(open) }}>
      <Button ref={props.menuTriggerRef} data-agent-menu="true" aria-label={`Actions for ${props.row.title}`} className={state => stylex.props(props.menuButtonStyle(state), touch && styles.menuNarrow).className ?? ''}><Icon name="chevron-right" /></Button>
      {menuOpen && <DeferredSurfaceBoundary load={loadAgentRowMenu} surfaceProps={{ ...props, onDetailsPress: () => { props.onDetailsPress?.(); setDetailsOpen(true) } }} fallback={<Popover placement="bottom end" {...stylex.props(styles.failureSurface)}><Dialog aria-label={`Actions for ${props.row.title}`}><p role="alert">These agent actions could not load.</p><Button onPress={() => { setMenuOpen(false); props.onMenuOpenChange?.(false) }}>Close</Button></Dialog></Popover>} />}
    </MenuTrigger>
  </ThemePortal>
}

const styles = stylex.create({
  failureSurface: { backgroundColor: surface.raised, color: ink.fg, padding: s.lg, borderRadius: r.md, maxWidth: 'calc(100vw - 32px)' },
  menuNarrow: { gridColumn: '-2 / -1', gridRow: '1 / -1', width: g.touchTarget, height: g.touchTarget, opacity: 1 },
  detailsTouchTarget: { width: g.touchTarget, height: g.touchTarget },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, minHeight: g.controlSm, borderWidth: 0, padding: s.zero, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowActive }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
})
