/**
 * The one hover-card primitive. Every entity card (agent, mission, resource, folder, a plugin's GitHub item) opens
 * through it, so timing, placement, dismissal, touch and the card frame are decided once:
 * - Intent: open 150ms after the pointer rests on the trigger or keyboard focus lands on it (focus-visible only), close
 *   300ms after both leave. Opening one card closes any other at once, so a pass across rows never stacks cards.
 * - Touch (a coarse primary pointer or a narrow host) never opens a card; touch hosts disclose details through a menu.
 * - Dismissal: Escape, a press on the trigger, scrolling the page or another card opening.
 * - Passive: the card never takes pointer events or focus, so it never steals a hover or a click from what lies beneath.
 *   Everything it shows is also reachable without hover (the trigger's description or a details dialog).
 * - The frame (surface, border, radius, shadow, padding, width) lives on `HoverCardSurface`; hosts never restyle it.
 * Hosts whose trigger is a whole region (a sidebar row, a tab) own the intent with `useHoverCardIntent` and render
 * `HoverCardSurface` against their anchor; anything else wraps its trigger in `HoverCard`.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useFocusVisible } from 'react-aria'
import { Tooltip, TooltipTriggerStateContext, type TooltipProps, type TooltipTriggerState } from 'react-aria-components'
import { ThemePortal } from '../taste/ThemePortal'
import { useTouchLayout } from '../sidebar/useTouchLayout'
import { borderVars as border, elevationVars as elevation, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { hoverCardTiming } from './entity.ts'

export type HoverCardPlacement = NonNullable<TooltipProps['placement']>

const hoverCardOpenEvent = 'fractal:hover-card-open'

export interface HoverCardIntent {
  /** False on touch layouts: the host shows no hover affordance at all. */
  readonly enabled: boolean
  readonly isOpen: boolean
  /** Open or close now, without intent timing (a press, a menu opening, Escape). */
  readonly setOpen: (open: boolean) => void
  /** Pointer or focus arrived (`true`) or left (`false`): opens or closes after the canonical delay. */
  readonly intend: (open: boolean) => void
}

/** Canonical hover intent: 150ms to open, 300ms to close, nothing on touch, one open card per document. */
export function useHoverCardIntent(): HoverCardIntent {
  const touch = useTouchLayout()
  const id = React.useId()
  // Open state remembers the layout it was set under; a layout switch closes it during render. A reset effect would
  // also run after mount, and a hover that lands before mount effects flush would be closed by it.
  const [hover, setHover] = React.useState({ open: false, touch })
  if (hover.touch !== touch) setHover({ open: false, touch })
  const timer = React.useRef<number | undefined>(undefined)
  React.useEffect(() => () => { window.clearTimeout(timer.current) }, [touch])
  const isOpen = !touch && hover.touch === touch && hover.open
  React.useEffect(() => {
    if (!isOpen) return
    window.dispatchEvent(new CustomEvent(hoverCardOpenEvent, { detail: id }))
    const onOther = (event: Event) => { if ((event as CustomEvent<string>).detail !== id) { window.clearTimeout(timer.current); setHover({ open: false, touch }) } }
    window.addEventListener(hoverCardOpenEvent, onOther)
    return () => window.removeEventListener(hoverCardOpenEvent, onOther)
  }, [isOpen, id, touch])
  const setOpen = React.useCallback((open: boolean) => {
    window.clearTimeout(timer.current)
    setHover({ open: !touch && open, touch })
  }, [touch])
  const intend = React.useCallback((open: boolean) => {
    window.clearTimeout(timer.current)
    if (touch) return
    timer.current = window.setTimeout(() => setHover({ open, touch }), open ? hoverCardTiming.openMs : hoverCardTiming.closeMs)
  }, [touch])
  return { enabled: !touch, isOpen, setOpen, intend }
}

export interface HoverCardSurfaceProps {
  readonly triggerRef: React.RefObject<Element | null>
  readonly isOpen: boolean
  readonly onOpenChange?: (open: boolean) => void
  /** Where the card sits beside its trigger: `right top` beside a sidebar row, `bottom start` under a tab or a link. */
  readonly placement?: HoverCardPlacement
  readonly offset?: number
  /** Names the card for assistive tech, e.g. "Reported details for Polish the sidebar". */
  readonly label?: string
  /** The surface's id, for a trigger that names it in `aria-describedby`. */
  readonly id?: string
  readonly children: React.ReactNode
}

/**
 * The canonical card frame, positioned against a trigger the host owns. Mounts nothing while closed. Render it inside
 * the host's `ThemePortal` (mounted before the card opens) so the card paints the host's theme. It is a passive
 * `tooltip` named by `label`: assistive tech reads it as the trigger's supplementary description, never a focus stop.
 */
export function HoverCardSurface({ triggerRef, isOpen, onOpenChange, placement = 'right top', offset = 8, label, id, children }: HoverCardSurfaceProps) {
  const close = React.useCallback(() => onOpenChange?.(false), [onOpenChange])
  const state = React.useMemo<TooltipTriggerState>(() => ({ isOpen, shouldSkipAnimation: false, open: () => onOpenChange?.(true), close }), [isOpen, onOpenChange, close])
  React.useEffect(() => {
    if (!isOpen) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      // Escape dismisses this passive layer first, without dismissing an enclosing modal or moving focus.
      close()
    }
    document.addEventListener('keydown', onKeyDown, true)
    return () => document.removeEventListener('keydown', onKeyDown, true)
  }, [isOpen, close])
  if (!isOpen) return null
  return <TooltipTriggerStateContext.Provider value={state}>
      <Tooltip isOpen triggerRef={triggerRef as React.RefObject<Element>} placement={placement} offset={offset} containerPadding={8} shouldFlip
        ref={element => { if (element !== null) { if (id === undefined) element.removeAttribute('id'); else element.id = id } }}
        aria-label={label} data-hover-card {...stylex.props(styles.frame)}>
        {children}
      </Tooltip>
    </TooltipTriggerStateContext.Provider>
}

