import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, VisuallyHidden } from 'react-aria-components'
import { Icon } from '../composition/Icons'
import { accentVars as accent, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import type { AgentRowActionsProps } from './SidebarAgentRow.tsx'
import { useTouchLayout } from './useTouchLayout'

const AgentRowActions = React.lazy(() => import('./SidebarAgentRow.tsx').then(module => ({ default: module.AgentRowActions })))
export const AgentHoverCard = React.lazy(() => import('./SidebarAgentRow.tsx').then(module => ({ default: module.AgentHoverCard })))

/** Only the action affordance suspends: the tree item, badges and navigation never remount. */
export const DeferredRowActions = (props: AgentRowActionsProps) => {
  const touch = useTouchLayout()
  const [requested, request] = React.useState<'menu' | 'details'>()
  const [pending, startTransition] = React.useTransition()
  const trigger = <>
    <VisuallyHidden isFocusable><Button aria-label={`Reported details for ${props.row.title}`} aria-busy={pending} onPress={() => { props.onDetailsPress?.(); startTransition(() => request('details')) }} {...stylex.props(styles.iconButton, touch && styles.detailsTouchTarget)}><Icon name="message" /></Button></VisuallyHidden>
    <Button ref={props.menuTriggerRef} data-agent-menu="true" aria-label={`Actions for ${props.row.title}`} aria-haspopup="menu" aria-busy={pending} onPress={() => startTransition(() => request('menu'))} className={state => stylex.props(props.menuButtonStyle(state), touch && styles.menuNarrow).className ?? ''}><Icon name="chevron-right" /></Button>
  </>
  return <React.Suspense fallback={trigger}>{requested === undefined ? trigger :
    <AgentRowActions {...props} defaultMenuOpen={requested === 'menu'} defaultDetailsOpen={requested === 'details'} />
  }</React.Suspense>
}

const styles = stylex.create({
  menuNarrow: { gridColumn: '-2 / -1', gridRow: '1 / -1', width: g.touchTarget, height: g.touchTarget, opacity: 1 },
  detailsTouchTarget: { width: g.touchTarget, height: g.touchTarget },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, minHeight: g.controlSm, borderWidth: 0, padding: s.zero, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowActive }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
})
