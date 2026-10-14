import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button as AriaButton, Tooltip as AriaTooltip, TooltipTrigger, type ButtonProps } from 'react-aria-components'
import { surfaceVars as surface, textVars as text, borderVars as border, accentVars as accent, geometryVars as g, geometryNumbers, radiusVars as r, spaceVars as s, typeVars as t, elevationVars as elevation } from '../composition-tokens.stylex'
import { Icon, type IconName } from './Icons'
import { ThemePortal } from '../taste/ThemePortal'

export function Button({ children, variant = 'ghost', size = 'sm', ...props }: ButtonProps & { variant?: 'ghost' | 'outline'; size?: 'sm' | 'md' | 'lg' }) {
  return <AriaButton {...props} {...stylex.props(styles.button, styles[size], variant === 'outline' && styles.outline)}>{children}</AriaButton>
}
export function IconButton({ icon, label, ...props }: Omit<ButtonProps, 'children'> & { icon: IconName; label: string }) {
  return <Button {...props} aria-label={label}><Icon name={icon} /></Button>
}
/** Quiet time after the last scroll before a keyboard-focused card reopens at its new position. */
const scrollSettleMs = 100
/** Announces an opened kit card so every other open kit card closes, including one reopened outside React Aria's registry. */
const tooltipOpenEvent = 'fractal-kit-tooltip-open'
/**
 * The themed host owns each React Aria portal; menus never fall back to the global palette.
 * React Aria closes an open card on any ancestor scroll, including the scroll that brings a newly focused trigger into view
 * (a transcript row reached with Tab). While the trigger keeps keyboard focus, the card reopens once that scroll settles;
 * Escape during the settle cancels it, and only one kit card is open at a time.
 */
export function Tooltip({ label, placement = 'bottom', delay = 350, children }: { label: string; placement?: 'top' | 'bottom'; delay?: number; children: React.ReactNode }) {
  const id = React.useId()
  const scope = React.useRef<HTMLSpanElement>(null)
  const lastScroll = React.useRef(Number.NEGATIVE_INFINITY)
  const reopenRequest = React.useRef(0)
  const [isOpen, setOpen] = React.useState(false)
  const [settling, setSettling] = React.useState(false)
  const keyboardFocused = () => {
    const active = scope.current?.ownerDocument.activeElement
    return active != null && scope.current!.contains(active) && active.matches(':focus-visible')
  }
  React.useEffect(() => {
    if (!isOpen && !settling) return
    let timer: number | undefined
    const settle = () => {
      window.clearTimeout(timer)
      timer = window.setTimeout(() => { setSettling(false); if (keyboardFocused()) setOpen(true) }, scrollSettleMs)
    }
    const onScroll = () => {
      lastScroll.current = performance.now()
      if (settling) settle()
    }
    window.addEventListener('scroll', onScroll, true)
    if (settling) settle()
    return () => { window.clearTimeout(timer); window.removeEventListener('scroll', onScroll, true) }
  }, [isOpen, settling])
  React.useEffect(() => {
    if (!isOpen && !settling) return
    const onOtherOpen = (event: Event) => { if ((event as CustomEvent<string>).detail !== id) { reopenRequest.current++; lastScroll.current = Number.NEGATIVE_INFINITY; setSettling(false); setOpen(false) } }
    window.addEventListener(tooltipOpenEvent, onOtherOpen)
    if (isOpen) window.dispatchEvent(new CustomEvent(tooltipOpenEvent, { detail: id }))
    return () => window.removeEventListener(tooltipOpenEvent, onOtherOpen)
  }, [isOpen, settling, id])
  const onOpenChange = (open: boolean) => {
    setOpen(open)
    if (open) return
    const closedAt = performance.now()
    // Decide after the scroll dispatch finishes, whichever capture listener ran first.
    const request = ++reopenRequest.current
    window.setTimeout(() => { if (request === reopenRequest.current && Math.abs(closedAt - lastScroll.current) < scrollSettleMs && keyboardFocused()) setSettling(true) })
  }
  return <ThemePortal><span ref={scope} onKeyDownCapture={event => { if (event.key === 'Escape') { reopenRequest.current++; lastScroll.current = Number.NEGATIVE_INFINITY; setSettling(false); setOpen(false) } }} {...stylex.props(styles.tooltipScope)}><TooltipTrigger delay={delay} closeDelay={0} isOpen={isOpen} onOpenChange={onOpenChange}>{children}<AriaTooltip placement={placement} offset={geometryNumbers.tooltipOffset} {...stylex.props(tooltipStyles.tooltip)}>{label}</AriaTooltip></TooltipTrigger></span></ThemePortal>
}
const styles = stylex.create({
  button: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: s.xs, paddingInline: s.sm, borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', flexShrink: 0, ':hover': { backgroundColor: surface.rowHover, color: text.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.hairline }, ':disabled': { opacity: 0.64, cursor: 'default' } },
  sm: { minWidth: g.controlSm, height: g.controlSm }, md: { minWidth: g.controlMd, height: g.controlMd }, lg: { minWidth: g.controlLg, height: g.controlLg },
  outline: { borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: text.fg },
  tooltipScope: { display: 'contents' },
})
/** Shared tooltip surface; portal ownership and trigger behavior stay with each caller. */
export const tooltipStyles = stylex.create({
  tooltip: { zIndex: 50, borderRadius: r.control, backgroundColor: text.fg, paddingInline: s.row, paddingBlock: s.sm, fontSize: t.denseSize, fontFamily: t.fontSans, color: surface.raised, boxShadow: elevation.popover, width: 'max-content', maxWidth: g.tooltipMax, whiteSpace: 'normal', overflowWrap: 'break-word', textAlign: 'start' },
})