export interface HoverCardProps {
  /** One focusable element (a link, a button, a tab). It keeps its own role, name and press behavior. */
  readonly children: React.ReactElement
  /**
   * The card body, evaluated only while open, so a body that reads data starts its read when the card opens.
   * Returning `null` keeps the card shut (a plugin with nothing to show vetoes the card).
   */
  readonly content: React.ReactNode | (() => React.ReactNode)
  readonly placement?: HoverCardPlacement
  readonly label?: string
  /** Controlled open state; omit for the canonical intent. `onOpenChange` reports intent either way. */
  readonly isOpen?: boolean
  readonly onOpenChange?: (open: boolean) => void
  readonly isDisabled?: boolean
}

/** Wraps one trigger with the canonical card: pointer rest or keyboard focus opens it, leaving closes it. */
export function HoverCard({ children, content, placement = 'bottom start', label, isOpen: controlled, onOpenChange, isDisabled = false }: HoverCardProps) {
  const intent = useHoverCardIntent()
  const surfaceId = React.useId()
  const { isFocusVisible } = useFocusVisible()
  const triggerRef = React.useRef<Element | null>(null)
  const enabled = intent.enabled && !isDisabled
  const intentOpen = enabled && intent.isOpen
  const lastReported = React.useRef(intentOpen)
  React.useEffect(() => { if (lastReported.current !== intentOpen) { lastReported.current = intentOpen; onOpenChange?.(intentOpen) } }, [intentOpen, onOpenChange])
  const open = enabled && (controlled ?? intentOpen)
  const body = open ? typeof content === 'function' ? content() : content : null
  // The trigger keeps its own element, role and handlers; a box-less wrapper hears pointer, focus and keys around it.
  const scope = React.useRef<HTMLSpanElement | null>(null)
  const setScope = React.useCallback((element: HTMLSpanElement | null) => {
    scope.current = element
    triggerRef.current = element?.firstElementChild ?? null
  }, [])
  const handlers = enabled ? {
    onPointerEnter: (event: React.PointerEvent) => { if (event.pointerType !== 'touch') intent.intend(true) },
    onPointerLeave: () => intent.intend(false),
    onFocus: () => { if (isFocusVisible) intent.intend(true) },
    onBlur: (event: React.FocusEvent) => { if (!event.currentTarget.contains(event.relatedTarget)) intent.setOpen(false) },
    onPointerDown: () => intent.setOpen(false),
    onKeyDown: (event: React.KeyboardEvent) => { if (event.key === 'Escape' && open) { event.stopPropagation(); intent.setOpen(false) } },
  } : {}
  React.useEffect(() => {
    if (!open) return
    triggerRef.current = scope.current?.firstElementChild ?? null
    const onScroll = () => intent.setOpen(false)
    window.addEventListener('scroll', onScroll, true)
    return () => window.removeEventListener('scroll', onScroll, true)
  }, [open, intent.setOpen])
  // While open, the card describes its trigger, so assistive tech reads it without a focus stop inside the card.
  const described = body !== null && body !== undefined
  React.useLayoutEffect(() => {
    const trigger = scope.current?.firstElementChild
    if (!described || trigger === null || trigger === undefined) return
    const previous = trigger.getAttribute('aria-describedby')
    trigger.setAttribute('aria-describedby', previous === null ? surfaceId : `${previous} ${surfaceId}`)
    return () => { if (previous === null) trigger.removeAttribute('aria-describedby'); else trigger.setAttribute('aria-describedby', previous) }
  }, [described, surfaceId])
  // The portal layer mounts with the trigger, so it exists before the card first opens.
  return <ThemePortal>
    <span ref={setScope} data-hover-card-trigger {...handlers} {...stylex.props(styles.scope)}>{children}</span>
    {described && <HoverCardSurface id={surfaceId} triggerRef={triggerRef} isOpen onOpenChange={intent.setOpen} placement={placement} label={label}>{body}</HoverCardSurface>}
  </ThemePortal>
}

const enter = stylex.keyframes({ from: { opacity: 0 }, to: { opacity: 1 } })
const styles = stylex.create({
  scope: { display: 'contents' },
  // The sidebar row's card frame, unchanged: raised surface, strong hairline, md radius, popover elevation, 16px gutters.
  frame: {
    backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md,
    padding: s.lg, boxSizing: 'border-box', width: 'max-content', maxWidth: 'calc(100vw - 32px)', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading,
    boxShadow: elevation.popover, outline: 'none', pointerEvents: 'none', zIndex: 50,
    // RAC supplies placement only after measuring the mounted anchor; never paint at the origin first.
    visibility: { default: 'hidden', ':is([data-placement])': 'visible' },
    animationName: enter, animationDuration: m.fast, animationTimingFunction: 'ease-out', animationFillMode: 'backwards',
    '@media (prefers-reduced-motion: reduce)': { animationName: 'none' },
  },
})
