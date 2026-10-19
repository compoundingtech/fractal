// Chrome controls: thread header, sidebar header, toolbar and footer. Explicit-fact presentation only.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Focusable, Pressable } from 'react-aria-components'
import { interactionVars as interaction, geometryVars as g, motionVars as m, radiusVars as r, surfaceVars as surface, textVars as ink } from '../composition-tokens.stylex'
import { Icon, type IconName } from './Icons'
import { Tooltip } from './Controls'
import { NarrowLayoutContext } from './NarrowLayout'

export interface ChromeControlProps {
  readonly icon: IconName
  /** Accessible name and tooltip text: "Toggle right panel", "Actions for fractal-web". */
  readonly label: string
  /** Toggle state; omitted for plain actions. A pressed toggle keeps the hover fill. */
  readonly pressed?: boolean
  /** The fill without toggle semantics: a menu trigger whose filter or sort is in effect. The label says what is set. */
  readonly active?: boolean
  /**
   * Why the control cannot run. It stays in the tab order as an `aria-disabled` button that never presses,
   * is described by the reason, and shows "label · reason" in the kit tooltip on hover and keyboard focus.
   */
  readonly disabledReason?: string
  /** Briefly unavailable, e.g. while its own action is in flight. */
  readonly isDisabled?: boolean
  readonly onPress?: () => void
  /** The control opens a popup (a sheet on narrow hosts): its kind, open state and the opened element's id. */
  readonly popup?: 'dialog' | 'menu'
  readonly expanded?: boolean
  readonly controls?: string
  /** Tooltip text when it should say more than the accessible name, e.g. a keyboard shortcut. Defaults to `label`. */
  readonly tooltip?: string
  /** The enabled button, for hosts that move focus to it. */
  readonly ref?: React.Ref<HTMLButtonElement>
  /** A header-edge control: in a narrow host its 44px touch target overhangs the gutter so the glyph stays where the 28px control put it. */
  readonly edge?: 'lead' | 'trail'
  readonly tooltipPlacement?: 'top' | 'bottom'
}

/**
 * One quiet 28px icon button for the chrome: borderless, filled on hover, named by the kit tooltip.
 * Inside a React Aria trigger (MenuTrigger, DialogTrigger) it is the trigger's button.
 * Tooltips follow hover or user keyboard navigation, never autofocus or programmatic focus restoration.
 */
export function ChromeControl({ icon, label, pressed, active = false, disabledReason, isDisabled = false, onPress, popup, expanded, controls, tooltip, ref, edge, tooltipPlacement = 'bottom' }: ChromeControlProps) {
  const reasonId = React.useId()
  // Inside a narrow (phone) host every chrome control is a 44px touch target; a narrow desktop pane keeps desktop density.
  const narrow = React.useContext(NarrowLayoutContext)
  const touch = narrow ? [styles.touch, edge === 'lead' && styles.touchLead, edge === 'trail' && styles.touchTrail] : []
  if (disabledReason !== undefined) {
    // A natively disabled button leaves the tab order and receives no pointer events, so its reason would be
    // unreachable. The disabled Pressable consumes a surrounding trigger's press (DialogTrigger, MenuTrigger) and
    // fires nothing; Focusable hands hover and focus to the kit tooltip. A MenuTrigger also puts its own key
    // handler on the button, which opens on Enter, Space and the arrows unless the event is default-prevented,
    // so the capture phase prevents those keys before any trigger sees them. Tab and Escape still pass.
    return <>
      <Tooltip label={`${label} · ${disabledReason}`} placement={tooltipPlacement} navigationFocusOnly>
        <Focusable>
          <Pressable isDisabled>
            <button
              onKeyDownCapture={event => { if (triggerKeys[event.key] === true) event.preventDefault() }}
              type="button"
              aria-label={label}
              aria-disabled="true"
              aria-describedby={reasonId}
              {...(pressed === undefined ? {} : { 'aria-pressed': pressed })}
              {...stylex.props(styles.control, styles.disabled, ...touch)}
            >
              <Icon name={icon} />
            </button>
          </Pressable>
        </Focusable>
      </Tooltip>
      <span id={reasonId} hidden>{disabledReason}</span>
    </>
  }
  return <Tooltip label={tooltip ?? label} placement={tooltipPlacement} navigationFocusOnly>
    <Button
      {...(ref === undefined ? {} : { ref })}
      aria-label={label}
      {...(pressed === undefined ? {} : { 'aria-pressed': pressed })}
      {...(popup === undefined ? {} : { 'aria-haspopup': popup })}
      {...(expanded === undefined ? {} : { 'aria-expanded': expanded })}
      {...(controls === undefined ? {} : { 'aria-controls': controls })}
      isDisabled={isDisabled}
      {...(onPress === undefined ? {} : { onPress })}
      {...stylex.props(styles.control, (pressed === true || active) && styles.pressed, ...touch)}
    >
      <Icon name={icon} />
    </Button>
  </Tooltip>
}

/** Keys a React Aria trigger acts on; a control with a reason swallows them. */
const triggerKeys: Record<string, true> = { Enter: true, ' ': true, ArrowDown: true, ArrowUp: true }

const styles = stylex.create({
  touch: { width: g.touchTarget, height: g.touchTarget },
  touchLead: { marginInlineStart: `calc((${g.controlMd} - ${g.touchTarget}) / 2)` },
  touchTrail: { marginInlineEnd: `calc((${g.controlMd} - ${g.touchTarget}) / 2)` },
  control: {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, boxSizing: 'border-box',
    width: g.controlMd, height: g.controlMd, padding: 0,
    borderWidth: 0, borderRadius: r.control, backgroundColor: 'transparent', color: ink.fgMuted,
    cursor: 'pointer',
    transitionProperty: 'background-color, color', transitionDuration: m.fast, transitionTimingFunction: m.ease,
    ':hover': { backgroundColor: surface.rowHover, color: ink.fg },
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
    ':disabled': { opacity: 0.64, cursor: 'default', backgroundColor: 'transparent', color: ink.fgMuted },
  },
  pressed: { backgroundColor: surface.rowActive, color: ink.fg, ':hover': { backgroundColor: surface.rowActive, color: ink.fg } },
  // The aria-disabled twin of `:disabled`: dimmed and inert to hover, but it keeps the focus ring.
  disabled: { opacity: 0.64, cursor: 'default', ':hover': { backgroundColor: 'transparent', color: ink.fgMuted } },
})
