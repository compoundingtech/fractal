import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, DialogTrigger, MenuTrigger, VisuallyHidden } from 'react-aria-components'
import { Icon } from '../composition/Icons'
import { ThemePortal } from '../taste/ThemePortal'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import type { SidebarActions, SidebarAgentRow as Row } from './model'
import type { SidebarClock } from './SidebarTime'
import { useTouchLayout } from './useTouchLayout'

// Only overlay contents cross this boundary; their real triggers never suspend or remount.
const AgentRowMenu = React.lazy(() => import('./RowOverlays.tsx').then(module => ({ default: module.AgentRowMenu })))
const AgentReportedDetails = React.lazy(() => import('./RowOverlays.tsx').then(module => ({ default: module.AgentReportedDetails })))
const HoverCardContents = React.lazy(() => import('./RowOverlays.tsx').then(module => ({ default: module.AgentHoverCard })))
export function AgentHoverCard(props: { readonly row: Row; readonly reportedStatusOnly?: boolean } & SidebarClock) {
  return <React.Suspense fallback={null}><HoverCardContents {...props} /></React.Suspense>
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
      {detailsOpen && <React.Suspense fallback={null}><AgentReportedDetails {...props} /></React.Suspense>}
    </DialogTrigger>
    <MenuTrigger isOpen={menuOpen} onOpenChange={open => { setMenuOpen(open); props.onMenuOpenChange?.(open) }}>
      <Button ref={props.menuTriggerRef} data-agent-menu="true" aria-label={`Actions for ${props.row.title}`} className={state => stylex.props(props.menuButtonStyle(state), touch && styles.menuNarrow).className ?? ''}><Icon name="chevron-right" /></Button>
      {menuOpen && <React.Suspense fallback={null}><AgentRowMenu {...props} onDetailsPress={() => { props.onDetailsPress?.(); setDetailsOpen(true) }} /></React.Suspense>}
    </MenuTrigger>
  </ThemePortal>
}

const styles = stylex.create({
  menuNarrow: { gridColumn: '-2 / -1', gridRow: '1 / -1', width: g.touchTarget, height: g.touchTarget, opacity: 1 },
  detailsTouchTarget: { width: g.touchTarget, height: g.touchTarget },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, minHeight: g.controlSm, borderWidth: 0, padding: s.zero, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowActive }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
})
